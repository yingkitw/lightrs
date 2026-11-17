use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::{mpsc, watch};

use crate::chat::{ApiMessage, ChatRequest, TokenUsage, UiMessage, UiRole};
use crate::provider::{ModelClient, is_interrupted};
use crate::tools::{ShellPolicy, execute_tool, tool_schemas};
use crate::tui::AgentEvent;

use super::approval::{ShellGate, ask_user_outcome, resolve_shell_gate, spawn_agent_outcome};
use super::helpers::{
    MAX_TOOL_STEPS, append_assistant_delta, append_thinking_delta, normalize_assistant_text,
    remove_trailing_empty_assistant, truncate_tool_ui,
};
use super::{Agent, ObserveEvent};

impl Agent {
    fn begin_user_turn(&mut self, user_text: String) {
        self.heal_ran = 0;
        self.weak_active = self.weak_model.is_some();
        self.turn_started = Some(std::time::Instant::now());
        self.token_stats.begin_turn();
        self.ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::User, user_text.clone()));
        let api_full =
            crate::skills::api_user_content(&self.workdir, &user_text, &self.skill_catalog);
        self.active_user_expansion =
            super::context::user_expansion_suffix(&api_full, &user_text);
        // `$id` tokens that resolved to catalog skills — the attach is silent
        // on the wire, so record it for the status bar + transcript note.
        self.turn_skills = crate::skills::collect_skill_invocations(&user_text)
            .into_iter()
            .filter(|id| self.skill_catalog.skills.iter().any(|s| &s.id == id))
            .collect();
        self.api_history.push(ApiMessage::user(user_text));
        self.record_send_estimate(&api_full);
        if !self.turn_skills.is_empty() {
            let names = self
                .turn_skills
                .iter()
                .map(|id| format!("${id}"))
                .collect::<Vec<_>>()
                .join(", ");
            self.push_system(format!("skills attached: {names}"));
        }
        self.refresh_context_estimate();
        self.maybe_context_pressure_sys();
    }

    /// Headless turn (exec/ACP): no cancel, no approval UI — shell commands
    /// that would need a prompt are denied.
    pub async fn run_user_turn_headless(
        agent: Arc<tokio::sync::Mutex<Self>>,
        user_text: String,
        on: impl FnMut(ObserveEvent<'_>) + Send,
    ) -> Result<String> {
        let (_stop, cancel) = watch::channel(false);
        Self::run_user_turn_shared(agent, user_text, on, cancel, None).await
    }

    pub async fn run_user_turn_shared(
        agent: Arc<tokio::sync::Mutex<Self>>,
        user_text: String,
        mut on: impl FnMut(ObserveEvent<'_>) + Send,
        cancel: watch::Receiver<bool>,
        ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    ) -> Result<String> {
        {
            let mut g = agent.lock().await;
            g.begin_user_turn(user_text);
        }

        for _step in 0..MAX_TOOL_STEPS {
            if *cancel.borrow() {
                anyhow::bail!(crate::provider::INTERRUPTED);
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
                super::approval::resolve_shell_approvals(&calls, &shell_policy, ui_tx).await?;

            if calls.is_empty() {
                let mut g = agent.lock().await;
                g.after_model_response(usage.as_ref(), finish_reason.as_deref());
                return Ok(g.finish_text_turn(message));
            }

            {
                let mut g = agent.lock().await;
                g.after_model_response(usage.as_ref(), finish_reason.as_deref());
                g.record_tool_call_step(message);
                g.escalate_from_weak(&calls);
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
                    cancel.clone(),
                    &mut on,
                    ui_tx,
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


    fn build_chat_request(&mut self) -> ChatRequest {
        self.auto_compact_context();
        let messages = self.messages_for_request();
        // One-time transparency note when the wire window first trims history.
        let wire_turns = super::context::user_turn_count(&messages);
        let history_turns = super::context::user_turn_count(&self.api_history);
        if wire_turns < history_turns && !self.wire_window_noted {
            self.wire_window_noted = true;
            self.push_system(format!(
                "context window: sending last {wire_turns} of {history_turns} turns to the model (history kept for /rewind; /clear resets)"
            ));
        }
        ChatRequest {
            model: if self.weak_active {
                self.weak_model.clone().unwrap_or_default()
            } else {
                self.client.model().to_string()
            },
            messages,
            tools: Some(tool_schemas()),
            tool_choice: Some(serde_json::json!("auto")),
            stream: None,
            stream_options: None,
        }
    }

    /// One model fetch; on a non-interrupt error, retry once on the fallback client.
    async fn fetch_model_message(
        client: &Arc<dyn ModelClient>,
        fallback: Option<&Arc<dyn ModelClient>>,
        request: ChatRequest,
        ui_history: &Arc<Mutex<Vec<UiMessage>>>,
        on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
        cancel: watch::Receiver<bool>,
    ) -> Result<crate::provider::ModelReply> {
        on(ObserveEvent::ModelWait);
        let mut on_apply = |apply: crate::provider::StreamApply| match apply {
            crate::provider::StreamApply::ContentDelta(text) => {
                append_assistant_delta(ui_history, &text, on)
            }
            crate::provider::StreamApply::ReasoningDelta(text) => {
                append_thinking_delta(ui_history, &text, on)
            }
            crate::provider::StreamApply::ToolArgumentsDelta { .. } => {}
        };
        match client
            .send(request.clone(), &mut on_apply, cancel.clone())
            .await
        {
            Err(e) if is_interrupted(&e) => Err(e),
            Err(e) => {
                let Some(fb) = fallback else { return Err(e) };
                ui_history.lock().unwrap().push(UiMessage::with_role(
                    UiRole::System,
                    format!("model error — retrying with fallback `{}`", fb.model()),
                ));
                let mut retry = request;
                retry.model = fb.model().to_string();
                fb.send(retry, &mut on_apply, cancel).await
            }
            ok => ok,
        }
    }

    fn after_model_response(&mut self, usage: Option<&TokenUsage>, finish_reason: Option<&str>) {
        super::context::tokens::apply_usage(&mut self.token_stats, usage);
        self.refresh_context_estimate();
        self.record_model_usage(usage);
        self.maybe_context_pressure_sys();
        if matches!(finish_reason, Some("length" | "max_tokens")) {
            self.push_system("Model stopped: output length limit.");
        }
    }

    /// Record an assistant tool-call message into API history; UI trailing
    /// empty streaming bubble is removed first.
    fn record_tool_call_step(&mut self, message: ApiMessage) {
        remove_trailing_empty_assistant(&self.ui_history);
        self.api_history.push(message);
    }

    /// Weak-model routing (craftine pattern): a `weak_model` serves steps until
    /// one calls a write/exec tool, then the rest of the turn runs on the
    /// primary model — the weak pick still executes, only later reasoning
    /// escalates. One `[sys]` note per escalation.
    fn escalate_from_weak(&mut self, calls: &[crate::chat::ToolCall]) {
        const WRITE_STEP_TOOLS: &[&str] =
            &["write_file", "patch_file", "multi_edit", "run_command"];
        if self.weak_active
            && calls
                .iter()
                .any(|c| WRITE_STEP_TOOLS.contains(&c.function.name.as_str()))
        {
            self.weak_active = false;
            self.push_system(format!(
                "weak model → {} (write/exec tool called)",
                self.client.model()
            ));
        }
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

    /// Show the tool row immediately as `◌ … running` so slow calls are
    /// visible; `record_tool_*` replaces this row in place with the result.
    fn start_tool_ui(&mut self, name: &str, args: &str) {
        let mut row = UiMessage::tool_result(name, args, "running…", None);
        row.pending = true;
        let mut ui = self.ui_history.lock().unwrap();
        ui.push(row);
        self.pending_tool = Some((ui.len() - 1, std::time::Instant::now()));
    }

    /// Replace the pending tool row (or push when none — e.g. restored paths).
    fn finish_tool_ui(&mut self, mut row: UiMessage) {
        if let Some((idx, started)) = self.pending_tool.take() {
            let mut ui = self.ui_history.lock().unwrap();
            if ui.get(idx).is_some_and(|m| m.pending) {
                row.elapsed_ms = Some(started.elapsed().as_millis() as u64);
                ui[idx] = row;
                return;
            }
        }
        self.ui_history.lock().unwrap().push(row);
    }

    /// Push a denied-tool result into both histories.
    fn record_tool_denial(&mut self, name: &str, args: &str, call_id: &str, msg: &str) {
        self.finish_tool_ui(UiMessage::tool_result(
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
        if let Some(diff) = &file_diff {
            self.changed_files.insert(diff.path.clone());
        }
        self.finish_tool_ui(UiMessage::tool_result(
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
    cancel: watch::Receiver<bool>,
    on: &mut (impl FnMut(ObserveEvent<'_>) + Send),
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<()> {
    on(ObserveEvent::ToolStart {
        name,
        arguments: args,
    });
    agent.lock().await.start_tool_ui(name, args);
    let ctx = shell_ctx(name, args, shell_policy);
    if agent.lock().await.read_only() && READ_ONLY_DENY.contains(&name) {
        let msg = "error: plan mode is on — file edits and shell commands are disabled (/plan to exit)";
        audit_denied(workdir, &ctx, ui_tx.is_none(), msg);
        on(ObserveEvent::ToolEnd { body: msg });
        agent
            .lock()
            .await
            .record_tool_denial(name, args, call_id, msg);
        return Ok(());
    }
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
    let outcome = match name {
        "web_fetch" => crate::tools::web::run(workdir, args, Some(&cancel)).await,
        "ask_user" => ask_user_outcome(args, ui_tx, &cancel).await,
        "spawn_agent" => spawn_agent_outcome(agent, workdir, args, &cancel).await,
        "mcp_call" => crate::mcp::call_outcome(agent, args, &cancel).await,
        "lsp" => crate::lsp::call_outcome(agent, workdir, args, &cancel).await,
        _ => {
            let (wd, n, a, p) = (
                workdir.to_path_buf(),
                name.to_string(),
                args.to_string(),
                shell_policy.clone(),
            );
            let cancel_rx = cancel.clone();
            tokio::task::spawn_blocking(move || {
                execute_tool(&wd, &n, &a, &p, bypass, Some(&cancel_rx))
            })
            .await
            .unwrap_or_else(|e| crate::tools::ToolOutcome::ok(format!("error: tool join: {e}")))
        }
    };
    audit_outcome(workdir, &ctx, bypass, &outcome.body);
    on(ObserveEvent::ToolEnd {
        body: &outcome.body,
    });
    let (fmt_cmd, heal_cmd) = {
        let mut g = agent.lock().await;
        g.record_tool_outcome(name, args, call_id, &outcome.body, outcome.file_diff.clone());
        (
            g.format_plan(name, &outcome.body, outcome.file_diff.as_ref()),
            g.heal_plan(name, &outcome.body),
        )
    };
    for (label, cmd) in [("fmt", fmt_cmd), ("heal", heal_cmd)]
        .into_iter()
        .filter_map(|(l, c)| c.map(|c| (l, c)))
    {
        on(ObserveEvent::ToolStart {
            name: label,
            arguments: &cmd,
        });
        let (wd, p) = (workdir.to_path_buf(), shell_policy.clone());
        let c = cmd.clone();
        let hook_cancel = cancel.clone();
        let body =
            tokio::task::spawn_blocking(move || run_heal_command(&wd, &c, &p, Some(&hook_cancel)))
                .await
                .unwrap_or_else(|e| format!("error: {label} join: {e}"));
        on(ObserveEvent::ToolEnd { body: &body });
        agent.lock().await.hook_record(label, &cmd, &body);
    }
    Ok(())
}

/// Write/edit tools whose success triggers the configured post-edit check.
const HEAL_TOOLS: &[&str] = &["write_file", "patch_file", "multi_edit"];

/// Tools denied outright in plan/read-only mode (`/plan`, `--read-only`).
/// `todo_write` stays — it's session scratch, not a file or shell change.
const READ_ONLY_DENY: &[&str] = &[
    "write_file",
    "patch_file",
    "multi_edit",
    "run_command",
    // MCP tools have unknown side effects — fail closed in plan mode.
    "mcp_call",
];

/// Run the configured heal/test command (bypasses the shell policy, 120s cap).
fn run_heal_command(
    workdir: &Path,
    command: &str,
    shell_policy: &ShellPolicy,
    cancel: Option<&watch::Receiver<bool>>,
) -> String {
    crate::tools::shell::run_command(
        workdir,
        crate::tools::RunCommandArgs {
            command: command.into(),
            timeout_secs: Some(120),
        },
        shell_policy,
        true,
        cancel,
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

    /// Whether this write outcome should trigger the configured formatter
    /// (`[shell].format_command`). `{path}` substitutes the diff path;
    /// otherwise the path is appended, single-quoted.
    fn format_plan(
        &mut self,
        tool_name: &str,
        tool_body: &str,
        diff: Option<&crate::chat::FileDiffUi>,
    ) -> Option<String> {
        if !HEAL_TOOLS.contains(&tool_name) || tool_body.starts_with("error:") {
            return None;
        }
        let cmd = self.format_cmd.as_ref()?;
        let path = diff?.path.clone();
        if cmd.contains("{path}") {
            Some(cmd.replace("{path}", &path))
        } else {
            Some(format!("{cmd} '{}'", path.replace('\'', "'\\''")))
        }
    }

    /// Record a post-edit hook run's output (`fmt` or `heal`): transcript row
    /// plus model feedback on failure. Heal failures ask for a fix; fmt
    /// failures just report — the model decides whether to act.
    fn hook_record(&mut self, label: &str, command: &str, body: &str) {
        self.ui_history.lock().unwrap().push(UiMessage::tool_result(
            label,
            command.to_string(),
            truncate_tool_ui(body),
            None,
        ));
        if body.starts_with("exit=0") {
            self.push_system(format!("{label} ok: `{command}`"));
            return;
        }
        let hint = if label == "heal" {
            " — fix the errors before continuing"
        } else {
            ""
        };
        let mut api = format!(
            "[{label}] `{command}` failed after your edit{hint}.\n{}",
            super::context::truncate_tool_api(body)
        );
        if let Some(pat) = crate::tools::injection::detect(body) {
            self.push_system(format!(
                "{label} output matched injection pattern {pat:?} — flagged for the model"
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
            let out = run_heal_command(workdir, &cmd, &agent.shell_policy.clone(), None);
            agent.hook_record("heal", &cmd, &out);
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

    fn diff(path: &str) -> crate::chat::FileDiffUi {
        crate::chat::FileDiffUi {
            path: path.into(),
            unified: String::new(),
            lines_added: 1,
            lines_removed: 0,
            is_new_file: false,
        }
    }

    #[test]
    fn format_plan_substitutes_path_or_appends_quoted() {
        let mut agent = make_agent(None, 3);
        agent.set_format_command(Some("cargo fmt -- {path}".into()));
        let d = diff("src/main.rs");
        assert_eq!(
            agent.format_plan("write_file", "wrote", Some(&d)).as_deref(),
            Some("cargo fmt -- src/main.rs")
        );
        agent.set_format_command(Some("rustfmt".into()));
        let d = diff("weird 'name.rs");
        assert_eq!(
            agent.format_plan("patch_file", "ok", Some(&d)).as_deref(),
            Some("rustfmt 'weird '\\''name.rs'")
        );
        // Read tools, failed edits, and missing diffs never trigger the hook.
        assert!(agent.format_plan("read_file", "x", Some(&d)).is_none());
        assert!(agent.format_plan("write_file", "error: no", Some(&d)).is_none());
        assert!(agent.format_plan("write_file", "wrote", None).is_none());
    }
}
#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::chat::{FunctionCall, ToolCall};
    use crate::provider::{MockModel, ModelClient};

    fn agent() -> Agent {
        Agent::new(
            MockModel::text("m", "hi"),
            Arc::from(std::path::Path::new(".")),
            ShellPolicy::default(),
        )
    }

    fn chat_request() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            messages: vec![],
            tools: None,
            tool_choice: None,
            stream: None,
            stream_options: None,
        }
    }

    #[tokio::test]
    async fn fallback_retries_on_primary_error() {
        let primary: Arc<dyn ModelClient> =
            Arc::new(MockModel::new("m", vec![Err(anyhow::anyhow!("boom"))]));
        let fb = Arc::new(MockModel::text("fb", "rescued"));
        let fb_dyn: Arc<dyn ModelClient> = fb.clone();
        let ui = Arc::new(Mutex::new(Vec::new()));
        let (_tx, rx) = watch::channel(false);
        let (msg, _, _) = Agent::fetch_model_message(
            &primary,
            Some(&fb_dyn),
            chat_request(),
            &ui,
            &mut |_| {},
            rx,
        )
        .await
        .expect("fallback rescues the step");

        assert_eq!(msg.text_content().unwrap(), "rescued");
        let reqs = fb.requests.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].model, "fb", "request rewritten to fallback model");
        assert!(
            ui.lock()
                .unwrap()
                .iter()
                .any(|m| m.content.contains("retrying with fallback"))
        );
    }

    #[tokio::test]
    async fn reasoning_content_lands_in_thinking_row() {
        let msg = ApiMessage {
            role: crate::chat::ApiRole::Assistant,
            content: Some(serde_json::Value::String("the answer".into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: Some("step by step".into()),
        };
        let primary: Arc<dyn ModelClient> = Arc::new(MockModel::new(
            "m",
            vec![Ok((msg, Some("stop".into()), None))],
        ));
        let ui = Arc::new(Mutex::new(Vec::new()));
        let (_tx, rx) = watch::channel(false);
        Agent::fetch_model_message(&primary, None, chat_request(), &ui, &mut |_| {}, rx)
            .await
            .expect("reply");
        let lock = ui.lock().unwrap();
        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].role, UiRole::Thinking);
        assert_eq!(lock[0].content, "step by step");
    }

    #[tokio::test]
    async fn interrupt_never_retries_fallback() {
        let primary: Arc<dyn ModelClient> = Arc::new(MockModel::new(
            "m",
            vec![Err(anyhow::anyhow!("turn interrupted"))],
        ));
        let fb = Arc::new(MockModel::text("fb", "rescued"));
        let fb_dyn: Arc<dyn ModelClient> = fb.clone();
        let ui = Arc::new(Mutex::new(Vec::new()));
        let (_tx, rx) = watch::channel(false);
        let err = Agent::fetch_model_message(
            &primary,
            Some(&fb_dyn),
            chat_request(),
            &ui,
            &mut |_| {},
            rx,
        )
        .await
        .expect_err("interrupt propagates");

        assert!(is_interrupted(&err));
        assert_eq!(fb.requests.lock().unwrap().len(), 0, "no fallback retry");
    }

    #[test]
    fn weak_model_used_while_active() {
        let mut a = agent();
        a.set_weak_model(Some("weak".into()));
        a.weak_active = true;
        assert_eq!(a.build_chat_request().model, "weak");
        a.weak_active = false;
        assert_eq!(a.build_chat_request().model, "m");
    }

    #[test]
    fn pending_tool_row_replaced_in_place_with_elapsed() {
        let mut a = agent();
        let before = a.ui_history.lock().unwrap().len();
        a.start_tool_ui("run_command", "{\"command\":\"x\"}");
        {
            let ui = a.ui_history.lock().unwrap();
            assert_eq!(ui.len(), before + 1);
            assert!(ui[before].pending, "row starts pending");
        }
        a.record_tool_outcome("run_command", "{}", "c1", "done-body", None);
        {
            let ui = a.ui_history.lock().unwrap();
            assert_eq!(ui.len(), before + 1, "result replaced in place");
            assert!(!ui[before].pending);
            assert_eq!(ui[before].content, "done-body");
            assert!(ui[before].elapsed_ms.is_some());
        }
        assert!(a.pending_tool.is_none(), "pending slot consumed");
    }

    #[test]
    fn record_tool_outcome_tracks_changed_files() {
        let mut a = agent();
        a.start_tool_ui("write_file", "{}");
        a.record_tool_outcome(
            "write_file",
            "{}",
            "c1",
            "wrote",
            Some(crate::chat::FileDiffUi {
                path: "src/a.rs".into(),
                unified: "+x".into(),
                lines_added: 1,
                lines_removed: 0,
                is_new_file: false,
            }),
        );
        a.start_tool_ui("read_file", "{}");
        a.record_tool_outcome("read_file", "{}", "c2", "ok", None);
        let files: Vec<&String> = a.changed_files().collect();
        assert_eq!(files, ["src/a.rs"], "only diff-bearing tools count");
    }

    #[test]
    fn begin_user_turn_records_attached_skills() {
        let mut a = agent();
        a.set_skill_catalog(crate::skills::SkillCatalog {
            skills: vec![crate::skills::SkillEntry {
                id: "lean".into(),
                name: "Lean".into(),
                description: String::new(),
                path: std::path::PathBuf::from("."),
                body: "keep it small".into(),
                trigger: None,
            }],
            rules: String::new(),
        });
        a.begin_user_turn("run $lean and $ghost".into());
        assert_eq!(a.turn_skills(), &["lean"], "unknown $ids don't count");
        let ui = a.ui_history.lock().unwrap();
        assert!(
            ui.iter().any(|m| m.content.contains("skills attached: $lean")),
            "transcript note missing"
        );
    }

    #[test]
    fn interrupted_turn_unsticks_pending_row() {
        let mut a = agent();
        a.start_tool_ui("run_command", "{}");
        let idx = a.ui_history.lock().unwrap().len() - 1;
        a.finish_user_turn_api();
        let ui = a.ui_history.lock().unwrap();
        assert!(!ui[idx].pending);
        assert_eq!(ui[idx].content, "(interrupted)");
        assert!(ui[idx].elapsed_ms.is_some());
    }

    #[test]
    fn write_tool_escalates_weak_to_primary() {
        let mut a = agent();
        a.set_weak_model(Some("weak".into()));
        a.weak_active = true;
        let call = |name: &str| ToolCall {
            id: "c".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
        };
        a.escalate_from_weak(&[call("read_file")]);
        assert!(a.weak_active, "read-only stays weak");
        a.escalate_from_weak(&[call("write_file")]);
        assert!(!a.weak_active, "write escalates");
        let sys = a.ui_history.lock().unwrap();
        assert!(sys
            .iter()
            .any(|m| matches!(m.role, UiRole::System) && m.content.contains("weak model → m")));
    }
}
