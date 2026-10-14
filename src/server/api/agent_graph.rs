//! Read-only, profile-scoped graph HTTP and WebSocket surfaces.
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use tokio::sync::broadcast::error::RecvError;

use super::AppState;
use crate::observability::{self, GraphSnapshot};

fn load(state: &AppState, rows: &[crate::session::Instance]) -> anyhow::Result<GraphSnapshot> {
    let profile = crate::session::config::effective_profile(&state.profile);
    let path = crate::session::get_profile_dir_path(&profile)?;
    let rows = rows
        .iter()
        .filter(|row| row.effective_profile() == profile)
        .cloned()
        .collect::<Vec<_>>();
    observability::observe_sessions(
        &profile,
        &path,
        &rows,
        observability::ObserveOptions {
            mark_missing_unavailable: false,
        },
    )?;
    observability::snapshot(&path)
}

pub async fn get_agent_graph(State(state): State<Arc<AppState>>) -> Response {
    if state.cityhall_mode {
        return StatusCode::FORBIDDEN.into_response();
    }
    let rows = state.instances.read().await.clone();
    match tokio::task::spawn_blocking(move || load(&state, &rows)).await {
        Ok(Ok(snapshot)) => Json(snapshot).into_response(),
        Ok(Err(error)) if error.is::<observability::SnapshotLimitExceeded>() => super::api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "agent_graph_limit",
            format!(
                "Agent graph exceeds the {}-node snapshot limit",
                observability::MAX_NODES
            ),
        ),
        error => {
            tracing::warn!("agent graph snapshot failed: {error:?}");
            super::api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent_graph_unavailable",
                "Agent graph snapshot is unavailable",
            )
        }
    }
}

pub async fn agent_graph_ws(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    if state.cityhall_mode {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.protocols(["aoe-auth"])
        .on_upgrade(move |socket| stream(socket, state))
        .into_response()
}

async fn stream(mut socket: WebSocket, state: Arc<AppState>) {
    let shutdown = state.shutdown.clone();
    let profile = crate::session::config::effective_profile(&state.profile);
    let mut statuses = state.status_tx.subscribe();
    let mut cursor = None;
    let mut next_disk_probe = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_secs(3));
    let mut shutting_down = false;
    loop {
        let refresh = tokio::select! {
            _ = shutdown.cancelled() => {
                shutting_down = true;
                break
            },
            _ = interval.tick() => true,
            status = statuses.recv() => {
                match status {
                    Ok(change) if change.effective_profile == profile => true,
                    Ok(_) => false,
                    Err(RecvError::Lagged(_)) => true,
                    Err(RecvError::Closed) => break,
                }
            }
            _ = state.disk_changed.notified(), if Instant::now() >= next_disk_probe => {
                next_disk_probe = Instant::now() + Duration::from_millis(250);
                true
            }
            received = socket.recv() => {
                if !matches!(received, Some(Ok(Message::Ping(_) | Message::Pong(_)))) {
                    break;
                }
                false
            }
        };
        if !refresh {
            continue;
        }
        let rows = state.instances.read().await.clone();
        let state_for_load = state.clone();
        let result = tokio::task::spawn_blocking(move || load(&state_for_load, &rows)).await;
        match result {
            Ok(Ok(snapshot)) if cursor != Some(snapshot.cursor) => {
                let kind = if cursor.is_none() {
                    "snapshot"
                } else {
                    "update"
                };
                let text = serde_json::json!({"type": kind, "snapshot": snapshot}).to_string();
                cursor = Some(snapshot.cursor);
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            Ok(Ok(_)) => {}
            error => {
                tracing::warn!("agent graph stream failed: {error:?}");
                break;
            }
        }
    }
    if shutting_down {
        let _ = socket
            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                code: 1001,
                reason: "server shutdown".into(),
            })))
            .await;
    }
}
