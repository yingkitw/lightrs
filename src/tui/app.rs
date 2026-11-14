use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use ratatui::layout::Rect;
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::config::{self, AppConfig};
use crate::tui::composer::Composer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    Ready,
    Busy,
}

pub struct CommandApproval {
    pub command: String,
    pub reply: tokio::sync::oneshot::Sender<bool>,
}

pub enum AgentEvent {
    Done,
    Failed(String),
    Interrupted,
    /// Transcript changed while a turn is in progress (streaming).
    Refresh,
    /// TUI must approve/deny before the agent continues this shell command.
    CommandApproval(CommandApproval),
}

pub struct App {
    pub config: AppConfig,
    pub input: String,
    /// Lines scrolled up from the bottom of the transcript (0 = follow latest).
    pub scroll: u16,
    /// Last rendered transcript height in rows (for page up/down).
    pub transcript_viewport: u16,
    pub state: RunState,
    pub status: String,
    /// When true, new output keeps the viewport pinned to the latest messages.
    pub pin_bottom: bool,
    pub composer: Composer,
    pub busy_since: Option<Instant>,
    /// Indices in the UI transcript with expanded tool-call detail.
    pub expanded_tools: HashSet<usize>,
    /// Tool message index closest to the bottom of the visible transcript (for `o`).
    pub focus_tool_msg_idx: Option<usize>,
    /// Last rendered transcript widget area (for mouse hit-testing).
    pub transcript_rect: Rect,
    /// First visible line index in the full transcript layout.
    pub transcript_visible_start: usize,
    /// `(first_line, last_line_exclusive, ui_message_index)` per tool block.
    pub tool_hit_spans: Vec<(usize, usize, usize)>,
    pub pending_approval: Option<CommandApproval>,
    /// Detect `config.toml` changes for auto `[shell]` reload before each send.
    config_mtime: Option<SystemTime>,
}

impl App {
    pub fn new(config: AppConfig) -> Self {
        Self {
            status: String::new(),
            config,
            input: String::new(),
            scroll: 0,
            transcript_viewport: 1,
            state: RunState::Ready,
            pin_bottom: true,
            composer: Composer::default(),
            busy_since: None,
            expanded_tools: HashSet::new(),
            focus_tool_msg_idx: None,
            transcript_rect: Rect::default(),
            transcript_visible_start: 0,
            tool_hit_spans: Vec::new(),
            pending_approval: None,
            config_mtime: None,
        }
    }

    pub fn refresh_config_mtime(&mut self) {
        self.config_mtime = config::config_file_mtime();
    }

    pub fn maybe_reload_shell(&mut self, agent: &Arc<Mutex<Agent>>) {
        let Some(mtime) = config::config_file_mtime() else {
            return;
        };
        if self.config_mtime == Some(mtime) {
            return;
        }
        self.config_mtime = Some(mtime);
        let mut guard = agent.blocking_lock();
        match self.config.reload_shell_from_file() {
            Ok(msg) => {
                guard.set_shell_policy(self.config.shell_policy.clone());
                guard.push_system(format!("{msg} (auto on send)"));
            }
            Err(e) => guard.push_error(format!("config reload: {e:#}")),
        }
    }

    pub fn resolve_approval(&mut self, approved: bool) {
        if let Some(req) = self.pending_approval.take() {
            let _ = req.reply.send(approved);
        }
    }

    /// Map a terminal mouse position to a tool-call message index when clicked inside the transcript.
    pub fn tool_msg_at_transcript_click(&self, x: u16, y: u16) -> Option<usize> {
        let area = self.transcript_rect;
        if area.width < 3 || area.height < 3 {
            return None;
        }
        if x < area.x || y < area.y || x >= area.x + area.width || y >= area.y + area.height {
            return None;
        }
        let inner_y = y.saturating_sub(area.y + 1);
        let view_h = usize::from(self.transcript_viewport);
        if usize::from(inner_y) >= view_h {
            return None;
        }
        let abs_line = self.transcript_visible_start + usize::from(inner_y);
        tool_msg_index_at_line(abs_line, &self.tool_hit_spans)
    }

    pub fn toggle_tool_expand(&mut self, msg_idx: usize) {
        if self.expanded_tools.contains(&msg_idx) {
            self.expanded_tools.remove(&msg_idx);
        } else {
            self.expanded_tools.insert(msg_idx);
        }
    }

    pub fn clear_tool_expansion(&mut self) {
        self.expanded_tools.clear();
    }

    pub fn busy_spinner(&self) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let tick = self
            .busy_since
            .map(|t| t.elapsed().as_millis() / 80)
            .unwrap_or(0) as usize;
        FRAMES[tick % FRAMES.len()]
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    pub fn take_input(&mut self) -> String {
        std::mem::take(&mut self.input)
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_add(n);
        if n > 0 {
            self.pin_bottom = false;
        }
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
        if self.scroll == 0 {
            self.pin_bottom = true;
        }
    }

    pub fn scroll_page_up(&mut self) {
        let step = self.transcript_viewport.saturating_sub(1).max(1);
        self.scroll_up(step);
    }

    pub fn scroll_page_down(&mut self) {
        let step = self.transcript_viewport.saturating_sub(1).max(1);
        self.scroll_down(step);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
        self.pin_bottom = true;
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = u16::MAX;
        self.pin_bottom = false;
    }

    pub fn clamp_scroll(&mut self, max_scroll: u16) {
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    pub fn set_busy(&mut self) {
        self.state = RunState::Busy;
        self.status.clear();
        self.busy_since = Some(Instant::now());
        self.pin_bottom = true;
        self.scroll_to_bottom();
    }

    pub fn set_ready(&mut self, _config: &AppConfig) {
        self.state = RunState::Ready;
        self.status.clear();
        self.busy_since = None;
    }

    pub fn set_error_status(&mut self, msg: String) {
        self.state = RunState::Ready;
        self.status = msg;
    }

    pub fn workdir(&self) -> &PathBuf {
        &self.config.workdir
    }
}

pub fn tool_msg_index_at_line(
    abs_line: usize,
    spans: &[(usize, usize, usize)],
) -> Option<usize> {
    spans
        .iter()
        .find(|(start, end, _)| abs_line >= *start && abs_line < *end)
        .map(|(_, _, idx)| *idx)
}