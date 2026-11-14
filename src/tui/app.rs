use std::path::PathBuf;

use crate::config::AppConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    Ready,
    Busy,
}

pub enum AgentEvent {
    Done,
    Failed(String),
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
        }
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
        self.status = "waiting for model…".into();
        self.pin_bottom = true;
        self.scroll_to_bottom();
    }

    pub fn set_ready(&mut self, _config: &AppConfig) {
        self.state = RunState::Ready;
        self.status.clear();
    }

    pub fn set_error_status(&mut self, msg: String) {
        self.state = RunState::Ready;
        self.status = msg;
    }

    pub fn workdir(&self) -> &PathBuf {
        &self.config.workdir
    }
}