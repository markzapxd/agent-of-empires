//! Hidden hook entrypoint. Accept only AoE-owned local session and native identifiers.
use std::io::Read;

use anyhow::{bail, Result};
use clap::Args;

use crate::observability::{self, NativeHook};

#[derive(Args)]
pub struct AgentGraphHookArgs {
    #[arg(long, value_parser = ["claude", "codex"])]
    provider: String,
    #[arg(long, value_parser = ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd", "SubagentStart", "SubagentStop"])]
    event: String,
}

pub async fn run(args: AgentGraphHookArgs) -> Result<()> {
    if let Err(error) = run_inner(&args, std::io::stdin().lock()) {
        tracing::warn!(target: "hooks.agent_graph", "graph hook ignored: {error:#}");
    }
    Ok(())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn event_name(name: &str) -> Result<NativeHook> {
    Ok(match name {
        "SessionStart" => NativeHook::SessionStart,
        "UserPromptSubmit" => NativeHook::UserPromptSubmit,
        "Stop" => NativeHook::Stop,
        "SessionEnd" => NativeHook::SessionEnd,
        "SubagentStart" => NativeHook::SubagentStart,
        "SubagentStop" => NativeHook::SubagentStop,
        _ => bail!("unsupported event"),
    })
}

fn run_inner(args: &AgentGraphHookArgs, stdin: impl Read) -> Result<()> {
    let profile = crate::session::resolve_existing_profile(&std::env::var("AOE_PROFILE")?)?;
    let instance_id = std::env::var("AOE_INSTANCE_ID")?;
    crate::session::validate_instance_id(&instance_id)?;
    let current_agent = std::env::var("AOE_AGENT_BIN")?;
    let profile_dir = crate::session::get_profile_dir_path(&profile)?;
    let storage = crate::session::Storage::open_unwatched(&profile)?;
    let owner = storage
        .load()?
        .into_iter()
        .find(|instance| instance.id == instance_id && instance.tool == args.provider)
        .ok_or_else(|| anyhow::anyhow!("owner session not found"))?;
    let expected_agent = owner
        .resolved_agent()
        .map(|agent| agent.binary)
        .ok_or_else(|| anyhow::anyhow!("owner agent is unsupported"))?;
    let event = event_name(&args.event)?;
    if current_agent != expected_agent || !super::extract_session_id::fired_by_pane_agent() {
        bail!("hook publisher does not match pane agent");
    }
    let mut buf = Vec::new();
    stdin.take(64 * 1024 + 1).read_to_end(&mut buf)?;
    if buf.len() > 64 * 1024 {
        bail!("hook exceeds size limit");
    }
    let payload: serde_json::Value = serde_json::from_slice(&buf)?;
    if payload
        .get("hook_event_name")
        .and_then(|value| value.as_str())
        != Some(&args.event)
    {
        bail!("event mismatch");
    }
    let native_session_id = payload
        .get("session_id")
        .and_then(|value| value.as_str())
        .filter(|value| safe_id(value))
        .ok_or_else(|| anyhow::anyhow!("missing native session id"))?;
    let authority = std::env::var(crate::hooks::SESSION_SOURCE_ENV)?;
    let receipt = crate::tmux::env::get_hidden_env(
        &owner.tmux_session()?.name(),
        crate::hooks::SESSION_SOURCE_ENV,
    );
    if !matches!(event, NativeHook::SessionStart) && receipt.as_deref() != Some(authority.as_str())
    {
        bail!("hook launch does not own this AoE session");
    }
    if matches!(event, NativeHook::SessionStart) {
        if receipt
            .as_deref()
            .is_some_and(|receipt| receipt != authority)
        {
            bail!("another launch owns this AoE session");
        }
        crate::tmux::env::set_hidden_env(
            &owner.tmux_session()?.name(),
            crate::hooks::SESSION_SOURCE_ENV,
            &authority,
        )?;
    }
    let agent_id = if matches!(event, NativeHook::SubagentStart | NativeHook::SubagentStop) {
        Some(
            payload
                .get("agent_id")
                .and_then(|value| value.as_str())
                .filter(|value| safe_id(value))
                .ok_or_else(|| anyhow::anyhow!("missing explicit agent_id"))?,
        )
    } else {
        None
    };
    let agent_type = payload
        .get("agent_type")
        .and_then(|value| value.as_str())
        .filter(|value| value.len() <= 128 && !value.chars().any(char::is_control));
    observability::observe_sessions(
        &profile,
        &profile_dir,
        &storage.load()?,
        observability::ObserveOptions {
            mark_missing_unavailable: true,
        },
    )?;
    observability::record_native_hook(
        &profile_dir,
        &owner,
        native_session_id,
        agent_id,
        agent_type,
        event,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_untrusted_agent_ids() {
        for value in ["", "../escape", "id/foreign", "space id", "a\n"] {
            assert!(!safe_id(value));
        }
        assert!(safe_id("f1e2d3-a4b5"));
    }
}
