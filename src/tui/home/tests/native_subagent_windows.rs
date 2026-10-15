use super::*;
use crate::session::{ConversationBinding, ConversationProvenance, ExecutionBinding};
use std::path::PathBuf;

fn selected_claude_env() -> (TestEnv, PathBuf) {
    let (temp, guard) = test_home();
    let project = temp.path().join("project");
    let store = temp.path().join("claude");
    std::fs::create_dir_all(&project).unwrap();
    let session_id = "11111111-2222-4333-8444-555555555555";
    let transcript = store
        .join("projects")
        .join(
            super::super::native_subagent_transcript::encode_claude_project_path(
                &project.to_string_lossy(),
            ),
        )
        .join(format!("{session_id}.jsonl"));
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(&transcript, "{}\n").unwrap();

    let mut instance = Instance::new("claude", project.to_str().unwrap());
    instance.source_profile = "test".into();
    instance.tool = "claude".into();
    instance.agent_session_id = Some(session_id.into());
    instance.agent_session_binding = Some(ConversationBinding {
        session_id: session_id.into(),
        execution: Some(ExecutionBinding {
            agent: "claude".into(),
            stores: vec![store],
            configuration: vec![],
            cwd: project,
            cwd_filesystem: "host".into(),
            filesystem: "host".into(),
            exported_default_store: None,
        }),
        provenance: ConversationProvenance::Observed,
        transcript_path: Some(transcript.clone()),
    });
    let mut env = seeded_env((temp, guard), &[instance], true);
    env.view.selected_session = env.view.instances.keys().next().cloned();
    env.view.view_mode = ViewMode::Terminal;
    (env, transcript)
}

fn wait_for_window(view: &mut HomeView) {
    let start = std::time::Instant::now();
    loop {
        view.refresh_native_subagent_windows();
        if !view.native_subagent_windows.windows.is_empty() {
            return;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "native sub-agent scan timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn selected_terminal_claude_session_opens_draggable_chat() {
    let (mut env, transcript) = selected_claude_env();
    let subagents = transcript.with_extension("").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    let agent = subagents.join("agent-a1.jsonl");
    std::fs::write(
        &agent,
        concat!(
            r#"{"isSidechain":true,"type":"user","message":{"role":"user","content":"inspect TUI"}}"#,
            "\n",
            r#"{"isSidechain":true,"message":{"role":"assistant","content":[{"type":"text","text":"live reply"}],"stop_reason":null}}"#,
            "\n"
        ),
    )
    .unwrap();
    std::fs::write(
        agent.with_extension("meta.json"),
        r#"{"description":"Analyze TUI"}"#,
    )
    .unwrap();

    wait_for_window(&mut env.view);
    let screen = render_home_to_string(&mut env.view, 120, 40);
    assert!(screen.contains("Analyze TUI · running"), "{screen}");
    assert!(screen.contains("live reply"), "{screen}");
    let initial_target = env.view.native_subagent_windows.targets[0].clone();
    assert_eq!(initial_target.area.width, 80);
    assert_eq!(initial_target.area.height, 24);
    let expired_at = env.view.native_subagent_windows.windows[0]
        .visible_until
        .unwrap()
        + std::time::Duration::from_millis(1);
    assert!(env.view.expire_native_subagent_popups(expired_at));
    let hidden = render_home_to_string(&mut env.view, 120, 40);
    assert!(!hidden.contains("Analyze TUI · running"), "{hidden}");
    assert!(env.view.native_subagent_windows.targets.is_empty());

    assert!(!env.view.native_subagent_edit_mode());
    assert!(!env.view.native_subagent_mouse_down(0, 0));
    assert!(env.view.native_subagent_toggle_edit_mode());
    let visible = render_home_to_string(&mut env.view, 120, 40);
    assert!(visible.contains("Analyze TUI · running"), "{visible}");
    assert!(visible.contains("l done"), "{visible}");
    let target = env.view.native_subagent_windows.targets[0].clone();
    let down_x = target.title.x + 2;
    assert!(env.view.native_subagent_mouse_down(down_x, target.title.y));
    assert!(env.view.native_subagent_dragging());
    assert!(env
        .view
        .native_subagent_drag_move(down_x.saturating_sub(10), target.title.y + 4));
    assert!(env.view.native_subagent_drag_end());
    let moved = env.view.native_subagent_windows.windows.last().unwrap();
    assert!(moved.position.is_some());

    let screen = render_home_to_string(&mut env.view, 120, 40);
    assert!(screen.contains("EDIT: drag title"), "{screen}");
    let resize = env.view.native_subagent_windows.targets[0].resize;
    assert!(env.view.native_subagent_mouse_down(resize.x, resize.y));
    assert!(env
        .view
        .native_subagent_drag_move(resize.x + 8, resize.y + 3));
    assert!(env.view.native_subagent_drag_end());
    assert!(env
        .view
        .native_subagent_windows
        .windows
        .last()
        .unwrap()
        .size
        .is_some());
}

#[test]
fn closed_native_chat_does_not_reopen_on_refresh() {
    let (mut env, transcript) = selected_claude_env();
    let subagents = transcript.with_extension("").join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    std::fs::write(
        subagents.join("agent-a1.jsonl"),
        r#"{"isSidechain":true,"type":"user","message":{"role":"user","content":"inspect"}}"#,
    )
    .unwrap();
    wait_for_window(&mut env.view);
    assert!(env.view.native_subagent_toggle_edit_mode());
    render_home_to_string(&mut env.view, 120, 40);
    let close = env.view.native_subagent_windows.targets[0].close;
    assert!(env.view.native_subagent_mouse_down(close.x, close.y));
    assert!(env.view.native_subagent_windows.windows.is_empty());
    let native_parent = env.view.native_subagent_windows.session_id.clone().unwrap();

    env.view.native_subagent_windows.last_refresh = None;
    std::thread::sleep(std::time::Duration::from_millis(20));
    env.view.refresh_native_subagent_windows();
    std::thread::sleep(std::time::Duration::from_millis(20));
    env.view.refresh_native_subagent_windows();
    assert!(env.view.native_subagent_windows.windows.is_empty());

    let session_id = env.view.selected_session.clone().unwrap();
    env.view.selected_session = None;
    env.view.refresh_native_subagent_windows();
    env.view.selected_session = Some(session_id);
    env.view.native_subagent_windows.last_refresh = None;
    std::thread::sleep(std::time::Duration::from_millis(20));
    env.view.refresh_native_subagent_windows();
    std::thread::sleep(std::time::Duration::from_millis(20));
    env.view.refresh_native_subagent_windows();
    assert!(env.view.native_subagent_windows.windows.is_empty());
    assert_eq!(
        env.view.native_subagent_windows.session_id.as_deref(),
        Some(native_parent.as_str())
    );
}
