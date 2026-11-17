mod approval;
mod context;
mod helpers;
mod prompt;
mod turn;
mod workdir_context;

use std::path::Path;
use std::sync::{Arc, Mutex};

pub use context::tokens::{
    ContextBreakdown, TokenStats, TokenSummary, context_compact_text, context_usage_text,
};

use crate::chat::{ApiMessage, ApiRole, UiMessage, UiRole};
use crate::session::SessionFile;
use crate::provider::{LlmClient, ModelClient};
use crate::skills::SkillCatalog;
use crate::tools::ShellPolicy;
use prompt::system_prompt;
use context::tokens::turn_usage_line;
use helpers::remove_trailing_empty_assistant;

pub enum ObserveEvent<'a> {
    /// HTTP/stream not yet delivering tokens (user-visible wait).
    ModelWait,
    AssistantDelta {
        text: &'a str,
    },
    /// Reasoning/thinking tokens — routed to the `[think]` UI row and to ACP
    /// `AgentThoughtChunk`; never part of the assistant message.
    ThinkingDelta {
        text: &'a str,
    },
    ToolStart {
        name: &'a str,
        arguments: &'a str,
    },
    ToolEnd {
        body: &'a str,
    },
}

/// Post-edit verification: run `command` after write/edit tools; failures go back to the model.
#[derive(Clone, Debug)]
pub struct HealSetting {
    pub command: String,
    pub max_steps: u32,
}

pub struct Agent {
    pub(crate) client: Arc<dyn ModelClient>,
    /// Optional secondary model client retried once when the primary errors mid-turn.
    pub(crate) fallback_client: Option<Arc<dyn ModelClient>>,
    /// Cheaper model name for read-only steps (`weak_model`); escalates to
    /// `client.model()` once a step calls a write/exec tool.
    pub(crate) weak_model: Option<String>,
    /// Whether the current turn is still running on `weak_model`.
    pub(crate) weak_active: bool,
    /// In-flight tool's transcript row + start time — replaced in place by the
    /// result so slow tools are visible while running.
    pub(crate) pending_tool: Option<(usize, std::time::Instant)>,
    /// Turn wall-clock start, for the `turn · … · Ns` close line.
    pub(crate) turn_started: Option<std::time::Instant>,
    pub(crate) workdir: Arc<Path>,
    pub(crate) shell_policy: ShellPolicy,
    pub(crate) api_history: Vec<ApiMessage>,
    /// UI transcript; readable from the TUI thread while the worker streams LLM output.
    pub(crate) ui_history: Arc<Mutex<Vec<UiMessage>>>,
    pub(crate) token_stats: TokenStats,
    pub(crate) estimated_context_tokens: u32,
    /// When true, skip `send · ~N ctx` and per-step model usage lines in the UI transcript.
    pub(crate) quiet_token_sys: bool,
    pub(crate) skill_catalog: SkillCatalog,
    /// Last `estimated_context_tokens` at which a context-pressure `[sys]` was shown.
    pub(crate) context_warned_above: u32,
    /// `@` / `$` expansion for the in-flight user turn only (not persisted in `api_history`).
    pub(crate) active_user_expansion: Option<String>,
    /// `$skill` ids attached for the current/latest turn — surfaced in the
    /// status bar and a `[sys]` transcript note.
    pub(crate) turn_skills: Vec<String>,
    /// Files written by write/edit tools this session (diff paths, sorted).
    pub(crate) changed_files: std::collections::BTreeSet<String>,
    pub(crate) heal: Option<HealSetting>,
    /// Formatter run after each write/edit tool (`[shell].format_command`);
    /// `{path}` is replaced with the diff path, shell-quoted otherwise.
    pub(crate) format_cmd: Option<String>,
    /// Plan/read-only mode — write/exec tools denied at the tool gate.
    pub(crate) read_only: bool,
    /// Sub-agent depth — `spawn_agent` children get depth+1; depth ≥1 cannot spawn.
    pub(crate) depth: u8,
    /// Lazy MCP servers (`[mcp.<name>]`); behind its own mutex so the agent
    /// lock is never held across a server RPC.
    pub(crate) mcp: Option<Arc<tokio::sync::Mutex<crate::mcp::McpManager>>>,
    /// Lazy LSP servers (`[lsp.<lang>]`); same lock-isolation rule.
    pub(crate) lsp: Option<Arc<tokio::sync::Mutex<crate::lsp::LspManager>>>,
    /// Configured MCP server names — advertised in the system prompt so the
    /// model knows valid `mcp_call.server` values.
    pub(crate) mcp_names: Vec<String>,
    /// Heal runs so far this user turn (reset in `begin_user_turn`).
    pub(crate) heal_ran: u32,
    /// One-shot `[sys]` note once the wire window first sends fewer turns than history holds.
    pub(crate) wire_window_noted: bool,
}

impl Agent {
    /// Sub-agent constructor — shares the parent's client `Arc` (clients are
    /// stateless per request) and records depth so grandchildren are denied.
    pub(crate) fn new_shared(
        client: Arc<dyn ModelClient>,
        workdir: Arc<Path>,
        shell_policy: ShellPolicy,
        depth: u8,
    ) -> Self {
        let mut agent = Self::new_inner(client, workdir, shell_policy);
        agent.depth = depth;
        agent
    }

    pub fn new(client: impl ModelClient + 'static, workdir: Arc<Path>, shell_policy: ShellPolicy) -> Self {
        Self::new_inner(Arc::new(client), workdir, shell_policy)
    }

    fn new_inner(client: Arc<dyn ModelClient>, workdir: Arc<Path>, shell_policy: ShellPolicy) -> Self {
        let mut api_history = Vec::new();
        let skill_catalog = SkillCatalog::default();
        api_history.push(ApiMessage::system(system_prompt(
            &workdir,
            &shell_policy,
            &skill_catalog,
            false,
            &[],
        )));
        let mut agent = Self {
            client,
            fallback_client: None,
            weak_model: None,
            weak_active: false,
            pending_tool: None,
            turn_started: None,
            workdir,
            shell_policy,
            api_history,
            ui_history: Arc::new(Mutex::new(Vec::new())),
            token_stats: TokenStats::default(),
            estimated_context_tokens: 0,
            quiet_token_sys: false,
            skill_catalog,
            context_warned_above: 0,
            active_user_expansion: None,
            turn_skills: Vec::new(),
            changed_files: std::collections::BTreeSet::new(),
            heal: None,
            format_cmd: None,
            read_only: false,
            depth: 0,
            mcp: None,
            lsp: None,
            mcp_names: Vec::new(),
            heal_ran: 0,
            wire_window_noted: false,
        };
        agent.refresh_context_estimate();
        agent
    }

    /// Build from config: primary `LlmClient` + fallback client + weak-model
    /// routing. Callers add session restore, heal, and skills as needed
    /// (TUI `run`, ACP `make_agent`).
    pub fn from_config(cfg: &crate::config::AppConfig, workdir: Arc<Path>) -> Self {
        let mut agent = Self::new(
            LlmClient::new(cfg.base_url.clone(), cfg.api_key.clone(), cfg.model.clone()),
            workdir,
            cfg.shell_policy.clone(),
        );
        if let Some(fm) = &cfg.fallback_model {
            agent.set_fallback_client(Some(LlmClient::new(
                cfg.base_url.clone(),
                cfg.api_key.clone(),
                fm.clone(),
            )));
        }
        agent.set_weak_model(cfg.weak_model.clone());
        agent.set_format_command(cfg.format_command.clone());
        if cfg.read_only {
            agent.set_read_only(true);
        }
        if !cfg.mcp.is_empty() {
            agent.mcp_names = cfg.mcp.keys().cloned().collect();
            agent.mcp = Some(Arc::new(tokio::sync::Mutex::new(crate::mcp::McpManager::new(
                cfg.mcp.clone(),
                cfg.shell_policy.sandbox().clone(),
                cfg.workdir.clone(),
            ))));
            agent.ensure_system_prompt();
        }
        if !cfg.lsp.is_empty() {
            agent.lsp = Some(Arc::new(tokio::sync::Mutex::new(crate::lsp::LspManager::new(
                cfg.lsp.clone(),
                cfg.workdir.clone(),
                cfg.shell_policy.sandbox().clone(),
            ))));
        }
        agent
    }

    pub(crate) fn messages_for_request(&self) -> Vec<ApiMessage> {
        context::messages_for_api(&self.api_history, self.active_user_expansion.as_deref())
    }

    pub(crate) fn finish_user_turn_api(&mut self) {
        self.active_user_expansion = None;
        // Turn ended mid-tool (interrupt/error): unstick any `◌ running` row.
        if let Some((idx, started)) = self.pending_tool.take() {
            let mut ui = self.ui_history.lock().unwrap();
            if let Some(m) = ui.get_mut(idx)
                && m.pending
            {
                m.pending = false;
                m.elapsed_ms = Some(started.elapsed().as_millis() as u64);
                m.content = "(interrupted)".into();
            }
        }
        helpers::trim_ui_history(&self.ui_history);
        if !self.quiet_token_sys
            && let Some(line) = turn_usage_line(&self.token_stats)
        {
            let elapsed = self
                .turn_started
                .take()
                .map(|t| format!(" · {}", crate::chat::fmt_elapsed(t.elapsed().as_millis() as u64)))
                .unwrap_or_default();
            self.push_system(format!("{line}{elapsed}"));
        } else {
            self.turn_started = None;
        }
    }


    pub fn restore_session(&mut self, saved: SessionFile) {
        self.active_user_expansion = None;
        let mut api_history = saved.api_history;
        context::slim_api_history_for_storage(&mut api_history);
        self.api_history = api_history;
        self.ensure_system_prompt();
        *self.ui_history.lock().unwrap() =
            crate::session::conversation_ui_history(saved.ui_history);
        // A row pending at save-time (crash/quit mid-tool) must not restore as
        // `◌ running` — mark it finished-but-interrupted.
        for m in self.ui_history.lock().unwrap().iter_mut() {
            if m.pending {
                m.pending = false;
                m.content = "(interrupted)".into();
            }
        }
        helpers::trim_ui_history(&self.ui_history);
        // Rebuild the changed-files indicator from persisted diff rows.
        self.changed_files = self
            .ui_history
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| m.file_diff.as_ref().map(|d| d.path.clone()))
            .collect();
        self.token_stats = saved.token_stats;
        self.refresh_context_estimate();
    }

    pub fn is_persistable(&self) -> bool {
        self.api_history.len() > 1
            || self
                .ui_history
                .lock()
                .unwrap()
                .iter()
                .any(|m| m.role == UiRole::User)
    }

    pub fn export_api_history(&self) -> Vec<ApiMessage> {
        let mut h = self.api_history.clone();
        context::slim_api_history_for_storage(&mut h);
        h
    }

    pub fn export_ui_history(&self) -> Vec<UiMessage> {
        self.ui_history.lock().unwrap().clone()
    }

    pub fn ui_transcript(&self) -> Arc<Mutex<Vec<UiMessage>>> {
        Arc::clone(&self.ui_history)
    }

    fn ensure_system_prompt(&mut self) {
        let prompt = ApiMessage::system(system_prompt(
            &self.workdir,
            &self.shell_policy,
            &self.skill_catalog,
            self.read_only,
            &self.mcp_names,
        ));
        match self.api_history.first_mut() {
            Some(first) if matches!(first.role, ApiRole::System) => *first = prompt,
            _ => self.api_history.insert(0, prompt),
        }
    }

    pub fn ui_messages(&self) -> Vec<UiMessage> {
        self.ui_history.lock().unwrap().clone()
    }

    pub fn streaming_assistant_chars(&self) -> usize {
        use crate::chat::UiRole;
        self.ui_history
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m.role == UiRole::Assistant)
            .map(|m| m.content.chars().count())
            .unwrap_or(0)
    }

    pub fn last_assistant_text(&self) -> Option<String> {
        use crate::chat::UiRole;
        self.ui_history
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m.role == UiRole::Assistant && !m.content.trim().is_empty())
            .map(|m| m.content.clone())
    }

    pub fn push_system(&mut self, content: impl Into<String>) {
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::System, content));
    }

    /// `$skill` ids attached for the current/latest user turn.
    pub fn turn_skills(&self) -> &[String] {
        &self.turn_skills
    }

    /// Sorted paths written by write/edit tools this session.
    pub fn changed_files(&self) -> impl Iterator<Item = &String> {
        self.changed_files.iter()
    }

    pub fn push_error(&mut self, content: impl Into<String>) {
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::Error, content));
    }

    /// Append a fully-formed UI message (e.g. `/diff` replay rows).
    pub fn push_ui_message(&mut self, msg: UiMessage) {
        self.ui_history.lock().unwrap().push(msg);
    }

    pub fn finish_interrupt(&mut self) {
        remove_trailing_empty_assistant(&self.ui_history);
        self.finish_user_turn_api();
        self.push_system("Turn interrupted.");
    }

    /// `/cd` — switch workdir mid-session; rebuilds the system prompt for the new root.
    pub fn set_workdir(&mut self, workdir: Arc<Path>) {
        self.workdir = workdir;
        // `/cd` is a new root — a stale session cwd would leak the old tree.
        self.shell_policy.reset_session_cwd();
        self.ensure_system_prompt();
    }

    /// `/model` — swap the primary client mid-session; fallback/weak unchanged.
    /// The header and context % read `client.model()` at render, so no other
    /// state needs updating.
    pub fn set_client(&mut self, client: impl ModelClient + 'static) {
        self.client = Arc::new(client);
        self.refresh_context_estimate();
    }

    pub fn set_fallback_client(&mut self, client: Option<impl ModelClient + 'static>) {
        self.fallback_client = client.map(|c| Arc::new(c) as Arc<dyn ModelClient>);
    }

    /// Model name used for read-only steps until a write/exec tool appears
    /// (`None` disables routing — every step uses the primary model).
    pub fn set_weak_model(&mut self, model: Option<String>) {
        self.weak_model = model;
        self.weak_active = false;
    }

    pub fn set_heal(&mut self, heal: Option<HealSetting>) {
        self.heal = heal;
    }

    pub fn set_format_command(&mut self, cmd: Option<String>) {
        self.format_cmd = cmd;
    }

    /// Plan/read-only mode (`/plan`, `--read-only`): write/exec tools are
    /// denied at the tool gate, and the rebuilt system prompt tells the
    /// model to explore and propose instead of discovering per-call.
    pub fn set_read_only(&mut self, on: bool) {
        if self.read_only == on {
            return;
        }
        self.read_only = on;
        self.ensure_system_prompt();
        self.push_system(if on {
            "Plan mode ON — file edits and shell commands are disabled."
        } else {
            "Plan mode OFF — file edits and shell commands are allowed again."
        });
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn set_shell_policy(&mut self, policy: ShellPolicy) {
        self.shell_policy = policy;
        self.ensure_system_prompt();
    }

    pub fn set_skill_catalog(&mut self, catalog: SkillCatalog) {
        self.skill_catalog = catalog;
        self.ensure_system_prompt();
    }

    pub fn skill_catalog(&self) -> &SkillCatalog {
        &self.skill_catalog
    }

    /// `/rewind n` — truncate API + UI history back n complete user turns.
    /// Truncates at a `User` boundary so tool_call pairs are never split.
    /// Returns the number of turns actually dropped.
    pub fn rewind_turns(&mut self, n: usize) -> usize {
        let n = n.max(1);
        let cut = nth_last_user_index(&self.api_history, n);
        let Some(api_cut) = cut else {
            return 0;
        };
        // Count turns being removed for the report.
        let dropped = self.api_history[api_cut..]
            .iter()
            .filter(|m| m.role == crate::chat::ApiRole::User)
            .count()
            .max(1);
        self.api_history.truncate(api_cut);
        self.active_user_expansion = None;
        let mut ui = self.ui_history.lock().unwrap();
        if let Some(ui_cut) = nth_last_ui_user_index(&ui, n) {
            ui.truncate(ui_cut);
        }
        drop(ui);
        self.context_warned_above = 0;
        self.refresh_context_estimate();
        dropped
    }

    pub fn clear_conversation(&mut self) {
        self.active_user_expansion = None;
        self.api_history.clear();
        self.api_history.push(ApiMessage::system(system_prompt(
            &self.workdir,
            &self.shell_policy,
            &self.skill_catalog,
            self.read_only,
            &self.mcp_names,
        )));
        let mut ui = self.ui_history.lock().unwrap();
        ui.clear();
        ui.push(UiMessage::with_role(
            UiRole::System,
            "Conversation cleared.",
        ));
        drop(ui);
        self.token_stats = TokenStats::default();
        self.turn_skills.clear();
        self.changed_files.clear();
        self.context_warned_above = 0;
        self.wire_window_noted = false;
        self.refresh_context_estimate();
    }
}

/// Index where the `n`th-from-last `User` message starts; clamps to the first
/// user turn when fewer than `n` exist. `None` when no user messages at all.
fn nth_last_user_index(history: &[ApiMessage], n: usize) -> Option<usize> {
    let mut seen = 0usize;
    for (i, m) in history.iter().enumerate().rev() {
        if m.role == crate::chat::ApiRole::User {
            seen += 1;
            if seen == n {
                return Some(i);
            }
        }
    }
    history
        .iter()
        .position(|m| m.role == crate::chat::ApiRole::User)
}

fn nth_last_ui_user_index(ui: &[UiMessage], n: usize) -> Option<usize> {
    let mut seen = 0usize;
    for (i, m) in ui.iter().enumerate().rev() {
        if m.role == UiRole::User {
            seen += 1;
            if seen == n {
                return Some(i);
            }
        }
    }
    ui.iter().position(|m| m.role == UiRole::User)
}

#[cfg(test)]
mod streaming_tests {
    use std::path::Path;
    use std::sync::Arc;

    use crate::agent::context::tokens::estimate_context_tokens;
    use crate::chat::{ApiMessage, ApiRole, FunctionCall, ToolCall, UiMessage, UiRole};
    use crate::provider::LlmClient;
    use crate::tools::ShellPolicy;

    use super::Agent;

    #[test]
    fn from_config_wires_fallback_and_weak() {
        let cfg = crate::config::AppConfig {
            provider: crate::config::Provider::Kimi,
            model: "main".into(),
            fallback_model: Some("fb".into()),
            weak_model: Some("weak".into()),
            base_url: "http://x".into(),
            api_key: "k".into(),
            workdir: std::path::PathBuf::from("."),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            format_command: None,
            read_only: false,
            heal_steps: 3,
            endpoints: Default::default(),
            mcp: Default::default(),
            lsp: Default::default(),
        };
        let agent = Agent::from_config(&cfg, Arc::from(Path::new(".")));
        assert_eq!(agent.client.model(), "main");
        assert!(agent.fallback_client.is_some());
        assert_eq!(agent.fallback_client.as_ref().unwrap().model(), "fb");
        assert_eq!(agent.weak_model.as_deref(), Some("weak"));

        let mut slim = cfg.clone();
        slim.fallback_model = None;
        slim.weak_model = None;
        let agent = Agent::from_config(&slim, Arc::from(Path::new(".")));
        assert!(agent.fallback_client.is_none() && agent.weak_model.is_none());
    }

    #[test]
    fn streaming_assistant_chars_counts_unicode_scalars() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());
        assert_eq!(agent.streaming_assistant_chars(), 0);

        agent
            .ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::User, "hi"));
        assert_eq!(agent.streaming_assistant_chars(), 0);

        agent
            .ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::Assistant, "αβ"));
        assert_eq!(agent.streaming_assistant_chars(), 2);
    }

    #[test]
    fn streaming_assistant_chars_uses_last_assistant_only() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());
        let ui = agent.ui_history.lock().unwrap();
        drop(ui);
        {
            let mut ui = agent.ui_history.lock().unwrap();
            ui.push(UiMessage::with_role(UiRole::Assistant, "old"));
            ui.push(UiMessage::with_role(UiRole::Assistant, "new"));
        }
        assert_eq!(agent.streaming_assistant_chars(), 3);
    }

    #[test]
    fn auto_compact_prunes_oversized_history() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let mut agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());

        let big_call = || ApiMessage {
            role: ApiRole::Assistant,
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "c".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "write_file".into(),
                    arguments: "x".repeat(60_000),
                },
            }]),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        };
        for i in 0..8 {
            agent.api_history.push(ApiMessage::user(format!("u{i}")));
            agent.api_history.push(big_call());
            agent.api_history.push(ApiMessage::tool_result(
                format!("t{i}"),
                "write_file",
                "b".repeat(6_000),
            ));
        }
        // Raw stored history (not the bounded wire estimate) must exceed the cap.
        let before = estimate_context_tokens(&agent.api_history);
        assert!(before > 64_000, "fixture should exceed the hard cap");
        let len_before = agent.api_history.len();

        agent.auto_compact_context();

        assert!(estimate_context_tokens(&agent.api_history) < before);
        assert!(agent.api_history.len() < len_before);
        // The in-flight (most recent) user turn is never dropped.
        let last_user = agent
            .api_history
            .iter()
            .rposition(|m| m.role == ApiRole::User)
            .unwrap();
        assert_eq!(agent.api_history[last_user].text_content().unwrap(), "u7");
        // And the history fits under the hard cap.
        assert!(estimate_context_tokens(&agent.api_history) <= 64_000);
    }

    #[test]
    fn rewind_drops_turns_at_user_boundary() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let mut agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());
        for i in 0..3 {
            agent.api_history.push(ApiMessage::user(format!("u{i}")));
            agent
                .api_history
                .push(ApiMessage::tool_result(format!("t{i}"), "read_file", "b"));
        }
        {
            let mut ui = agent.ui_history.lock().unwrap();
            ui.push(UiMessage::with_role(UiRole::User, "u2"));
            ui.push(UiMessage::with_role(UiRole::Assistant, "a2"));
        }

        assert_eq!(agent.rewind_turns(1), 1);
        // sys + u0/t0 + u1/t1 remain; the u2 turn and its tools are gone.
        assert_eq!(agent.api_history.len(), 5);
        assert_eq!(
            agent.api_history.last().unwrap().name.as_deref(),
            Some("read_file")
        );
        assert!(
            agent
                .ui_history
                .lock()
                .unwrap()
                .iter()
                .all(|m| m.role != UiRole::User)
        );
    }

    #[test]
    fn restore_session_rebuilds_changed_files() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let mut agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());
        let mut diff_row = UiMessage::tool_result("write_file", "{}", "wrote", None);
        diff_row.file_diff = Some(crate::chat::FileDiffUi {
            path: "src/x.rs".into(),
            unified: String::new(),
            lines_added: 1,
            lines_removed: 0,
            is_new_file: false,
        });
        let saved = crate::session::SessionFile {
            version: 1,
            workdir: std::path::PathBuf::from("."),
            api_history: vec![ApiMessage::user("u")],
            ui_history: vec![
                UiMessage::with_role(UiRole::User, "u"),
                diff_row,
            ],
            token_stats: Default::default(),
        };
        agent.restore_session(saved);
        let files: Vec<&String> = agent.changed_files().collect();
        assert_eq!(files, ["src/x.rs"]);
    }

    #[test]
    fn rewind_more_than_available_drops_all_turns() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let mut agent = Agent::new(client, Arc::from(Path::new(".")), ShellPolicy::default());
        for i in 0..2 {
            agent.api_history.push(ApiMessage::user(format!("u{i}")));
            agent
                .api_history
                .push(ApiMessage::tool_result(format!("t{i}"), "read_file", "b"));
        }
        assert_eq!(agent.rewind_turns(5), 2);
        assert_eq!(agent.api_history.len(), 1); // system prompt only
        assert_eq!(agent.rewind_turns(1), 0); // nothing left to rewind
    }
}