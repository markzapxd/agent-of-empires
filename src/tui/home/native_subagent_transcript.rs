//! Discovery and bounded parsing of native Claude sub-agent transcripts.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::subagent_windows::{NativeSubagentMessage, NativeSubagentRecord};
use crate::session::Instance;

const MAX_TRANSCRIPT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MESSAGES: usize = 100;

pub(super) fn native_claude_transcript(instance: &Instance) -> Option<PathBuf> {
    let binding = instance.agent_session_binding.as_ref()?;
    let execution = binding.execution.as_ref()?;
    if !matches!(
        execution.agent.as_str(),
        "claude" | "claude-code" | "claudex"
    ) || execution.filesystem != "host"
    {
        return None;
    }
    if let Some(path) = binding
        .transcript_path
        .as_ref()
        .filter(|path| path.is_file())
    {
        return Some(path.clone());
    }
    let store = execution.stores.first()?;
    let encoded = encode_claude_project_path(&execution.cwd.to_string_lossy());
    let path = store
        .join("projects")
        .join(encoded)
        .join(format!("{}.jsonl", binding.session_id));
    path.is_file().then_some(path)
}
pub(super) fn encode_claude_project_path(project_path: &str) -> String {
    project_path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn scan_subagents(parent: &Path) -> Vec<NativeSubagentRecord> {
    let Some(session_id) = parent.file_stem().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let completed = completed_agent_ids(parent);
    let directory = parent.with_file_name(session_id).join("subagents");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| parse_subagent(&path, &completed))
        .collect()
}

fn completed_agent_ids(parent: &Path) -> std::collections::HashSet<String> {
    let Ok(file) = std::fs::File::open(parent) else {
        return Default::default();
    };
    let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let mut reader = BufReader::new(file);
    if len > MAX_TRANSCRIPT_BYTES {
        if reader
            .seek(SeekFrom::Start(len - MAX_TRANSCRIPT_BYTES))
            .is_err()
        {
            return Default::default();
        }
        let mut partial = String::new();
        let _ = reader.read_line(&mut partial);
    }
    let mut completed = std::collections::HashSet::new();
    for line in reader.lines().map_while(Result::ok) {
        if !line.contains("<status>completed</status>") {
            continue;
        }
        let Some(start) = line.find("<task-id>").map(|index| index + 9) else {
            continue;
        };
        let Some(end) = line[start..].find("</task-id>").map(|index| start + index) else {
            continue;
        };
        completed.insert(line[start..end].to_string());
    }
    completed
}

fn parse_subagent(
    path: &Path,
    completed: &std::collections::HashSet<String>,
) -> Option<NativeSubagentRecord> {
    let id = path
        .file_stem()?
        .to_str()?
        .strip_prefix("agent-")?
        .to_string();
    let description = read_json(&path.with_extension("meta.json"))
        .and_then(|meta| text_field(&meta, "description"))
        .unwrap_or_default();
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut reader = BufReader::new(file);
    if len > MAX_TRANSCRIPT_BYTES {
        reader
            .seek(SeekFrom::Start(len - MAX_TRANSCRIPT_BYTES))
            .ok()?;
        let mut partial = String::new();
        reader.read_line(&mut partial).ok()?;
    }
    let mut prompt = String::new();
    let mut messages = VecDeque::new();
    let running = !completed.contains(&id);
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if prompt.is_empty()
            && value.get("type").and_then(Value::as_str) == Some("user")
            && value.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            prompt = message_text(&value).unwrap_or_default();
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(text) = content_text(message.get("content")) {
            if messages.len() == MAX_MESSAGES {
                messages.pop_front();
            }
            messages.push_back(NativeSubagentMessage { text });
        }
    }
    Some(NativeSubagentRecord {
        id,
        description,
        prompt,
        running,
        messages: messages.into(),
    })
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() <= 64 * 1024)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn message_text(value: &Value) -> Option<String> {
    content_text(value.get("message")?.get("content"))
}

fn content_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(text) => Some(text.trim().to_owned()),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_claude_subagent_transcript() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-a1.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"isSidechain":true,"type":"user","message":{"role":"user","content":"inspect TUI"}}"#,
                "\n",
                r#"{"isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"first reply"}],"stop_reason":null}}"#,
                "\n",
                r#"{"isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"done"}],"stop_reason":"end_turn"}}"#,
                "\n"
            ),
        )
        .unwrap();
        std::fs::write(
            path.with_extension("meta.json"),
            r#"{"description":"Analyze TUI"}"#,
        )
        .unwrap();
        let completed = std::collections::HashSet::from(["a1".to_string()]);

        let record = parse_subagent(&path, &completed).unwrap();
        assert_eq!(record.id, "a1");
        assert_eq!(record.description, "Analyze TUI");
        assert_eq!(record.prompt, "inspect TUI");
        assert!(!record.running);
        assert_eq!(
            record
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["first reply", "done"]
        );
    }
}
