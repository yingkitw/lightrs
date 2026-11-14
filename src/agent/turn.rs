use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::{mpsc, watch};

use crate::chat::{ApiMessage, ChatRequest, TokenUsage, UiMessage, UiRole};
use crate::provider::LlmClient;
use crate::tools::{ShellPolicy, execute_tool, tool_schemas};
use crate::tui::AgentEvent;

use super::approval::{ShellGate, resolve_shell_gate};
use super::helpers::{
    MAX_TOOL_STEPS, append_assistant_delta, is_interrupted, remove_trailing_empty_assistant,
    truncate_tool_ui,
};
use super::{Agent, ObserveEvent};

impl Agent {
    fn begin_user_turn(&mut self, user_text: String) {
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::User, user_text.clone()));
        let api_full =
            crate::skills::api_user_content(&self.workdir, &user_text, &self.skill_catalog);
        self.active_user_expansion =
            super::context::user_expansion_suffix(&api_full, &user_text);
        self.api_history.push(ApiMessage::user(user_text));
        self.record_send_estimate(&api_full);
        self.refresh_context_estimate();
        self.maybe_context_pressure_sys();
    }

    pub async fn run_user_turn(&mut self, user_text: String) -> Result<String> {
        self.run_user_turn_observed(user_text, |_| {}).await
    }

    pub async fn run_user_turn_observed(
        &mut self,
        user_text: String,
        mut on: impl FnMut(ObserveEvent<'_>) + Send,
    ) -> Result<String> {
        self.begin_user_turn(user_text);
        let (_stop, cancel) = watch::channel(false);
        self.drive_turn_loop(&mut on, cancel, None).await
    }

    pub async fn run_user_turn_shared(
        agent: Arc<tokio::sync::Mutex<Self>>,
        user_text: String,
        mut on: impl FnMut(ObserveEvent<'_>) + Send,
        cancel: watch::Receiver<bool>,
        ui_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<String> {
        {
            let mut g = agent.lock().await;
            g.begin_user_turn(user_text);
        }

        for _step in 0..MAX_TOOL_STEPS {
            if *cancel.borrow() {
                anyhow::bail!("turn interrupted");
            }

            let (client, request, ui_history, workdir) = {
                let g = agent.lock().await;
                (
                    g.client.clone(),
                    g.build_chat_request(),
                    g.ui_transcript(),
                    Arc::clone(&g.workdir),
                )
            };

            let (message, finish_reason, usage) = Self::fetch_model_message(
                &client,
                request,
                &ui_history,
                &mut on,
                cancel.clone(),
            )
            .await?;

            let mut g = agent.lock().await;
            g.after_model_response(usage.as_ref(), finish_reason.as_deref());

            if let Some(text) = g
                .handle_model_message(
                    message,
                    &workdir,
                    &mut on,
                    Some(&ui_tx),
                )
                .await?
            {
                return Ok(text);
            }
            drop(g);
        }

        let msg = format!("Stopped after {MAX_TOOL_STEPS} tool steps.");
        let mut g = agent.lock().await;
        g.push_error(msg.clone());
        g.finish_user_turn_api();
        Ok(msg)
    }

    async fn drive_turn_loop(
        &mut self,
        on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
        cancel: watch::Receiver<bool>,
        ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<String> {
        for _step in 0..MAX_TOOL_STEPS {
            let client = self.client.clone();
            let request = self.build_chat_request();
            let ui_history = self.ui_transcript();
            let workdir = Arc::clone(&self.workdir);

            let (message, finish_reason, usage) =
                Self::fetch_model_message(&client, request, &ui_history, on, cancel.clone()).await?;

            self.after_model_response(usage.as_ref(), finish_reason.as_deref());
            if let Some(text) = self
                .handle_model_message(message, &workdir, on, ui_tx)
                .await?
            {
                return Ok(text);
            }
        }

        let msg = format!("Stopped after {MAX_TOOL_STEPS} tool steps.");
        self.push_error(msg.clone());
        self.finish_user_turn_api();
        Ok(msg)
    }

    fn build_chat_request(&self) -> ChatRequest {
        ChatRequest {
            model: self.client.model().to_string(),
            messages: self.messages_for_request(),
            tools: Some(tool_schemas()),
            tool_choice: Some(serde_json::json!("auto")),
            stream: None,
        }
    }

    async fn fetch_model_message(
        client: &LlmClient,
        request: ChatRequest,
        ui_history: &Arc<Mutex<Vec<UiMessage>>>,
        on: &mut impl FnMut(ObserveEvent<'_>),
        cancel: watch::Receiver<bool>,
    ) -> Result<(ApiMessage, Option<String>, Option<TokenUsage>)> {
        let messages = request.messages.clone();
        let model = request.model.clone();
        match client
            .chat_stream(
                request,
                |apply| {
                    if let crate::provider::StreamApply::ContentDelta(text) = apply {
                        append_assistant_delta(ui_history, &text, on);
                    }
                },
                cancel,
            )
            .await
        {
            Ok(acc) => {
                let finish_reason = acc.finish_reason.clone();
                let usage = acc.usage;
                Ok((acc.into_assistant_message(), finish_reason, usage))
            }
            Err(e) if is_interrupted(&e) => Err(e),
            Err(_) => {
                let response = client
                    .chat(ChatRequest {
                        model,
                        messages,
                        tools: Some(tool_schemas()),
                        tool_choice: Some(serde_json::json!("auto")),
                        stream: None,
                    })
                    .await?;
                let choice = response
                    .choices
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("empty choices"))?;
                Ok((choice.message, choice.finish_reason, response.usage))
            }
        }
    }

    fn after_model_response(&mut self, usage: Option<&TokenUsage>, finish_reason: Option<&str>) {
        super::tokens::apply_usage(&mut self.token_stats, usage);
        self.refresh_context_estimate();
        self.record_model_usage(usage);
        self.maybe_context_pressure_sys();
        if matches!(finish_reason, Some("length" | "max_tokens")) {
            self.push_system("Model stopped: output length limit.");
        }
    }

    /// `Ok(Some(text))` when the turn completes with assistant text; `Ok(None)` to continue the tool loop.
    async fn handle_model_message(
        &mut self,
        message: ApiMessage,
        workdir: &Path,
        on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
        ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<Option<String>> {
        if let Some(calls) = message.tool_calls.clone() {
            if !calls.is_empty() {
                remove_trailing_empty_assistant(&self.ui_history);
                self.api_history.push(message);
                for call in calls {
                    execute_one_tool(
                        self,
                        workdir,
                        &self.shell_policy.clone(),
                        &call.function.name,
                        &call.function.arguments,
                        &call.id,
                        on,
                        ui_tx,
                    )
                    .await?;
                }
                self.refresh_context_estimate();
                return Ok(None);
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
        self.finish_user_turn_api();
        Ok(Some(text))
    }
}

async fn execute_one_tool(
    agent: &mut Agent,
    workdir: &Path,
    shell_policy: &ShellPolicy,
    name: &str,
    args: &str,
    call_id: &str,
    on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<()> {
    on(ObserveEvent::ToolStart {
        name,
        arguments: args,
    });
    let bypass = match resolve_shell_gate(name, args, shell_policy, ui_tx).await? {
        ShellGate::Denied(msg) => {
            on(ObserveEvent::ToolEnd { body: &msg });
            agent.ui_history.lock().unwrap().push(UiMessage::tool_result(
                name,
                args,
                truncate_tool_ui(&msg),
                None,
            ));
            agent.api_history.push(ApiMessage::tool_result(
                call_id,
                name,
                super::context::truncate_tool_api(&msg),
            ));
            return Ok(());
        }
        ShellGate::NotShell => false,
        ShellGate::Run {
            bypass_allowlist,
        } => bypass_allowlist,
    };
    let outcome = execute_tool(workdir, name, args, shell_policy, bypass);
    on(ObserveEvent::ToolEnd { body: &outcome.body });
    agent.ui_history.lock().unwrap().push(UiMessage::tool_result(
        name,
        args,
        truncate_tool_ui(&outcome.body),
        outcome.file_diff.clone(),
    ));
    agent.api_history.push(ApiMessage::tool_result(
        call_id,
        name,
        super::context::truncate_tool_api(&outcome.body),
    ));
    Ok(())
}