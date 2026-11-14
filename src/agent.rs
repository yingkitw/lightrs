mod tokens;

use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use crate::tui::{AgentEvent, CommandApproval};

pub use tokens::{TokenStats, TokenSummary, context_usage_text};

use crate::chat::{ApiMessage, ApiRole, ChatRequest, UiMessage, UiRole};
use crate::session::SessionFile;
use tokens::{apply_usage, estimate_context_tokens};
use crate::provider::LlmClient;
use crate::tools::{RunCommandArgs, ShellPolicy, execute_tool, tool_schemas};

const MAX_TOOL_STEPS: usize = 16;

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

fn system_prompt(workdir: &Path, shell: &ShellPolicy) -> String {
    let mut text = format!(
        r#"You are lightrs, a minimal terminal coding agent.
Workdir: {}.

Use tools to read or change the codebase and to run commands — do not invent file contents.
Prefer small, focused edits. Explain briefly after completing the task.
If a tool returns an error, fix the issue or ask the user for missing info."#,
        workdir.display()
    );
    if shell.is_restrictive() {
        text.push_str(
            "\nShell allowlist is enabled: only commands matching configured prefixes can run.",
        );
    }
    if shell.requires_all_commands_approved() {
        text.push_str("\nThe user must approve each shell command in the TUI before it runs.");
    }
    text
}

pub struct Agent {
    client: LlmClient,
    workdir: Arc<Path>,
    shell_policy: ShellPolicy,
    api_history: Vec<ApiMessage>,
    /// UI transcript; readable from the TUI thread while the worker streams LLM output.
    ui_history: Arc<Mutex<Vec<UiMessage>>>,
    token_stats: TokenStats,
    estimated_context_tokens: u32,
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

    /// True if there is conversation worth saving to disk.
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

    fn refresh_context_estimate(&mut self) {
        self.estimated_context_tokens = estimate_context_tokens(&self.api_history);
    }

    pub fn ui_messages(&self) -> Vec<UiMessage> {
        self.ui_history.lock().unwrap().clone()
    }

    /// Number of messages in the LLM API history (system + turns + tool results).
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

    pub fn clear_conversation(&mut self) {
        self.api_history.clear();
        self.api_history
            .push(ApiMessage::system(system_prompt(
                &self.workdir,
                &self.shell_policy,
            )));
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

    pub async fn run_user_turn(&mut self, user_text: String) -> Result<String> {
        self.run_user_turn_observed(user_text, |_| {}).await
    }

    pub async fn run_user_turn_observed(
        &mut self,
        user_text: String,
        mut on: impl FnMut(ObserveEvent<'_>) + Send,
    ) -> Result<String> {
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::User, user_text.clone()));
        self.api_history.push(ApiMessage::user(user_text));
        self.refresh_context_estimate();

        for _step in 0..MAX_TOOL_STEPS {
            let request = ChatRequest {
                model: self.client.model().to_string(),
                messages: self.api_history.clone(),
                tools: Some(tool_schemas()),
                tool_choice: Some(serde_json::json!("auto")),
                stream: None,
            };
            let client = self.client.clone();
            let ui_history = self.ui_transcript();
            let workdir = Arc::clone(&self.workdir);
            let (_stop, cancel) = watch::channel(false);

            let (message, finish_reason, usage) = match client
                .chat_stream(
                    request,
                    |apply| {
                        if let crate::provider::StreamApply::ContentDelta(text) = apply {
                            append_assistant_delta(&ui_history, &text, &mut on);
                        }
                    },
                    cancel,
                )
                .await
            {
                Ok(acc) => {
                    let finish_reason = acc.finish_reason.clone();
                    let usage = acc.usage;
                    let message = acc.into_assistant_message();
                    (message, finish_reason, usage)
                }
                Err(e) if is_interrupted(&e) => return Err(e),
                Err(_) => {
                    let request = ChatRequest {
                        model: self.client.model().to_string(),
                        messages: self.api_history.clone(),
                        tools: Some(tool_schemas()),
                        tool_choice: Some(serde_json::json!("auto")),
                        stream: None,
                    };
                    let response = client.chat(request).await?;
                    let choice = response
                        .choices
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("empty choices"))?;
                    (
                        choice.message,
                        choice.finish_reason,
                        response.usage,
                    )
                }
            };

            apply_usage(&mut self.token_stats, usage.as_ref());
            self.refresh_context_estimate();

            if matches!(finish_reason.as_deref(), Some("length" | "max_tokens")) {
                self.push_system("Model stopped: output length limit.");
            }

            if let Some(calls) = &message.tool_calls {
                if !calls.is_empty() {
                    remove_trailing_empty_assistant(&self.ui_history);
                    self.api_history.push(message.clone());
                    for call in calls {
                        let name = call.function.name.clone();
                        let args = call.function.arguments.clone();
                        on(ObserveEvent::ToolStart {
                            name: &name,
                            arguments: &args,
                        });
                        let outcome =
                            execute_tool(&workdir, &name, &args, &self.shell_policy, false);
                        on(ObserveEvent::ToolEnd { body: &outcome.body });
                        self.ui_history.lock().unwrap().push(UiMessage::tool_result(
                            &name,
                            &args,
                            truncate(&outcome.body, 8000),
                            outcome.file_diff.clone(),
                        ));
                        self.api_history.push(ApiMessage::tool_result(
                            call.id.clone(),
                            name,
                            outcome.body,
                        ));
                    }
                    self.refresh_context_estimate();
                    continue;
                }
            }

            let text = message
                .text_content()
                .unwrap_or_else(|| "(empty response)".to_string());
            self.api_history.push(message);
            let streamed = self
                .ui_history
                .lock()
                .unwrap()
                .last()
                .is_some_and(|m| m.role == UiRole::Assistant && !m.content.is_empty());
            if !streamed {
                self.ui_history.lock().unwrap().push(UiMessage::with_role(
                    UiRole::Assistant,
                    text.clone(),
                ));
            }
            return Ok(text);
        }

        let msg = format!("Stopped after {MAX_TOOL_STEPS} tool steps.");
        self.push_error(msg.clone());
        Ok(msg)
    }

    /// Run a user turn without holding the async agent lock across HTTP I/O (enables live TUI updates).
    pub async fn run_user_turn_shared(
        agent: Arc<tokio::sync::Mutex<Self>>,
        user_text: String,
        mut on: impl FnMut(ObserveEvent<'_>) + Send,
        cancel: watch::Receiver<bool>,
        ui_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<String> {
        {
            let mut g = agent.lock().await;
            g.ui_history
                .lock()
                .unwrap()
                .push(UiMessage::with_role(UiRole::User, user_text.clone()));
            g.api_history.push(ApiMessage::user(user_text));
            g.refresh_context_estimate();
        }

        for _step in 0..MAX_TOOL_STEPS {
            if *cancel.borrow() {
                anyhow::bail!("turn interrupted");
            }

            let (client, request, ui_history, workdir) = {
                let g = agent.lock().await;
                (
                    g.client.clone(),
                    ChatRequest {
                        model: g.client.model().to_string(),
                        messages: g.api_history.clone(),
                        tools: Some(tool_schemas()),
                        tool_choice: Some(serde_json::json!("auto")),
                        stream: None,
                    },
                    g.ui_transcript(),
                    Arc::clone(&g.workdir),
                )
            };

            let cancel_rx = cancel.clone();
            let (message, finish_reason, usage) = match client
                .chat_stream(
                    request,
                    |apply| {
                        if let crate::provider::StreamApply::ContentDelta(text) = apply {
                            append_assistant_delta(&ui_history, &text, &mut on);
                        }
                    },
                    cancel_rx,
                )
                .await
            {
                Ok(acc) => {
                    let finish_reason = acc.finish_reason.clone();
                    let usage = acc.usage;
                    let message = acc.into_assistant_message();
                    (message, finish_reason, usage)
                }
                Err(e) if is_interrupted(&e) => return Err(e),
                Err(_) => {
                    let request = {
                        let g = agent.lock().await;
                        ChatRequest {
                            model: g.client.model().to_string(),
                            messages: g.api_history.clone(),
                            tools: Some(tool_schemas()),
                            tool_choice: Some(serde_json::json!("auto")),
                            stream: None,
                        }
                    };
                    let response = client.chat(request).await?;
                    let choice = response
                        .choices
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("empty choices"))?;
                    (
                        choice.message,
                        choice.finish_reason,
                        response.usage,
                    )
                }
            };

            let mut g = agent.lock().await;
            apply_usage(&mut g.token_stats, usage.as_ref());
            g.refresh_context_estimate();

            if matches!(finish_reason.as_deref(), Some("length" | "max_tokens")) {
                g.push_system("Model stopped: output length limit.");
            }

            if let Some(calls) = &message.tool_calls {
                if !calls.is_empty() {
                    remove_trailing_empty_assistant(&g.ui_history);
                    g.api_history.push(message.clone());
                    for call in calls {
                        let name = call.function.name.clone();
                        let args = call.function.arguments.clone();
                        on(ObserveEvent::ToolStart {
                            name: &name,
                            arguments: &args,
                        });
                        let approval = await_run_command_approval(
                            &name,
                            &args,
                            &g.shell_policy,
                            &ui_tx,
                        )
                        .await?;
                        if approval == Some(false) {
                            let denied = "error: user denied shell command".to_string();
                            on(ObserveEvent::ToolEnd { body: &denied });
                            g.ui_history.lock().unwrap().push(UiMessage::tool_result(
                                &name,
                                &args,
                                &denied,
                                None,
                            ));
                            g.api_history.push(ApiMessage::tool_result(
                                call.id.clone(),
                                name.clone(),
                                denied,
                            ));
                            g.refresh_context_estimate();
                            continue;
                        }
                        let bypass = approval == Some(true)
                            && shell_allowlist_bypass(&name, &args, &g.shell_policy);
                        let outcome = execute_tool(
                            &workdir,
                            &name,
                            &args,
                            &g.shell_policy,
                            bypass,
                        );
                        on(ObserveEvent::ToolEnd { body: &outcome.body });
                        g.ui_history.lock().unwrap().push(UiMessage::tool_result(
                            &name,
                            &args,
                            truncate(&outcome.body, 8000),
                            outcome.file_diff.clone(),
                        ));
                        g.api_history.push(ApiMessage::tool_result(
                            call.id.clone(),
                            name,
                            outcome.body,
                        ));
                    }
                    g.refresh_context_estimate();
                    drop(g);
                    continue;
                }
            }

            let text = message
                .text_content()
                .unwrap_or_else(|| "(empty response)".to_string());
            g.api_history.push(message);
            let streamed = g
                .ui_history
                .lock()
                .unwrap()
                .last()
                .is_some_and(|m| m.role == UiRole::Assistant && !m.content.is_empty());
            if !streamed {
                g.ui_history.lock().unwrap().push(UiMessage::with_role(
                    UiRole::Assistant,
                    text.clone(),
                ));
            }
            return Ok(text);
        }

        let msg = format!("Stopped after {MAX_TOOL_STEPS} tool steps.");
        agent.lock().await.push_error(msg.clone());
        Ok(msg)
    }
}

fn append_assistant_delta(
    ui_history: &Arc<Mutex<Vec<UiMessage>>>,
    chunk: &str,
    on: &mut impl FnMut(ObserveEvent<'_>),
) {
    if chunk.is_empty() {
        return;
    }
    let mut ui = ui_history.lock().unwrap();
    let needs_row = ui.last().is_none_or(|m| m.role != UiRole::Assistant);
    if needs_row {
        ui.push(UiMessage::with_role(UiRole::Assistant, String::new()));
    }
    if let Some(last) = ui.last_mut() {
        last.content.push_str(chunk);
    }
    drop(ui);
    on(ObserveEvent::AssistantDelta { text: chunk });
}

fn remove_trailing_empty_assistant(ui_history: &Arc<Mutex<Vec<UiMessage>>>) {
    let mut ui = ui_history.lock().unwrap();
    if let Some(last) = ui.last() {
        if last.role == UiRole::Assistant && last.content.is_empty() {
            ui.pop();
        }
    }
}

fn shell_allowlist_bypass(name: &str, args: &str, policy: &ShellPolicy) -> bool {
    if name != "run_command" {
        return false;
    }
    let Ok(parsed) = serde_json::from_str::<RunCommandArgs>(args) else {
        return false;
    };
    policy.is_restrictive() && !policy.is_allowed(&parsed.command)
}

async fn await_run_command_approval(
    name: &str,
    args: &str,
    policy: &ShellPolicy,
    ui_tx: &mpsc::UnboundedSender<AgentEvent>,
) -> Result<Option<bool>> {
    if name != "run_command" {
        return Ok(None);
    }
    let parsed: RunCommandArgs = serde_json::from_str(args).context("parse run_command JSON")?;
    if !policy.needs_interactive_approval(&parsed.command) {
        return Ok(None);
    }
    let (reply, rx) = tokio::sync::oneshot::channel();
    ui_tx
        .send(AgentEvent::CommandApproval(CommandApproval {
            command: parsed.command,
            reply,
        }))
        .map_err(|_| anyhow::anyhow!("ui channel closed"))?;
    Ok(Some(rx.await.unwrap_or(false)))
}

fn is_interrupted(err: &anyhow::Error) -> bool {
    err.to_string().contains("turn interrupted")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end: String = s.chars().take(max).collect();
    format!("{end}…")
}