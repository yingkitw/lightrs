use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use ratatui::layout::Rect;
use ratatui::text::Line;

/// (fingerprint, lines, tool hit-spans) — see `App::transcript_cache`.
pub type TranscriptCache = (u64, Vec<Line<'static>>, Vec<(usize, usize, usize)>);
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::config::{self, AppConfig};
use crate::skills::{SkillCatalog, load, load_settings};
use crate::tui::composer::Composer;
use crate::tui::theme;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    Ready,
    Busy,
}

pub struct CommandApproval {
    /// All `run_command` calls in the step needing approval — one prompt covers all.
    pub commands: Vec<String>,
    pub reply: tokio::sync::oneshot::Sender<bool>,
}

/// Model-initiated `ask_user` — the next Enter submits the answer, not a turn.
pub struct AskUserRequest {
    pub question: String,
    pub reply: tokio::sync::oneshot::Sender<String>,
}

pub enum AgentEvent {
    Done,
    Failed(String),
    Interrupted,
    /// Transcript changed while a turn is in progress (streaming).
    Refresh,
    /// Phase label for status bar (model wait, tool name, …).
    Progress(String),
    /// TUI must approve/deny before the agent continues this shell command.
    CommandApproval(CommandApproval),
    /// Model asked a question; next submitted line is the answer.
    AskUser(AskUserRequest),
    /// Background `/model` fetch finished: per-provider models or error text.
    ModelsListed(Vec<(crate::config::Provider, Result<Vec<String>, String>)>),
    /// User `!cmd` finished; body is the exit/stdout/stderr block.
    ShellDone(String),
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
    /// Rendered transcript cache — (fingerprint, lines, tool spans). The
    /// fingerprint covers width, expanded set, and every rendered field of
    /// each message; a hit skips the whole markdown/syntect rebuild.
    pub transcript_cache: Option<TranscriptCache>,
    /// `(first_line, last_line_exclusive, ui_message_index)` per tool block.
    pub tool_hit_spans: Vec<(usize, usize, usize)>,
    pub pending_approval: Option<CommandApproval>,
    /// Pending `ask_user` — Enter sends the reply, Esc declines.
    pub pending_ask: Option<AskUserRequest>,
    /// Detect `config.toml` changes for auto `[shell]` reload before each send.
    config_mtime: Option<SystemTime>,
    pub skill_catalog: SkillCatalog,
    /// `.claude/commands/*.md` project slash commands (rescanned on sends).
    pub commands: Vec<crate::commands::SlashDef>,
    pub theme: String,
    /// Live streamed assistant chars this turn (from UI transcript).
    pub stream_chars: usize,
    /// User lines submitted while `Busy`; drained when the turn ends.
    /// Busy-typed lines. Shared with the in-flight turn — the tool loop
    /// drains non-`/`/`!` lines as steering messages at each step boundary;
    /// `pump_queued` dispatches whatever remains (slash/bang, late arrivals)
    /// after the run ends.
    pub queued: Arc<std::sync::Mutex<VecDeque<String>>>,
    /// Git branch for the header path label; refreshed on `/cd`.
    pub git_branch: Option<String>,
    /// Tokio handle + event channel for slash commands that spawn background
    /// work (`/model` list fetch). Set once in `run()`; `None` in tests.
    pub rt: Option<tokio::runtime::Handle>,
    pub ev_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentEvent>>,
    /// Last `/model` fetch result in display order — numbered `/model <n>` picks.
    pub model_choices: Vec<(crate::config::Provider, String)>,
    /// Set after `/model` prints a numbered list — the next submitted bare
    /// number picks that model (one-shot; any other input clears it).
    pub pending_model_pick: bool,
}

impl App {
    pub fn new(config: AppConfig) -> Self {
        let git_branch = crate::tui::info::git_branch_label(&config.workdir);
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
            transcript_cache: None,
            tool_hit_spans: Vec::new(),
            pending_approval: None,
            pending_ask: None,
            config_mtime: None,
            skill_catalog: SkillCatalog::default(),
            commands: Vec::new(),
            theme: theme::normalize_theme_id(config::load_tui_theme()),
            stream_chars: 0,
            queued: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            rt: None,
            ev_tx: None,
            model_choices: Vec::new(),
            pending_model_pick: false,
            git_branch,
        }
    }

    pub fn refresh_skill_catalog(&mut self, agent: &mut Agent) {
        let settings = load_settings();
        let catalog = load(&settings);
        self.skill_catalog = catalog.clone();
        agent.set_skill_catalog(catalog);
        self.commands = crate::commands::load(self.workdir());
    }

    pub fn refresh_config_mtime(&mut self) {
        self.config_mtime = self.watched_config_mtime();
    }

    /// Latest mtime of global `config.toml` or project `.lightrs/config.toml`.
    fn watched_config_mtime(&self) -> Option<SystemTime> {
        [
            config::config_file_mtime(),
            config::project_config_mtime(self.workdir()),
        ]
        .into_iter()
        .flatten()
        .max()
    }

    pub fn maybe_reload_shell(&mut self, agent: &Arc<Mutex<Agent>>) {
        let Some(mtime) = self.watched_config_mtime() else {
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

    /// Empty answer = declined (Esc); the tool body says so.
    pub fn resolve_ask(&mut self, answer: String) {
        if let Some(req) = self.pending_ask.take() {
            let body = if answer.trim().is_empty() {
                "(user declined to answer)".to_string()
            } else {
                answer
            };
            let _ = req.reply.send(body);
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

    /// Animated ellipsis for status / transcript progress rows.
    pub fn busy_dots(&self) -> &'static str {
        const DOTS: &[&str] = &["", ".", "..", "..."];
        let tick = self
            .busy_since
            .map(|t| t.elapsed().as_millis() / 350)
            .unwrap_or(0) as usize;
        DOTS[tick % DOTS.len()]
    }

    pub fn busy_phase(&self) -> &str {
        if self.status.is_empty() {
            "thinking"
        } else {
            self.status.as_str()
        }
    }

    pub fn busy_elapsed(&self) -> Option<u64> {
        let s = self.busy_since?.elapsed().as_secs();
        if s >= 1 { Some(s) } else { None }
    }

    /// Status bar / header label while `Busy`.
    pub fn busy_label(&self) -> String {
        let mut label = format!(
            "{} {}{}",
            self.busy_spinner(),
            crate::tools::shell::escape_controls(self.busy_phase()),
            self.busy_dots()
        );
        if let Some(s) = self.busy_elapsed() {
            label.push_str(&format!(" · {s}s"));
        }
        if self.stream_chars > 0 {
            label.push_str(&format!(" · {} chars", self.stream_chars));
        }
        let queued_n = self.queued.lock().unwrap().len();
        if queued_n > 0 {
            label.push_str(&format!(" · queued {queued_n}"));
        }
        label
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }

    pub fn insert_text(&mut self, text: &str) {
        self.input.push_str(text);
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
        self.status = "starting…".into();
        self.stream_chars = 0;
        self.busy_since = Some(Instant::now());
        self.pin_bottom = true;
        self.scroll_to_bottom();
    }

    pub fn set_ready(&mut self, _config: &AppConfig) {
        self.state = RunState::Ready;
        self.status.clear();
        self.busy_since = None;
        self.stream_chars = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_app() -> App {
        App::new(AppConfig::test())
    }

    #[test]
    fn busy_label_includes_spinner_and_phase() {
        let mut app = minimal_app();
        app.set_busy();
        app.status = "streaming".into();
        let label = app.busy_label();
        assert!(label.contains('⠋') || label.contains('⠙'));
        assert!(label.contains("streaming"));
    }

    #[test]
    fn busy_label_shows_stream_char_count() {
        let mut app = minimal_app();
        app.set_busy();
        app.status = "streaming".into();
        app.stream_chars = 42;
        let label = app.busy_label();
        assert!(label.contains("42 chars"));
    }

    #[test]
    fn busy_elapsed_none_during_first_second() {
        let mut app = minimal_app();
        app.set_busy();
        assert_eq!(app.busy_elapsed(), None);
    }

    #[test]
    fn set_ready_clears_stream_chars() {
        let mut app = minimal_app();
        app.set_busy();
        app.stream_chars = 9;
        app.set_ready(&app.config.clone());
        assert_eq!(app.stream_chars, 0);
        assert_eq!(app.state, RunState::Ready);
    }
    #[test]
    fn resolve_ask_sends_answer_or_decline() {
        let mut app = App::new(crate::config::AppConfig {
                                                  provider: crate::config::Provider::Glm,
                                                  model: "m".into(),
                                                  base_url: "http://x".into(),
                                                  api_key: "k".into(),
                                                  ..AppConfig::test()
                                              });
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_ask = Some(AskUserRequest {
            question: "which file?".into(),
            reply: tx,
        });
        app.resolve_ask("a.rs".into());
        assert_eq!(rx.blocking_recv().unwrap(), "a.rs");
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_ask = Some(AskUserRequest {
            question: "q".into(),
            reply: tx,
        });
        app.resolve_ask("  ".into());
        assert_eq!(rx.blocking_recv().unwrap(), "(user declined to answer)");
    }
}