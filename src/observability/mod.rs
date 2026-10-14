//! Durable, profile-local projection of observed AoE sessions and native subagents.
mod store;
pub use store::{
    backfill_project_keys, observe_sessions, record_native_hook, snapshot, GraphSnapshot, NativeHook,
    ObserveOptions, SnapshotLimitExceeded, MAX_NODES,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub session_id: Option<String>,
    pub owner_session_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub project_path: String,
    #[serde(default)]
    pub project_key: String,
    pub agent: String,
    pub agent_type: Option<String>,
    pub status: String,
    pub task: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub updated_at: String,
    pub available: bool,
    pub history_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: String,
    pub evidence: String,
}

pub(crate) fn session_key(profile: &str, id: &str) -> String {
    format!("session:{profile}:{id}")
}
