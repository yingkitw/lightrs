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
    MAX_TOOL_STEPS, append_assistant_delta, is_interrupted, normalize_assistant_text,
    remove_trailing_empty_assistant, truncate_tool_ui,
};
use super::{Agent, ObserveEvent};

impl Agent {
    fn begin_user_turn(&mut self, user_text: String) {
        self.heal_ran = 0;
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

            let (client, fallback, request, ui_history, workdir, shell_policy) = {
                let mut g = agent.lock().await;
                (
                    g.client.clone(),
                    g.fallback_client.clone(),
                    g.build_chat_request(),
                    g.ui_transcript(),
                    Arc::clone(&g.workdir),
                    g.shell_policy.clone(),
                )
            };

            let (message, finish_reason, usage) = Self::fetch_model_message(
                &client,
                fallback.as_ref(),
                request,
                &ui_history,
                &mut on,
                cancel.clone(),
            )
            .await?;

            // Approval prompt must run with the mutex FREE: the TUI needs
            // `blocking_lock` to draw the banner and read the y/n reply.
            let calls = message.tool_calls.clone().unwrap_or_default();
            let shell_verdict =
                super::approval::resolve_shell_approvals(&calls, &shell_policy, Some(&ui_tx))
                    .await?;

            if calls.is_empty() {
                let mut g = agent.lock().await;
                g.after_model_response(usage.as_ref(), finish_reason.as_deref());
                return Ok(g.finish_text_turn(message));
            }

            {
                let mut g = agent.lock().await;
                g.after_model_response(usage.as_ref(), finish_reason.as_deref());
                g.record_tool_call_step(message);
            }
            // Tools run without the agent mutex held: slow subprocess calls and
            // heal commands must not freeze the UI.
            for call in &calls {
                execute_one_tool_shared(
                    &agent,
                    &workdir,
                    &shell_policy,
                    &call.function.name,
                    &call.function.arguments,
                    &call.id,
                    shell_verdict,
                    &mut on,
                    Some(&ui_tx),
                )
                .await?;
            }
            agent.lock().await.refresh_context_estimate();
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
            let fallback = self.fallback_client.clone();
            let request = self.build_chat_request();
            let ui_history = self.ui_transcript();
            let workdir = Arc::clone(&self.workdir);

            let (message, finish_reason, usage) = Self::fetch_model_message(
                &client,
                fallback.as_ref(),
                request,
                &ui_history,
                on,
                cancel.clone(),
            )
            .await?;

            self.after_model_response(usage.as_ref(), finish_reason.as_deref());
            let calls = message.tool_calls.clone().unwrap_or_default();
            let shell_verdict =
                super::approval::resolve_shell_approvals(&calls, &self.shell_policy, ui_tx).await?;
            if let Some(text) =
                self.handle_model_message(message, &workdir, shell_verdict, on, ui_tx)?
            {
                return Ok(text);
            }
        }

        let msg = format!("Stopped after {MAX_TOOL_STEPS} tool steps.");
        self.push_error(msg.clone());
        self.finish_user_turn_api();
        Ok(msg)
    }

    fn build_chat_request(&mut self) -> ChatRequest {
        self.auto_compact_context();
        ChatRequest {
            model: self.client.model().to_string(),
            messages: self.messages_for_request(),
            tools: Some(tool_schemas()),
            tool_choice: Some(serde_json::json!("auto")),
            stream: None,
        }
    }

    /// One model fetch; on a non-interrupt error, retry once on the fallback client.
    async fn fetch_model_message(
        client: &LlmClient,
        fallback: Option<&LlmClient>,
        request: ChatRequest,
        ui_history: &Arc<Mutex<Vec<UiMessage>>>,
        on: &mut impl FnMut(ObserveEvent<'_>),
        cancel: watch::Receiver<bool>,
    ) -> Result<(ApiMessage, Option<String>, Option<TokenUsage>)> {
        match Self::fetch_once(client, request.clone(), ui_history, on, cancel.clone()).await {
            Err(e) if is_interrupted(&e) => Err(e),
            Err(e) => {
                let Some(fb) = fallback else { return Err(e) };
                ui_history.lock().unwrap().push(UiMessage::with_role(
                    UiRole::System,
                    format!("model error — retrying with fallback `{}`", fb.model()),
                ));
                let mut retry = request;
                retry.model = fb.model().to_string();
                Self::fetch_once(fb, retry, ui_history, on, cancel).await
            }
            ok => ok,
        }
    }

    async fn fetch_once(
        client: &LlmClient,
        request: ChatRequest,
        ui_history: &Arc<Mutex<Vec<UiMessage>>>,
        on: &mut impl FnMut(ObserveEvent<'_>),
        cancel: watch::Receiver<bool>,
    ) -> Result<(ApiMessage, Option<String>, Option<TokenUsage>)> {
        on(ObserveEvent::ModelWait);
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
    /// `shell_verdict` is the step's batched approval result (`resolve_shell_approvals`).
    fn handle_model_message(
        &mut self,
        message: ApiMessage,
        workdir: &Path,
        shell_verdict: Option<bool>,
        on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
        ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<Option<String>> {
        if let Some(calls) = message.tool_calls.clone()
            && !calls.is_empty() {
                self.record_tool_call_step(message);
                for call in &calls {
                    execute_one_tool(
                        self,
                        workdir,
                        &self.shell_policy.clone(),
                        &call.function.name,
                        &call.function.arguments,
                        &call.id,
                        shell_verdict,
                        on,
                        ui_tx,
                    )?;
                }
                self.refresh_context_estimate();
                return Ok(None);
            }
        Ok(Some(self.finish_text_turn(message)))
    }

    /// Record an assistant tool-call message into API history; UI trailing
    /// empty streaming bubble is removed first.
    fn record_tool_call_step(&mut self, message: ApiMessage) {
        remove_trailing_empty_assistant(&self.ui_history);
        self.api_history.push(message);
    }

    /// Normal (non-tool) assistant text: store in both histories, end the API turn.
    fn finish_text_turn(&mut self, message: ApiMessage) -> String {
        let text = normalize_assistant_text(
            message
                .text_content()
                .as_deref()
                .unwrap_or("(empty response)"),
        );
        let mut stored = message;
        stored.content = Some(serde_json::Value::String(text.clone()));
        self.api_history.push(stored);
        let streamed = self
            .ui_history
            .lock()
            .unwrap()
            .last()
            .is_some_and(|m| m.role == UiRole::Assistant && !m.content.is_empty());
        if streamed {
            let mut ui = self.ui_history.lock().unwrap();
            if let Some(last) = ui.last_mut()
                && last.role == UiRole::Assistant
            {
                last.content = text.clone();
            }
        } else {
            self.ui_history.lock().unwrap().push(UiMessage::with_role(
                UiRole::Assistant,
                text.clone(),
            ));
        }
        self.finish_user_turn_api();
        text
    }

    /// Push a denied-tool result into both histories.
    fn record_tool_denial(&mut self, name: &str, args: &str, call_id: &str, msg: &str) {
        self.ui_history.lock().unwrap().push(UiMessage::tool_result(
            name,
            args,
            truncate_tool_ui(msg),
            None,
        ));
        self.api_history.push(ApiMessage::tool_result(
            call_id,
            name,
            super::context::truncate_tool_api(msg),
        ));
    }

    /// Push a tool result into both histories; flags prompt-injection patterns
    /// in the API copy and notifies the transcript.
    fn record_tool_outcome(
        &mut self,
        name: &str,
        args: &str,
        call_id: &str,
        body: &str,
        file_diff: Option<crate::chat::FileDiffUi>,
    ) {
        self.ui_history.lock().unwrap().push(UiMessage::tool_result(
            name,
            args,
            truncate_tool_ui(body),
            file_diff,
        ));
        let mut api_body = super::context::truncate_tool_api(body);
        if let Some(pat) = crate::tools::injection::detect(body) {
            self.push_system(format!(
                "tool `{name}` output matched injection pattern {pat:?} — flagged for the model"
            ));
            api_body = format!("{}{api_body}", crate::tools::injection::flag_prefix(pat));
        }
        self.api_history.push(ApiMessage::tool_result(call_id, name, api_body));
    }
}

/// `(command, needed_interactive_approval)` for `run_command` calls — audit context.
fn shell_ctx(name: &str, args: &str, policy: &ShellPolicy) -> Option<(String, bool)> {
    if name != "run_command" {
        return None;
    }
    let cmd = serde_json::from_str::<crate::tools::RunCommandArgs>(args)
        .map(|a| a.command)
        .unwrap_or_default();
    Some((cmd.clone(), policy.needs_interactive_approval(&cmd)))
}

fn audit_denied(workdir: &Path, ctx: &Option<(String, bool)>, headless: bool, msg: &str) {
    if let Some((cmd, _)) = ctx {
        let decision = if headless { "denied_headless" } else { "denied" };
        crate::audit::log_shell(workdir, cmd, decision, &crate::audit::result_head(msg));
    }
}

fn audit_outcome(workdir: &Path, ctx: &Option<(String, bool)>, bypass: bool, body: &str) {
    if let Some((cmd, needs)) = ctx {
        let decision = match (*needs, bypass) {
            (true, true) => "approved_bypass",
            (true, false) => "approved",
            _ => "auto",
        };
        crate::audit::log_shell(workdir, cmd, decision, &crate::audit::result_head(body));
    }
}

/// `&mut Agent` path — headless/ACP turn loop where no TUI thread shares the mutex.
#[allow(clippy::too_many_arguments)]
fn execute_one_tool(
    agent: &mut Agent,
    workdir: &Path,
    shell_policy: &ShellPolicy,
    name: &str,
    args: &str,
    call_id: &str,
    shell_verdict: Option<bool>,
    on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<()> {
    on(ObserveEvent::ToolStart {
        name,
        arguments: args,
    });
    let ctx = shell_ctx(name, args, shell_policy);
    let bypass =
        match resolve_shell_gate(name, args, shell_policy, shell_verdict, ui_tx.is_none())? {
            ShellGate::Denied(msg) => {
                audit_denied(workdir, &ctx, ui_tx.is_none(), &msg);
                on(ObserveEvent::ToolEnd { body: &msg });
                agent.record_tool_denial(name, args, call_id, &msg);
                return Ok(());
            }
            ShellGate::NotShell => false,
            ShellGate::Run {
                bypass_allowlist,
            } => bypass_allowlist,
        };
    let outcome = execute_tool(workdir, name, args, shell_policy, bypass);
    audit_outcome(workdir, &ctx, bypass, &outcome.body);
    on(ObserveEvent::ToolEnd {
        body: &outcome.body,
    });
    agent.record_tool_outcome(name, args, call_id, &outcome.body, outcome.file_diff);
    if let Some(cmd) = agent.heal_plan(name, &outcome.body) {
        on(ObserveEvent::ToolStart {
            name: "heal",
            arguments: &cmd,
        });
        let body = run_heal_command(workdir, &cmd, shell_policy);
        on(ObserveEvent::ToolEnd { body: &body });
        agent.heal_record(&cmd, &body);
    }
    Ok(())
}

/// Shared path — the agent lives behind `Arc<Mutex>` (TUI). The mutex is only
/// held for short record/mutate sections; tool execution and heal subprocesses
/// run on `spawn_blocking` so neither the UI nor the tokio worker stalls.
#[allow(clippy::too_many_arguments)]
async fn execute_one_tool_shared(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    workdir: &Path,
    shell_policy: &ShellPolicy,
    name: &str,
    args: &str,
    call_id: &str,
    shell_verdict: Option<bool>,
    on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<()> {
    on(ObserveEvent::ToolStart {
        name,
        arguments: args,
    });
    let ctx = shell_ctx(name, args, shell_policy);
    let bypass =
        match resolve_shell_gate(name, args, shell_policy, shell_verdict, ui_tx.is_none())? {
            ShellGate::Denied(msg) => {
                audit_denied(workdir, &ctx, ui_tx.is_none(), &msg);
                on(ObserveEvent::ToolEnd { body: &msg });
                agent
                    .lock()
                    .await
                    .record_tool_denial(name, args, call_id, &msg);
                return Ok(());
            }
            ShellGate::NotShell => false,
            ShellGate::Run {
                bypass_allowlist,
            } => bypass_allowlist,
        };
    let (wd, n, a, p) = (
        workdir.to_path_buf(),
        name.to_string(),
        args.to_string(),
        shell_policy.clone(),
    );
    let outcome = tokio::task::spawn_blocking(move || execute_tool(&wd, &n, &a, &p, bypass))
        .await
        .unwrap_or_else(|e| crate::tools::ToolOutcome::ok(format!("error: tool join: {e}")));
    audit_outcome(workdir, &ctx, bypass, &outcome.body);
    on(ObserveEvent::ToolEnd {
        body: &outcome.body,
    });
    let heal_cmd = {
        let mut g = agent.lock().await;
        g.record_tool_outcome(name, args, call_id, &outcome.body, outcome.file_diff);
        g.heal_plan(name, &outcome.body)
    };
    if let Some(cmd) = heal_cmd {
        on(ObserveEvent::ToolStart {
            name: "heal",
            arguments: &cmd,
        });
        let (wd, p) = (workdir.to_path_buf(), shell_policy.clone());
        let c = cmd.clone();
        let body =
            tokio::task::spawn_blocking(move || run_heal_command(&wd, &c, &p))
                .await
                .unwrap_or_else(|e| format!("error: heal join: {e}"));
        on(ObserveEvent::ToolEnd { body: &body });
        agent.lock().await.heal_record(&cmd, &body);
    }
    Ok(())
}

/// Write/edit tools whose success triggers the configured post-edit check.
const HEAL_TOOLS: &[&str] = &["write_file", "patch_file", "multi_edit"];

/// Run the configured heal/test command (bypasses the shell policy, 120s cap).
fn run_heal_command(workdir: &Path, command: &str, shell_policy: &ShellPolicy) -> String {
    crate::tools::shell::run_command(
        workdir,
        crate::tools::RunCommandArgs {
            command: command.into(),
            timeout_secs: Some(120),
        },
        shell_policy,
        true,
    )
    .unwrap_or_else(|e| format!("error: {e:#}"))
}

impl Agent {
    /// Whether this tool outcome should trigger a heal run; bumps `heal_ran`
    /// and returns the configured command when so.
    fn heal_plan(&mut self, tool_name: &str, tool_body: &str) -> Option<String> {
        if !HEAL_TOOLS.contains(&tool_name) || tool_body.starts_with("error:") {
            return None;
        }
        let heal = self.heal.as_ref()?;
        if self.heal_ran >= heal.max_steps {
            return None;
        }
        self.heal_ran += 1;
        Some(heal.command.clone())
    }

    /// Record a heal run's output: transcript entry, `[sys]`/`[heal]` feedback to the model.
    fn heal_record(&mut self, command: &str, body: &str) {
        self.ui_history.lock().unwrap().push(UiMessage::tool_result(
            "heal",
            command.to_string(),
            truncate_tool_ui(body),
            None,
        ));
        if body.starts_with("exit=0") {
            self.push_system(format!("heal ok: `{command}`"));
            return;
        }
        let mut api = format!(
            "[heal] `{command}` failed after your edit — fix the errors before continuing.\n{}",
            super::context::truncate_tool_api(body)
        );
        if let Some(pat) = crate::tools::injection::detect(body) {
            self.push_system(format!(
                "heal output matched injection pattern {pat:?} — flagged for the model"
            ));
            api = format!("{}{api}", crate::tools::injection::flag_prefix(pat));
        }
        self.api_history.push(ApiMessage::user(api));
    }
}

#[cfg(test)]
mod heal_tests {
    use super::*;
    use crate::agent::HealSetting;
    use crate::provider::LlmClient;
    use std::sync::Arc;

    fn make_agent(command: Option<&str>, max_steps: u32) -> Agent {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let mut agent = Agent::new(
            client,
            Arc::from(std::path::Path::new(".")),
            ShellPolicy::default(),
        );
        if let Some(cmd) = command {
            agent.set_heal(Some(HealSetting {
                command: cmd.into(),
                max_steps,
            }));
        }
        agent
    }

    /// Test shim: plan → run → record in one call (sync path).
    fn run_heal(agent: &mut Agent, workdir: &Path, tool: &str, body: &str) {
        if let Some(cmd) = agent.heal_plan(tool, body) {
            let out = run_heal_command(workdir, &cmd, &agent.shell_policy.clone());
            agent.heal_record(&cmd, &out);
        }
    }

    #[test]
    fn heal_failure_appends_user_message() {
        let dir = std::env::temp_dir();
        let mut agent = make_agent(Some("exit 7"), 3);
        let before = agent.api_history.len();
        run_heal(&mut agent, &dir, "write_file", "wrote");
        assert_eq!(agent.api_history.len(), before + 1);
        let last = agent.api_history.last().unwrap();
        assert!(matches!(last.role, crate::chat::ApiRole::User));
        assert!(last.text_content().unwrap().contains("[heal]"));
        assert_eq!(agent.heal_ran, 1);
    }

    #[test]
    fn heal_success_stays_out_of_api_history() {
        let dir = std::env::temp_dir();
        let mut agent = make_agent(Some("true"), 3);
        let before = agent.api_history.len();
        run_heal(&mut agent, &dir, "patch_file", "patched");
        assert_eq!(agent.api_history.len(), before);
        assert_eq!(agent.heal_ran, 1);
    }

    #[test]
    fn heal_skips_read_tools_and_failed_edits() {
        let dir = std::env::temp_dir();
        let mut agent = make_agent(Some("true"), 3);
        run_heal(&mut agent, &dir, "read_file", "x");
        run_heal(&mut agent, &dir, "write_file", "error: nope");
        assert_eq!(agent.heal_ran, 0);
    }

    #[test]
    fn heal_respects_step_cap() {
        let dir = std::env::temp_dir();
        let mut agent = make_agent(Some("true"), 1);
        run_heal(&mut agent, &dir, "write_file", "wrote");
        run_heal(&mut agent, &dir, "write_file", "wrote");
        assert_eq!(agent.heal_ran, 1);
    }

    #[test]
    fn heal_disabled_without_command() {
        let dir = std::env::temp_dir();
        let mut agent = make_agent(None, 3);
        run_heal(&mut agent, &dir, "write_file", "wrote");
        assert_eq!(agent.heal_ran, 0);
    }
}