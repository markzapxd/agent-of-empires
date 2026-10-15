//! Floating chats for native Claude sub-agents in terminal-view sessions.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::native_subagent_transcript::{native_claude_transcript, scan_subagents};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::HomeView;
use crate::tui::styles::Theme;
use crate::tui::worker::Worker;

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const WINDOW_WIDTH: u16 = 62;
const WINDOW_HEIGHT: u16 = 18;
const MIN_WINDOW_WIDTH: u16 = 28;
const MIN_WINDOW_HEIGHT: u16 = 8;

#[derive(Clone)]
pub(super) struct NativeSubagentMessage {
    pub(super) text: String,
}

#[derive(Clone)]
pub(super) struct NativeSubagentRecord {
    pub(super) id: String,
    pub(super) description: String,
    pub(super) prompt: String,
    pub(super) running: bool,
    pub(super) messages: Vec<NativeSubagentMessage>,
}

pub(super) struct NativeSubagentWindow {
    pub(super) agent_id: String,
    pub(super) position: Option<(u16, u16)>,
    pub(super) size: Option<(u16, u16)>,
    pub(super) scroll: u16,
    pub(super) last_scroll_max: u16,
}

#[derive(Clone)]
pub(super) struct NativeSubagentTarget {
    pub(super) agent_id: String,
    pub(super) area: Rect,
    pub(super) title: Rect,
    pub(super) close: Rect,
    pub(super) resize: Rect,
}

#[derive(Clone, Copy)]
pub(super) enum NativeSubagentDragKind {
    Move { grab_x: u16, grab_y: u16 },
    Resize { origin_x: u16, origin_y: u16 },
}

#[derive(Clone)]
pub(super) struct NativeSubagentDrag {
    pub(super) agent_id: String,
    pub(super) kind: NativeSubagentDragKind,
}

#[derive(Clone)]
pub(super) struct NativeSubagentRequest {
    session_id: String,
    transcript: PathBuf,
}

pub(super) struct NativeSubagentResult {
    session_id: String,
    records: Vec<NativeSubagentRecord>,
}

pub(in crate::tui) struct NativeSubagentWindows {
    pub(super) session_id: Option<String>,
    records: Vec<NativeSubagentRecord>,
    pub(super) windows: Vec<NativeSubagentWindow>,
    known: HashSet<String>,
    pub(super) targets: Vec<NativeSubagentTarget>,
    drag: Option<NativeSubagentDrag>,
    edit_mode: bool,
    worker: Worker<NativeSubagentRequest, NativeSubagentResult>,
    in_flight: bool,
    pub(super) last_refresh: Option<Instant>,
}

impl NativeSubagentWindows {
    pub(super) fn new() -> Self {
        Self {
            session_id: None,
            records: Vec::new(),
            windows: Vec::new(),
            known: HashSet::new(),
            targets: Vec::new(),
            drag: None,
            edit_mode: false,
            worker: Worker::spawn("aoe-native-subagent", |request: NativeSubagentRequest| {
                let records = scan_subagents(&request.transcript);
                NativeSubagentResult {
                    session_id: request.session_id,
                    records,
                }
            }),
            in_flight: false,
            last_refresh: None,
        }
    }

    fn reset_for(&mut self, session_id: Option<&str>) -> bool {
        if self.session_id.as_deref() == session_id {
            return false;
        }
        self.session_id = session_id.map(str::to_owned);
        self.records.clear();
        self.windows.clear();
        self.known.clear();
        self.targets.clear();
        self.drag = None;
        self.edit_mode = false;
        self.in_flight = false;
        self.last_refresh = None;
        true
    }

    fn reconcile(&mut self, records: Vec<NativeSubagentRecord>) -> bool {
        let mut changed = records.len() != self.records.len();
        if !changed {
            changed = records.iter().zip(&self.records).any(|(next, old)| {
                next.id != old.id
                    || next.running != old.running
                    || next.description != old.description
                    || next.prompt != old.prompt
                    || next.messages.len() != old.messages.len()
                    || next.messages.last().map(|message| message.text.as_str())
                        != old.messages.last().map(|message| message.text.as_str())
            });
        }
        self.records = records;
        for record in &self.records {
            if self.known.insert(record.id.clone()) {
                self.windows.push(NativeSubagentWindow {
                    agent_id: record.id.clone(),
                    position: None,
                    size: None,
                    scroll: u16::MAX,
                    last_scroll_max: 0,
                });
                changed = true;
            }
        }
        self.windows.retain(|window| {
            self.records
                .iter()
                .any(|record| record.id == window.agent_id)
        });
        if self.drag.as_ref().is_some_and(|drag| {
            !self
                .windows
                .iter()
                .any(|window| window.agent_id == drag.agent_id)
        }) {
            self.drag = None;
        }
        changed
    }

    fn raise(&mut self, agent_id: &str) {
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.agent_id == agent_id)
        else {
            return;
        };
        let window = self.windows.remove(index);
        self.windows.push(window);
    }
}

impl HomeView {
    pub(in crate::tui) fn refresh_native_subagent_windows(&mut self) -> bool {
        let selected = self.selected_session.clone();
        let transcript = selected
            .as_deref()
            .and_then(|id| self.get_instance(id))
            .and_then(native_claude_transcript);
        let active_id = transcript.as_ref().and(selected.as_deref());
        let mut changed = self.native_subagent_windows.reset_for(active_id);

        match self.native_subagent_windows.worker.try_recv() {
            Ok(result) => {
                self.native_subagent_windows.in_flight = false;
                if self.native_subagent_windows.session_id.as_deref()
                    == Some(result.session_id.as_str())
                {
                    changed |= self.native_subagent_windows.reconcile(result.records);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.native_subagent_windows.in_flight = false;
            }
        }

        let due = self
            .native_subagent_windows
            .last_refresh
            .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL);
        if due && !self.native_subagent_windows.in_flight {
            if let (Some(session_id), Some(transcript)) = (active_id.map(str::to_owned), transcript)
            {
                self.native_subagent_windows
                    .worker
                    .request(NativeSubagentRequest {
                        session_id,
                        transcript,
                    });
                self.native_subagent_windows.in_flight = true;
                self.native_subagent_windows.last_refresh = Some(Instant::now());
            }
        }
        changed
    }

    pub(super) fn render_native_subagent_windows(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
    ) {
        self.native_subagent_windows.targets.clear();
        for (index, window) in self.native_subagent_windows.windows.iter_mut().enumerate() {
            let Some(record) = self
                .native_subagent_windows
                .records
                .iter()
                .find(|record| record.id == window.agent_id)
            else {
                continue;
            };
            let popup = window_rect(area, window.position, window.size, index);
            if popup.width < 8 || popup.height < 4 {
                continue;
            }
            let status = if record.running { "running" } else { "done" };
            let title = if record.description.is_empty() {
                format!(" Sub-agent · {status} ")
            } else {
                format!(" {} · {status} ", record.description)
            };
            let edit_mode = self.native_subagent_windows.edit_mode;
            let border_style =
                Style::default().fg(if edit_mode { theme.accent } else { theme.title });
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .padding(Padding::horizontal(1))
                .title(title)
                .title_bottom(Line::from(Span::styled(
                    if edit_mode {
                        " EDIT: drag title · drag ◢ resize · × close · L done "
                    } else {
                        " L edit "
                    },
                    Style::default().fg(if edit_mode {
                        theme.accent
                    } else {
                        theme.dimmed
                    }),
                )))
                .border_style(border_style);
            let inner = block.inner(popup);
            frame.render_widget(Clear, popup);
            frame.render_widget(block, popup);
            let close = Rect::new(popup.right().saturating_sub(4), popup.y, 3, 1);
            let resize = Rect::new(
                popup.right().saturating_sub(3),
                popup.bottom().saturating_sub(2),
                3,
                2,
            );
            if edit_mode {
                frame.render_widget(
                    Paragraph::new(Span::styled(" × ", Style::default().fg(theme.accent))),
                    close,
                );
                frame.render_widget(
                    Paragraph::new(Span::styled(" ◢ ", Style::default().fg(theme.accent))),
                    resize,
                );
            }
            if inner.width > 0 && inner.height > 0 {
                let lines = native_chat_lines(record, theme);
                let rows = wrapped_rows(&lines, inner.width);
                window.last_scroll_max = rows.saturating_sub(inner.height);
                let offset = window.scroll.min(window.last_scroll_max);
                frame.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .scroll((offset, 0)),
                    inner,
                );
            }
            self.native_subagent_windows
                .targets
                .push(NativeSubagentTarget {
                    agent_id: window.agent_id.clone(),
                    area: popup,
                    title: Rect::new(popup.x, popup.y, popup.width, 1),
                    close,
                    resize,
                });
        }
    }

    pub(in crate::tui) fn native_subagent_edit_mode(&self) -> bool {
        self.native_subagent_windows.edit_mode
    }

    pub(in crate::tui) fn native_subagent_toggle_edit_mode(&mut self) -> bool {
        if self.native_subagent_windows.windows.is_empty() {
            return false;
        }
        self.native_subagent_windows.edit_mode = !self.native_subagent_windows.edit_mode;
        if !self.native_subagent_windows.edit_mode {
            self.native_subagent_windows.drag = None;
        }
        true
    }

    pub(in crate::tui) fn native_subagent_mouse_down(&mut self, col: u16, row: u16) -> bool {
        if self.has_non_live_send_overlay() || !self.native_subagent_windows.edit_mode {
            return false;
        }
        let pos = Position::new(col, row);
        let Some(target) = self
            .native_subagent_windows
            .targets
            .iter()
            .rev()
            .find(|target| target.area.contains(pos))
            .cloned()
        else {
            return false;
        };
        if target.close.contains(pos) {
            self.native_subagent_windows
                .windows
                .retain(|window| window.agent_id != target.agent_id);
            self.native_subagent_windows.drag = None;
            if self.native_subagent_windows.windows.is_empty() {
                self.native_subagent_windows.edit_mode = false;
            }
            return true;
        }
        self.native_subagent_windows.raise(&target.agent_id);
        let kind = if target.resize.contains(pos) {
            NativeSubagentDragKind::Resize {
                origin_x: target.area.x,
                origin_y: target.area.y,
            }
        } else if target.title.contains(pos) {
            NativeSubagentDragKind::Move {
                grab_x: col.saturating_sub(target.area.x),
                grab_y: row.saturating_sub(target.area.y),
            }
        } else {
            return true;
        };
        self.native_subagent_windows.drag = Some(NativeSubagentDrag {
            agent_id: target.agent_id,
            kind,
        });
        true
    }

    pub(in crate::tui) fn native_subagent_dragging(&self) -> bool {
        self.native_subagent_windows.drag.is_some()
    }

    pub(in crate::tui) fn native_subagent_drag_move(&mut self, col: u16, row: u16) -> bool {
        let Some(drag) = self.native_subagent_windows.drag.clone() else {
            return false;
        };
        let Some(window) = self
            .native_subagent_windows
            .windows
            .iter_mut()
            .find(|window| window.agent_id == drag.agent_id)
        else {
            self.native_subagent_windows.drag = None;
            return false;
        };
        match drag.kind {
            NativeSubagentDragKind::Move { grab_x, grab_y } => {
                window.position = Some((col.saturating_sub(grab_x), row.saturating_sub(grab_y)));
            }
            NativeSubagentDragKind::Resize { origin_x, origin_y } => {
                window.size = Some((
                    col.saturating_sub(origin_x)
                        .saturating_add(1)
                        .max(MIN_WINDOW_WIDTH),
                    row.saturating_sub(origin_y)
                        .saturating_add(1)
                        .max(MIN_WINDOW_HEIGHT),
                ));
            }
        }
        true
    }

    pub(in crate::tui) fn native_subagent_drag_end(&mut self) -> bool {
        self.native_subagent_windows.drag.take().is_some()
    }

    pub(in crate::tui) fn native_subagent_scroll(
        &mut self,
        col: u16,
        row: u16,
        delta: i32,
    ) -> bool {
        if self.has_non_live_send_overlay() {
            return false;
        }
        let pos = Position::new(col, row);
        let Some(agent_id) = self
            .native_subagent_windows
            .targets
            .iter()
            .rev()
            .find(|target| target.area.contains(pos))
            .map(|target| target.agent_id.clone())
        else {
            return false;
        };
        let Some(window) = self
            .native_subagent_windows
            .windows
            .iter_mut()
            .find(|window| window.agent_id == agent_id)
        else {
            return false;
        };
        let current = window.scroll.min(window.last_scroll_max);
        if delta < 0 {
            window.scroll = current.saturating_sub((-delta) as u16);
        } else {
            let next = current.saturating_add(delta as u16);
            window.scroll = if next >= window.last_scroll_max {
                u16::MAX
            } else {
                next
            };
        }
        true
    }
}

fn native_chat_lines(record: &NativeSubagentRecord, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("prompt  ", Style::default().fg(theme.dimmed)),
            Span::styled(record.prompt.clone(), Style::default().fg(theme.text)),
        ]),
        Line::default(),
    ];
    if record.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "waiting for the first reply…",
            Style::default().fg(theme.dimmed),
        )));
    } else {
        for message in &record.messages {
            lines.extend(
                crate::tui::markdown::render(&message.text)
                    .into_iter()
                    .enumerate()
                    .map(|(index, mut line)| {
                        line.spans.insert(
                            0,
                            Span::styled(
                                if index == 0 { "• " } else { "  " },
                                Style::default().fg(theme.hint),
                            ),
                        );
                        line
                    }),
            );
            lines.push(Line::default());
        }
    }
    lines
}

fn window_rect(
    viewport: Rect,
    position: Option<(u16, u16)>,
    size: Option<(u16, u16)>,
    index: usize,
) -> Rect {
    let (requested_width, requested_height) = size.unwrap_or((WINDOW_WIDTH, WINDOW_HEIGHT));
    let width = requested_width.max(MIN_WINDOW_WIDTH).min(viewport.width);
    let height = requested_height.max(MIN_WINDOW_HEIGHT).min(viewport.height);
    let cascade = u16::try_from(index).unwrap_or(u16::MAX).saturating_mul(2);
    let default_x = viewport
        .right()
        .saturating_sub(width)
        .saturating_sub(cascade);
    let default_y = viewport.y.saturating_add(1).saturating_add(cascade);
    let (x, y) = position.unwrap_or((default_x, default_y));
    Rect::new(
        x.clamp(viewport.x, viewport.right().saturating_sub(width)),
        y.clamp(viewport.y, viewport.bottom().saturating_sub(height)),
        width,
        height,
    )
}

fn wrapped_rows(lines: &[Line<'_>], width: u16) -> u16 {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum::<usize>()
        .min(usize::from(u16::MAX)) as u16
}
