use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use crate::config::AppConfig;
use crate::tui::composer::Composer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    Ready,
    Busy,
}

pub enum AgentEvent {
    Done,
    Failed(String),
    Interrupted,
    /// Transcript changed while a turn is in progress (streaming).
    Refresh,
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
        }
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