mod approval;
mod helpers;
mod prompt;
mod tokens;
mod turn;

use std::path::Path;
use std::sync::{Arc, Mutex};

pub use tokens::{TokenStats, TokenSummary, context_usage_text};

use crate::chat::{ApiMessage, ApiRole, TokenUsage, UiMessage, UiRole};
use crate::session::SessionFile;
use crate::provider::LlmClient;
use crate::tools::ShellPolicy;
use prompt::system_prompt;
use tokens::{
    estimate_context_tokens, estimate_text_tokens, format_token_count, step_usage_line,
};
use helpers::remove_trailing_empty_assistant;

pub enum ObserveEvent<'a> {
    AssistantDelta {
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

pub struct Agent {
    pub(crate) client: LlmClient,
    pub(crate) workdir: Arc<Path>,
    pub(crate) shell_policy: ShellPolicy,
    pub(crate) api_history: Vec<ApiMessage>,
    /// UI transcript; readable from the TUI thread while the worker streams LLM output.
    pub(crate) ui_history: Arc<Mutex<Vec<UiMessage>>>,
    pub(crate) token_stats: TokenStats,
    pub(crate) estimated_context_tokens: u32,
}

impl Agent {
    pub fn new(client: LlmClient, workdir: Arc<Path>, shell_policy: ShellPolicy) -> Self {
        let mut api_history = Vec::new();
        api_history.push(ApiMessage::system(system_prompt(&workdir, &shell_policy)));
        let mut agent = Self {
            client,
            workdir,
            shell_policy,
            api_history,
            ui_history: Arc::new(Mutex::new(Vec::new())),
            token_stats: TokenStats::default(),
            estimated_context_tokens: 0,
        };
        agent.refresh_context_estimate();
        agent
    }

    pub fn restore_session(&mut self, saved: SessionFile) {
        self.api_history = saved.api_history;
        self.ensure_system_prompt();
        *self.ui_history.lock().unwrap() = saved.ui_history;
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
        self.api_history.clone()
    }

    pub fn export_ui_history(&self) -> Vec<UiMessage> {
        self.ui_history.lock().unwrap().clone()
    }

    pub fn ui_transcript(&self) -> Arc<Mutex<Vec<UiMessage>>> {
        Arc::clone(&self.ui_history)
    }

    fn ensure_system_prompt(&mut self) {
        let prompt = ApiMessage::system(system_prompt(&self.workdir, &self.shell_policy));
        match self.api_history.first_mut() {
            Some(first) if matches!(first.role, ApiRole::System) => *first = prompt,
            _ => self.api_history.insert(0, prompt),
        }
    }

    pub fn token_summary(&self) -> TokenSummary {
        TokenSummary {
            stats: self.token_stats,
            estimate: self.estimated_context_tokens,
        }
    }

    pub(crate) fn refresh_context_estimate(&mut self) {
        self.estimated_context_tokens = estimate_context_tokens(&self.api_history);
    }

    pub(crate) fn record_send_estimate(&mut self, api_text: &str) {
        let est = estimate_text_tokens(api_text);
        self.push_system(format!(
            "send · ~{} ctx",
            format_token_count(u64::from(est))
        ));
    }

    pub(crate) fn record_model_usage(&mut self, usage: Option<&TokenUsage>) {
        if let Some(line) = step_usage_line(usage) {
            self.push_system(line);
        }
    }

    pub fn ui_messages(&self) -> Vec<UiMessage> {
        self.ui_history.lock().unwrap().clone()
    }

    pub fn api_context_len(&self) -> usize {
        self.api_history.len()
    }

    pub fn push_system(&mut self, content: impl Into<String>) {
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::System, content));
    }

    pub fn push_error(&mut self, content: impl Into<String>) {
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::Error, content));
    }

    pub fn finish_interrupt(&mut self) {
        remove_trailing_empty_assistant(&self.ui_history);
        self.push_system("Turn interrupted.");
    }

    pub fn set_shell_policy(&mut self, policy: ShellPolicy) {
        self.shell_policy = policy;
        self.ensure_system_prompt();
    }

    pub fn clear_conversation(&mut self) {
        self.api_history.clear();
        self.api_history
            .push(ApiMessage::system(system_prompt(&self.workdir, &self.shell_policy)));
        let mut ui = self.ui_history.lock().unwrap();
        ui.clear();
        ui.push(UiMessage::with_role(
            UiRole::System,
            "Conversation cleared.",
        ));
        drop(ui);
        self.token_stats = TokenStats::default();
        self.refresh_context_estimate();
    }
}