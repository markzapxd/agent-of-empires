//! Discovery and bounded parsing of native Codex sub-agent rollouts.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::subagent_windows::{NativeSubagentMessage, NativeSubagentRecord};
use crate::session::Instance;

const MAX_ROLLOUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MESSAGES: usize = 100;

pub(super) fn scan_codex_subagents(instance: &Instance) -> Option<Vec<NativeSubagentRecord>> {
    let binding = instance.agent_session_binding.as_ref()?;
    let execution = binding.execution.as_ref()?;
    if execution.agent != "codex" || execution.filesystem != "host" {
        return None;
    }
    let sqlite_root = execution
        .stores
        .iter()
        .find(|root| root.join("state_5.sqlite").is_file())?;
    Some(scan_codex_store(sqlite_root, &binding.session_id))
}

fn scan_codex_store(root: &Path, parent_id: &str) -> Vec<NativeSubagentRecord> {
    let database = match Connection::open_with_flags(
        root.join("state_5.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(database) => database,
        Err(_) => return Vec::new(),
    };
    let _ = database.busy_timeout(std::time::Duration::from_millis(100));
    let mut statement = match database.prepare(
        "SELECT child.id, child.rollout_path, child.agent_nickname, child.agent_role, child.title
         FROM thread_spawn_edges edge
         JOIN threads child ON child.id = edge.child_thread_id
         WHERE edge.parent_thread_id = ?1
         ORDER BY child.created_at_ms, child.id",
    ) {
        Ok(statement) => statement,
        Err(_) => return Vec::new(),
    };
    let rows = match statement.query_map([parent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(Result::ok)
        .filter_map(|(id, rollout, nickname, role, title)| {
            parse_codex_rollout(Path::new(&rollout), parent_id, &id, nickname, role, title)
        })
        .collect()
}

fn parse_codex_rollout(
    path: &Path,
    parent_id: &str,
    child_id: &str,
    nickname: Option<String>,
    role: Option<String>,
    title: String,
) -> Option<NativeSubagentRecord> {
    let mut prompt = String::new();
    let mut messages = VecDeque::new();
    let mut running = true;
    let mut linked = false;
    for value in rollout_values(path)? {
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                let payload = value.get("payload")?;
                if payload.get("id").and_then(Value::as_str) == Some(child_id)
                    && payload.get("parent_thread_id").and_then(Value::as_str) == Some(parent_id)
                {
                    linked = true;
                    if prompt.is_empty() {
                        prompt = payload
                            .pointer("/source/subagent/thread_spawn/agent_path")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                }
            }
            Some("response_item") => {
                let payload = value.get("payload")?;
                if payload.get("type").and_then(Value::as_str) == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("assistant")
                {
                    if let Some(text) = codex_content_text(payload.get("content")) {
                        if messages.len() == MAX_MESSAGES {
                            messages.pop_front();
                        }
                        messages.push_back(NativeSubagentMessage { text });
                    }
                }
            }
            Some("event_msg") => {
                let payload = value.get("payload")?;
                if let Some("task_complete" | "task_failed" | "turn_aborted") =
                    payload.get("type").and_then(Value::as_str)
                {
                    running = false;
                }
            }
            _ => {}
        }
    }
    linked.then(|| NativeSubagentRecord {
        id: child_id.to_string(),
        description: nickname
            .or(role)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                if title.trim().is_empty() {
                    "Codex sub-agent".to_string()
                } else {
                    title
                }
            }),
        prompt,
        running,
        messages: messages.into(),
    })
}

fn rollout_values(path: &Path) -> Option<Vec<Value>> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_ROLLOUT_BYTES {
        return None;
    }
    let reader: Box<dyn Read> = if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".zst"))
    {
        Box::new(zstd::stream::read::Decoder::new(file).ok()?)
    } else {
        Box::new(file)
    };
    Some(
        BufReader::new(reader)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect(),
    )
}

fn codex_content_text(content: Option<&Value>) -> Option<String> {
    let text = content?
        .as_array()?
        .iter()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_child_rollout() {
        let temp = tempfile::tempdir().unwrap();
        let rollout = temp.path().join("child.jsonl");
        std::fs::write(
            &rollout,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent","source":{"subagent":{"thread_spawn":{"agent_path":"/root/scout"}}}}}"#,
                "\n",
                r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"working"}]}}"#,
                "\n",
                r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#,
                "\n"
            ),
        )
        .unwrap();

        let record = parse_codex_rollout(
            &rollout,
            "parent",
            "child",
            Some("Scout".into()),
            None,
            String::new(),
        )
        .unwrap();
        assert_eq!(record.description, "Scout");
        assert_eq!(record.prompt, "/root/scout");
        assert_eq!(record.messages[0].text, "working");
        assert!(!record.running);
    }
}
