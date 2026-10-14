use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{session_key, GraphEdge, GraphNode};
use crate::events::{self, Schema};
use crate::git::GitWorktree;
use crate::session::Instance;

const TOPIC: &str = "graph";
pub const MAX_NODES: usize = 2_000;

#[derive(Debug, thiserror::Error)]
#[error("agent graph exceeds the {MAX_NODES}-node snapshot limit")]
pub struct SnapshotLimitExceeded;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub cursor: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeHook {
    SessionStart,
    UserPromptSubmit,
    Stop,
    SessionEnd,
    SubagentStart,
    SubagentStop,
}

fn db_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join("agent-graph.sqlite")
}

fn open(profile_dir: &Path) -> Result<Connection> {
    let schema = Schema::new("agent_graph")?;
    let conn = events::open(&db_path(profile_dir), &schema)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_nodes (
            id TEXT PRIMARY KEY, data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS graph_edges (
            id TEXT PRIMARY KEY, source TEXT NOT NULL, target TEXT NOT NULL,
            data TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS graph_edges_source ON graph_edges(source);
        CREATE INDEX IF NOT EXISTS graph_edges_target ON graph_edges(target);",
    )?;
    Ok(conn)
}

fn upsert_node(conn: &Connection, node: &GraphNode) -> Result<bool> {
    let data = serde_json::to_string(node)?;
    Ok(conn.execute(
        "INSERT INTO graph_nodes(id, data) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET data = excluded.data
         WHERE graph_nodes.data != excluded.data",
        params![node.id, data],
    )? > 0)
}

fn upsert_edge(conn: &Connection, edge: &GraphEdge) -> Result<bool> {
    Ok(conn.execute(
        "INSERT INTO graph_edges(id, source, target, data) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO NOTHING",
        params![
            edge.id,
            edge.source,
            edge.target,
            serde_json::to_string(edge)?
        ],
    )? > 0)
}

fn existing_node(conn: &Connection, id: &str) -> Result<Option<GraphNode>> {
    let json: Option<String> = conn
        .query_row("SELECT data FROM graph_nodes WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()?;
    json.map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

pub fn backfill_project_keys(profile_dir: &Path) -> Result<()> {
    let conn = open(profile_dir)?;
    let mut stmt = conn.prepare("SELECT id, data FROM graph_nodes ORDER BY id")?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let mut changed = false;
    for (_, data) in rows {
        let mut node: GraphNode = serde_json::from_str(&data)?;
        let key = project_key(&node.project_path);
        if node.project_key == key {
            continue;
        }
        node.project_key = key;
        changed |= upsert_node(&conn, &node)?;
    }
    if changed {
        record_change(&conn, "project_keys")?;
    }
    Ok(())
}

fn remove_session_alias(conn: &Connection, alias: &str, canonical: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare("SELECT id, data FROM graph_edges WHERE source = ?1 OR target = ?1 ORDER BY id")?;
    let rows = stmt
        .query_map([alias], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut changed = false;
    for (old_id, data) in rows {
        let mut edge: GraphEdge = serde_json::from_str(&data)?;
        if edge.source == alias {
            edge.source = canonical.to_string();
        }
        if edge.target == alias {
            edge.target = canonical.to_string();
        }
        if edge.source != edge.target {
            edge.id = format!("{}:{}:{}", edge.kind, edge.source, edge.target);
            changed |= upsert_edge(conn, &edge)?;
        }
        changed |= conn.execute("DELETE FROM graph_edges WHERE id = ?1", [old_id])? > 0;
    }
    changed |= conn.execute("DELETE FROM graph_nodes WHERE id = ?1", [alias])? > 0;
    Ok(changed)
}

fn record_change(conn: &Connection, label: &str) -> Result<()> {
    let schema = Schema::new("agent_graph")?;
    let seq = events::highest_seq(conn, &schema, TOPIC)
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("agent graph cursor exhausted"))?;
    let mut stmt = conn.prepare("SELECT data FROM graph_nodes ORDER BY id")?;
    let nodes = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT data FROM graph_edges ORDER BY id")?;
    let edges = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let inserted = events::insert_event(
        conn,
        &schema,
        TOPIC,
        seq,
        &serde_json::json!({ "kind": label, "nodes": nodes, "edges": edges }).to_string(),
        Utc::now().timestamp_millis(),
    )?;
    if inserted != 1 {
        anyhow::bail!("agent graph event cursor collision at {seq}");
    }
    Ok(())
}

fn bounded_task(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(500).collect())
}

fn project_key(project_path: &str) -> String {
    let path = PathBuf::from(project_path);
    let canonical = path.canonicalize().unwrap_or(path);
    let root = GitWorktree::find_main_repo(&canonical).unwrap_or(canonical);
    let root = root.canonicalize().unwrap_or(root);
    format!("workspace:{}", root.to_string_lossy())
}

fn session_node(profile: &str, instance: &Instance, previous: Option<GraphNode>) -> GraphNode {
    let now = Utc::now().to_rfc3339();
    let status = instance.status.as_str().to_owned();
    let prior_status = previous.as_ref().map(|node| node.status.as_str());
    let started_at = if status == "running" && prior_status != Some("running") {
        Some(now.clone())
    } else {
        previous
            .as_ref()
            .and_then(|node| node.started_at.clone())
            .or_else(|| {
                matches!(status.as_str(), "starting" | "waiting" | "idle" | "error")
                    .then(|| instance.created_at.to_rfc3339())
            })
    };
    let task = previous
        .as_ref()
        .and_then(|node| node.task.clone())
        .or_else(|| {
            instance
                .pending_initial_turn
                .as_ref()
                .and_then(|turn| (!turn.synthesized).then_some(turn))
                .and_then(|turn| bounded_task(&turn.text))
        });
    let project_key = project_key(&instance.project_path);
    let stopped_or_error = matches!(status.as_str(), "stopped" | "error");
    let unchanged = previous.as_ref().is_some_and(|old| {
        old.title == instance.title
            && old.project_path == instance.project_path
            && old.project_key == project_key
            && old.agent == instance.tool
            && old.status == status
            && old.session_id.as_deref() == Some(&instance.id)
            && old.available
            && old.agent_type == instance.agent_name
            && old.task == task
    });
    GraphNode {
        id: session_key(profile, &instance.id),
        session_id: Some(instance.id.clone()),
        owner_session_id: None,
        kind: "session".into(),
        title: instance.title.clone(),
        project_path: instance.project_path.clone(),
        project_key,
        agent: instance.tool.clone(),
        agent_type: instance.agent_name.clone(),
        status,
        task,
        created_at: instance.created_at.to_rfc3339(),
        started_at,
        ended_at: previous
            .as_ref()
            .and_then(|node| {
                if stopped_or_error {
                    node.ended_at.clone().or_else(|| Some(now.clone()))
                } else {
                    None
                }
            })
            .or_else(|| stopped_or_error.then(|| now.clone())),
        updated_at: if unchanged {
            previous
                .as_ref()
                .expect("unchanged node exists")
                .updated_at
                .clone()
        } else {
            now
        },
        available: true,
        history_complete: false,
    }
}

/// Whether the input is the authoritative full profile snapshot.
#[derive(Debug, Clone, Copy)]
pub struct ObserveOptions {
    /// Marks nodes absent from `instances` unavailable. Keep false for partial snapshots.
    pub mark_missing_unavailable: bool,
}

pub fn observe_sessions(
    profile: &str,
    profile_dir: &Path,
    instances: &[Instance],
    options: ObserveOptions,
) -> Result<()> {
    let mut conn = open(profile_dir)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut seen = HashSet::new();
    let mut changed = false;
    for instance in instances {
        let id = session_key(profile, &instance.id);
        seen.insert(id.clone());
        let old = existing_node(&tx, &id)?;
        let node = session_node(profile, instance, old);
        changed |= upsert_node(&tx, &node)?;
        if let Some(parent) = &instance.parent_session_id {
            let source = session_key(profile, parent);
            changed |= upsert_edge(
                &tx,
                &GraphEdge {
                    id: format!("explicit_parent:{source}:{id}"),
                    source,
                    target: id,
                    kind: "explicit_parent".into(),
                    evidence: "sessions_parent".into(),
                },
            )?;
        }
    }
    if options.mark_missing_unavailable {
        let mut stmt = tx.prepare("SELECT id, data FROM graph_nodes")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (id, data) in rows {
            let mut node: GraphNode = serde_json::from_str(&data)?;
            if node.kind == "session" {
                if let Some(session_id) = node.session_id.as_deref() {
                    let canonical = session_key(profile, session_id);
                    if id != canonical && seen.contains(&canonical) {
                        changed |= remove_session_alias(&tx, &id, &canonical)?;
                        continue;
                    }
                }
            }
            if !id.starts_with(&format!("session:{profile}:")) || seen.contains(&id) {
                continue;
            }
            if !node.available {
                continue;
            }
            node.available = false;
            node.status = "stopped".into();
            node.ended_at = Some(Utc::now().to_rfc3339());
            node.updated_at = node.ended_at.clone().unwrap_or_default();
            changed |= upsert_node(&tx, &node)?;
        }
    }
    if changed {
        record_change(&tx, "sessions")?;
    }
    tx.commit()?;
    Ok(())
}

/// Payloads are accepted only after the caller has validated profile, owner and native context.
pub fn record_native_hook(
    profile_dir: &Path,
    owner: &Instance,
    native_session_id: &str,
    agent_id: Option<&str>,
    agent_type: Option<&str>,
    event: NativeHook,
) -> Result<()> {
    let mut conn = open(profile_dir)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let owner_id = session_key(&owner.source_profile, &owner.id);
    let mut changed = false;
    if let Some(agent_id) = agent_id {
        let id = format!(
            "subagent:{}:{}:{}:{}",
            owner.source_profile, owner.tool, native_session_id, agent_id
        );
        let old = existing_node(&tx, &id)?;
        let now = Utc::now().to_rfc3339();
        let status = if event == NativeHook::SubagentStart {
            "running"
        } else {
            "completed"
        };
        let node = GraphNode {
            id: id.clone(),
            session_id: None,
            owner_session_id: Some(owner.id.clone()),
            kind: "subagent".into(),
            title: agent_type
                .map(str::to_string)
                .or_else(|| old.as_ref().map(|node| node.title.clone()))
                .unwrap_or_else(|| "Subagent".into()),
            project_path: owner.project_path.clone(),
            project_key: project_key(&owner.project_path),
            agent: owner.tool.clone(),
            agent_type: agent_type
                .map(str::to_string)
                .or_else(|| old.as_ref().and_then(|n| n.agent_type.clone())),
            status: status.into(),
            task: None,
            created_at: old
                .as_ref()
                .map(|n| n.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            started_at: old
                .as_ref()
                .and_then(|n| n.started_at.clone())
                .or_else(|| Some(now.clone())),
            ended_at: if status == "completed" {
                Some(now.clone())
            } else {
                None
            },
            updated_at: if old.as_ref().is_some_and(|n| n.status == status) {
                old.as_ref().expect("old exists").updated_at.clone()
            } else {
                now
            },
            available: false,
            history_complete: false,
        };
        changed |= upsert_node(&tx, &node)?;
        changed |= upsert_edge(
            &tx,
            &GraphEdge {
                id: format!("native_subagent:{owner_id}:{id}"),
                source: owner_id.clone(),
                target: id,
                kind: "native_subagent".into(),
                evidence: "hook_context".into(),
            },
        )?;
    } else if let Some(mut node) = existing_node(&tx, &owner_id)? {
        let next = match event {
            NativeHook::SessionStart | NativeHook::UserPromptSubmit => "running",
            NativeHook::Stop => "idle",
            NativeHook::SessionEnd => "stopped",
            _ => return Ok(()),
        };
        if node.status != next {
            node.status = next.into();
            node.updated_at = Utc::now().to_rfc3339();
            if event == NativeHook::SessionEnd {
                node.ended_at = Some(node.updated_at.clone());
            } else if next == "running" {
                node.started_at = Some(node.updated_at.clone());
                node.ended_at = None;
            }
            changed |= upsert_node(&tx, &node)?;
        }
    }
    if changed {
        record_change(&tx, "hook")?;
    }
    tx.commit()?;
    Ok(())
}

fn snapshot_from_event(
    conn: &Connection,
    schema: &Schema,
    cursor: u64,
) -> Result<Option<GraphSnapshot>> {
    if cursor == 0 {
        return Ok(None);
    }
    let event = events::scan(
        conn,
        schema,
        TOPIC,
        crate::events::SeqBound::After(cursor - 1),
        crate::events::Order::Asc,
        Some(1),
    )
    .into_iter()
    .find(|(seq, _)| *seq == cursor);
    let Some((_, json)) = event else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(&json)?;
    let nodes = value["nodes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("graph event lacks node snapshot"))?
        .iter()
        .map(|json| -> Result<GraphNode> {
            let json = json
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid graph node event"))?;
            Ok(serde_json::from_str(json)?)
        })
        .collect::<Result<Vec<_>>>()?;
    let edges = value["edges"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("graph event lacks edge snapshot"))?
        .iter()
        .map(|json| -> Result<GraphEdge> {
            let json = json
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid graph edge event"))?;
            Ok(serde_json::from_str(json)?)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(GraphSnapshot {
        nodes,
        edges,
        cursor,
    }))
}

pub fn snapshot(profile_dir: &Path) -> Result<GraphSnapshot> {
    let mut conn = open(profile_dir)?;
    let conn = conn.transaction()?;
    let schema = Schema::new("agent_graph")?;
    let cursor = events::highest_seq(&conn, &schema, TOPIC);
    if let Some(snapshot) = snapshot_from_event(&conn, &schema, cursor)? {
        if snapshot.nodes.len() > MAX_NODES {
            return Err(SnapshotLimitExceeded.into());
        }
        return Ok(snapshot);
    }
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM graph_nodes", [], |row| row.get(0))?;
    if count > MAX_NODES as i64 {
        return Err(SnapshotLimitExceeded.into());
    }
    let mut stmt = conn.prepare("SELECT data FROM graph_nodes ORDER BY id")?;
    let nodes = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|json| serde_json::from_str::<GraphNode>(&json?).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut stmt = conn.prepare("SELECT data FROM graph_edges ORDER BY id")?;
    let edges = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|json| serde_json::from_str::<GraphEdge>(&json?).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()))
        .collect();
    Ok(GraphSnapshot {
        nodes,
        edges,
        cursor,
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
