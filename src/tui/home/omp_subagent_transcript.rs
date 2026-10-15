//! Discovery and bounded parsing of native OMP sub-agent transcripts.

use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::subagent_windows::{NativeSubagentMessage, NativeSubagentRecord};

const MAX_TRANSCRIPT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MESSAGES: usize = 100;

pub(super) fn scan_omp_subagents(parent: &Path) -> Vec<NativeSubagentRecord> {
    let Some(session_id) = parent.file_stem().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let completed = completed_labels(parent);
    let directory = parent.with_file_name(session_id);
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| parse_omp_subagent(&path, parent, &completed))
        .collect()
}

fn completed_labels(parent: &Path) -> HashSet<String> {
    let mut completed = HashSet::new();
    for value in bounded_json_lines(parent) {
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("toolResult") {
            continue;
        }
        let Some(text) = content_text(message.get("content")) else {
            continue;
        };
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("### ") else {
                continue;
            };
            let Some((label, status)) = rest.split_once(" [") else {
                continue;
            };
            if status.contains("completed") {
                completed.insert(label.trim().to_string());
            }
        }
    }
    completed
}

fn parse_omp_subagent(
    path: &Path,
    parent: &Path,
    completed: &HashSet<String>,
) -> Option<NativeSubagentRecord> {
    let description = path.file_stem()?.to_str()?.to_string();
    let mut id = None;
    let mut prompt = String::new();
    let mut messages = VecDeque::new();
    let mut running = !completed.contains(&description);
    for value in bounded_json_lines(path) {
        match value.get("type").and_then(Value::as_str) {
            Some("session") => {
                let linked = value
                    .get("parentSession")
                    .and_then(Value::as_str)
                    .is_some_and(|linked| Path::new(linked) == parent);
                if !linked {
                    return None;
                }
                id = value.get("id").and_then(Value::as_str).map(str::to_owned);
            }
            Some("custom")
                if value.get("customType").and_then(Value::as_str) == Some("session_exit") =>
            {
                running = false;
            }
            Some("message") => {
                let Some(message) = value.get("message") else {
                    continue;
                };
                match message.get("role").and_then(Value::as_str) {
                    Some("user") if prompt.is_empty() => {
                        prompt = content_text(message.get("content")).unwrap_or_default();
                    }
                    Some("assistant") => {
                        if let Some(text) = content_text(message.get("content")) {
                            if messages.len() == MAX_MESSAGES {
                                messages.pop_front();
                            }
                            messages.push_back(NativeSubagentMessage { text });
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Some(NativeSubagentRecord {
        id: id?,
        description,
        prompt,
        running,
        messages: messages.into(),
    })
}

fn bounded_json_lines(path: &Path) -> impl Iterator<Item = Value> {
    let mut lines = Vec::new();
    let Ok(file) = std::fs::File::open(path) else {
        return lines.into_iter();
    };
    let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let mut reader = BufReader::new(file);
    if len > MAX_TRANSCRIPT_BYTES {
        if reader
            .seek(SeekFrom::Start(len - MAX_TRANSCRIPT_BYTES))
            .is_err()
        {
            return lines.into_iter();
        }
        let mut partial = String::new();
        let _ = reader.read_line(&mut partial);
    }
    lines.extend(
        reader
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok()),
    );
    lines.into_iter()
}

fn content_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(text) => nonempty(text),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .filter_map(nonempty)
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn nonempty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_omp_child_linked_to_parent() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent.jsonl");
        let children = temp.path().join("parent");
        std::fs::create_dir(&children).unwrap();
        std::fs::write(
            &parent,
            "{\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"content\":[{\"type\":\"text\",\"text\":\"### Scout [task] - completed\"}]}}\n",
        )
        .unwrap();
        std::fs::write(
            children.join("Scout.jsonl"),
            format!(
                "{{\"type\":\"session\",\"id\":\"child-1\",\"parentSession\":{}}}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"inspect\"}}]}}}}\n{{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"found it\"}}]}}}}\n",
                serde_json::to_string(parent.to_str().unwrap()).unwrap()
            ),
        )
        .unwrap();

        let records = scan_omp_subagents(&parent);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "child-1");
        assert_eq!(records[0].description, "Scout");
        assert_eq!(records[0].prompt, "inspect");
        assert_eq!(records[0].messages[0].text, "found it");
        assert!(!records[0].running);
    }
}
