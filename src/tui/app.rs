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
    pub scroll: u16,
    pub state: RunState,
    pub status: String,
}

impl App {
    pub fn new(config: AppConfig) -> Self {
        Self {
            status: format!(
                "{} · {} · {}",
                format!("{:?}", config.provider).to_lowercase(),
                config.model,
                config.workdir.display()
            ),
            config,
            input: String::new(),
            scroll: 0,
            state: RunState::Ready,
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
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn set_busy(&mut self) {
        self.state = RunState::Busy;
        self.status = "thinking…".into();
    }

    pub fn set_ready(&mut self, config: &AppConfig) {
        self.state = RunState::Ready;
        self.status = format!(
            "{} · {} · {}",
            format!("{:?}", config.provider).to_lowercase(),
            config.model,
            config.workdir.display()
        );
    }

    pub fn set_error_status(&mut self, msg: String) {
        self.state = RunState::Ready;
        self.status = msg;
    }

    pub fn workdir(&self) -> &PathBuf {
        &self.config.workdir
    }
}