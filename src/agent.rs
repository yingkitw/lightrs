mod tokens;

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;

pub use tokens::{TokenStats, TokenSummary, context_usage_text};

use crate::chat::{ApiMessage, ApiRole, ChatRequest, UiMessage, UiRole};
use crate::session::SessionFile;
use tokens::{apply_usage, estimate_context_tokens};
use crate::provider::LlmClient;
use crate::tools::{execute_tool, tool_schemas};

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

fn system_prompt(workdir: &Path) -> String {
    format!(
        r#"You are lightrs, a minimal terminal coding agent.
Workdir: {}.

Use tools to read or change the codebase and to run commands — do not invent file contents.
Prefer small, focused edits. Explain briefly after completing the task.
If a tool returns an error, fix the issue or ask the user for missing info."#,
        workdir.display()
    )
}

pub struct Agent {
    client: LlmClient,
    workdir: Arc<Path>,
    api_history: Vec<ApiMessage>,
    /// UI transcript; readable from the TUI thread while the worker streams LLM output.
    ui_history: Arc<Mutex<Vec<UiMessage>>>,
    token_stats: TokenStats,
    estimated_context_tokens: u32,
}

impl Agent {
    pub fn new(client: LlmClient, workdir: Arc<Path>) -> Self {
        let mut api_history = Vec::new();
        api_history.push(ApiMessage::system(system_prompt(&workdir)));
        let mut agent = Self {
            client,
            workdir,
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
        let prompt = ApiMessage::system(system_prompt(&self.workdir));
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
        self.ui_history.lock().unwrap().push(UiMessage {
            role: UiRole::System,
            content: content.into(),
        });
    }

    pub fn push_error(&mut self, content: impl Into<String>) {
        self.ui_history.lock().unwrap().push(UiMessage {
            role: UiRole::Error,
            content: content.into(),
        });
    }

    pub fn clear_conversation(&mut self) {
        self.api_history.clear();
        self.api_history
            .push(ApiMessage::system(system_prompt(&self.workdir)));
        let mut ui = self.ui_history.lock().unwrap();
        ui.clear();
        ui.push(UiMessage {
            role: UiRole::System,
            content: "Conversation cleared.".into(),
        });
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
        self.ui_history.lock().unwrap().push(UiMessage {
            role: UiRole::User,
            content: user_text.clone(),
        });
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

            let mut stream_failed = false;
            let (message, finish_reason, usage) = match client
                .chat_stream(request, |apply| {
                    if let crate::provider::StreamApply::ContentDelta(text) = apply {
                        append_assistant_delta(&ui_history, &text, &mut on);
                    }
                })
                .await
            {
                Ok(acc) => {
                    let finish_reason = acc.finish_reason.clone();
                    let usage = acc.usage;
                    let message = acc.into_assistant_message();
                    (message, finish_reason, usage)
                }
                Err(_) => {
                    stream_failed = true;
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

            if stream_failed {
                apply_usage(&mut self.token_stats, usage.as_ref());
            } else {
                apply_usage(&mut self.token_stats, usage.as_ref());
            }
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
                        self.ui_history.lock().unwrap().push(UiMessage {
                            role: UiRole::Tool,
                            content: format!("→ {name}({})", truncate(&args, 120)),
                        });
                        let result = execute_tool(&workdir, &name, &args);
                        on(ObserveEvent::ToolEnd { body: &result });
                        self.ui_history.lock().unwrap().push(UiMessage {
                            role: UiRole::Tool,
                            content: truncate(&result, 8000),
                        });
                        self.api_history.push(ApiMessage::tool_result(
                            call.id.clone(),
                            name,
                            result,
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
                self.ui_history.lock().unwrap().push(UiMessage {
                    role: UiRole::Assistant,
                    content: text.clone(),
                });
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
    ) -> Result<String> {
        {
            let mut g = agent.lock().await;
            g.ui_history.lock().unwrap().push(UiMessage {
                role: UiRole::User,
                content: user_text.clone(),
            });
            g.api_history.push(ApiMessage::user(user_text));
            g.refresh_context_estimate();
        }

        for _step in 0..MAX_TOOL_STEPS {
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

            let (message, finish_reason, usage) = match client
                .chat_stream(request, |apply| {
                    if let crate::provider::StreamApply::ContentDelta(text) = apply {
                        append_assistant_delta(&ui_history, &text, &mut on);
                    }
                })
                .await
            {
                Ok(acc) => {
                    let finish_reason = acc.finish_reason.clone();
                    let usage = acc.usage;
                    let message = acc.into_assistant_message();
                    (message, finish_reason, usage)
                }
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
                        g.ui_history.lock().unwrap().push(UiMessage {
                            role: UiRole::Tool,
                            content: format!("→ {name}({})", truncate(&args, 120)),
                        });
                        let result = execute_tool(&workdir, &name, &args);
                        on(ObserveEvent::ToolEnd { body: &result });
                        g.ui_history.lock().unwrap().push(UiMessage {
                            role: UiRole::Tool,
                            content: truncate(&result, 8000),
                        });
                        g.api_history.push(ApiMessage::tool_result(
                            call.id.clone(),
                            name,
                            result,
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
                g.ui_history.lock().unwrap().push(UiMessage {
                    role: UiRole::Assistant,
                    content: text.clone(),
                });
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
        ui.push(UiMessage {
            role: UiRole::Assistant,
            content: String::new(),
        });
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end: String = s.chars().take(max).collect();
    format!("{end}…")
}