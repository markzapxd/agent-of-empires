use super::*;
use crate::session::Status;

fn observe_sessions_authoritative(
    profile: &str,
    profile_dir: &Path,
    instances: &[Instance],
) -> Result<()> {
    observe_sessions(
        profile,
        profile_dir,
        instances,
        ObserveOptions {
            mark_missing_unavailable: true,
        },
    )
}

#[test]
fn reconciles_deletion_and_keeps_evidenced_edges() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut parent = Instance::new("parent", "/repo");
    parent.source_profile = "default".into();
    parent.status = Status::Running;
    let mut child = Instance::new("child", "/repo");
    child.source_profile = "default".into();
    child.parent_session_id = Some(parent.id.clone());
    observe_sessions_authoritative("default", dir.path(), &[parent.clone(), child.clone()])?;
    let first = snapshot(dir.path())?;
    assert_eq!(first.nodes.len(), 2);
    assert_eq!(first.edges.len(), 1);
    assert_eq!(first.edges[0].kind, "explicit_parent");
    assert!(first.nodes.iter().all(|node| !node.project_key.is_empty()));

    observe_sessions_authoritative("default", dir.path(), &[])?;
    let second = snapshot(dir.path())?;
    assert_eq!(second.nodes.len(), 2);
    assert!(second
        .nodes
        .iter()
        .all(|node| !node.available && node.ended_at.is_some()));
    assert_eq!(second.edges, first.edges);
    assert!(second.cursor > first.cursor);
    observe_sessions_authoritative("default", dir.path(), &[])?;
    assert_eq!(snapshot(dir.path())?.cursor, second.cursor);
    Ok(())
}

#[test]
fn canonicalizes_legacy_empty_profile_nodes_and_edges() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut parent = Instance::new("parent", "/repo");
    parent.source_profile = "main".into();
    let mut child = Instance::new("child", "/repo");
    child.source_profile = "main".into();
    child.parent_session_id = Some(parent.id.clone());
    observe_sessions_authoritative("main", dir.path(), &[parent.clone(), child.clone()])?;

    let conn = open(dir.path())?;
    let canonical_parent = session_key("main", &parent.id);
    let legacy_parent = session_key("", &parent.id);
    let mut legacy_node = existing_node(&conn, &canonical_parent)?.expect("parent node");
    legacy_node.id.clone_from(&legacy_parent);
    upsert_node(&conn, &legacy_node)?;
    let child_id = session_key("main", &child.id);
    upsert_edge(
        &conn,
        &GraphEdge {
            id: format!("explicit_parent:{legacy_parent}:{child_id}"),
            source: legacy_parent,
            target: child_id.clone(),
            kind: "explicit_parent".into(),
            evidence: "sessions_parent".into(),
        },
    )?;
    record_change(&conn, "legacy_fixture")?;

    observe_sessions_authoritative("main", dir.path(), &[parent, child])?;
    let graph = snapshot(dir.path())?;
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].source, canonical_parent);
    assert_eq!(graph.edges[0].target, child_id);
    Ok(())
}

#[test]
fn backfills_legacy_project_keys() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut row = Instance::new("project", dir.path().to_string_lossy().to_string());
    row.source_profile = "default".into();
    observe_sessions_authoritative("default", dir.path(), &[row])?;
    let conn = open(dir.path())?;
    let mut node = snapshot(dir.path())?.nodes.pop().expect("graph node");
    node.project_key.clear();
    upsert_node(&conn, &node)?;
    backfill_project_keys(dir.path())?;
    let restored = snapshot(dir.path())?.nodes.pop().expect("graph node");
    assert!(!restored.project_key.is_empty());
    let cursor = snapshot(dir.path())?.cursor;
    backfill_project_keys(dir.path())?;
    assert_eq!(snapshot(dir.path())?.cursor, cursor);
    Ok(())
}

#[test]
fn native_subagent_has_stable_identity_and_owner() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut owner = Instance::new("owner", "/repo");
    owner.source_profile = "default".into();
    observe_sessions_authoritative("default", dir.path(), &[owner.clone()])?;
    record_native_hook(
        dir.path(),
        &owner,
        "sid",
        Some("agent-1"),
        Some("Explore"),
        NativeHook::SubagentStart,
    )?;
    let first = snapshot(dir.path())?;
    assert_eq!(first.nodes.len(), 2);
    assert_eq!(first.edges.len(), 1);
    let agent = first
        .nodes
        .iter()
        .find(|node| node.kind == "subagent")
        .unwrap();
    assert_eq!(agent.owner_session_id.as_deref(), Some(owner.id.as_str()));
    assert_eq!(agent.status, "running");

    record_native_hook(
        dir.path(),
        &owner,
        "sid",
        Some("agent-1"),
        Some("Explore"),
        NativeHook::SubagentStop,
    )?;
    let second = snapshot(dir.path())?;
    assert_eq!(second.edges, first.edges);
    let agent = second
        .nodes
        .iter()
        .find(|node| node.kind == "subagent")
        .unwrap();
    assert_eq!(agent.status, "completed");
    assert!(agent.ended_at.is_some());
    Ok(())
}

#[test]
fn captures_only_bounded_user_supplied_initial_task() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut row = Instance::new("task", "/repo");
    row.source_profile = "default".into();
    row.pending_initial_turn = Some(crate::session::PendingInitialTurn {
        text: format!("  {}  ", "x".repeat(700)),
        attachments: Vec::new(),
        synthesized: false,
    });
    observe_sessions_authoritative("default", dir.path(), &[row.clone()])?;
    let task = snapshot(dir.path())?.nodes[0].task.clone().unwrap();
    assert_eq!(task.chars().count(), 500);

    let mut synthesized = Instance::new("synthesized", "/repo");
    synthesized.source_profile = "default".into();
    synthesized.pending_initial_turn = Some(crate::session::PendingInitialTurn {
        text: "restart continuation".into(),
        attachments: Vec::new(),
        synthesized: true,
    });
    observe_sessions_authoritative("default", dir.path(), &[row, synthesized])?;
    assert!(snapshot(dir.path())?
        .nodes
        .iter()
        .find(|node| node.title == "synthesized")
        .is_some_and(|node| node.task.is_none()));
    Ok(())
}

#[test]
fn snapshot_replays_last_durable_event_when_projection_is_damaged() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut row = Instance::new("durable", "/repo");
    row.source_profile = "default".into();
    observe_sessions_authoritative("default", dir.path(), &[row])?;
    let expected = snapshot(dir.path())?;
    let conn = open(dir.path())?;
    conn.execute("DELETE FROM graph_nodes", [])?;
    conn.execute("DELETE FROM graph_edges", [])?;
    assert_eq!(snapshot(dir.path())?, expected);
    Ok(())
}

#[test]
fn snapshot_rejects_more_than_limit_instead_of_truncating() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut conn = open(dir.path())?;
    let tx = conn.transaction()?;
    for index in 0..=MAX_NODES {
        tx.execute(
            "INSERT INTO graph_nodes(id, data) VALUES (?1, '{}')",
            [format!("node-{index}")],
        )?;
    }
    tx.commit()?;
    let error = snapshot(dir.path()).unwrap_err();
    assert!(error.is::<SnapshotLimitExceeded>());
    Ok(())
}
