use anyhow::{Context, Result};
use tokio::sync::{mpsc, watch};

use crate::chat::ToolCall;
use crate::provider::INTERRUPTED;
use crate::tools::{RunCommandArgs, ShellPolicy, ToolOutcome};
use crate::tui::{AgentEvent, AskUserRequest, CommandApproval};

/// Result of shell gating before `execute_tool`.
pub(crate) enum ShellGate {
    NotShell,
    Run { bypass_allowlist: bool },
    Denied(String),
}

/// One approval prompt per step: collect every `run_command` call in `calls`
/// that needs interactive approval and ask the TUI once. `Ok(Some(verdict))`
/// applies to all calls in the batch; `Ok(None)` means nothing needed approval
/// or no TUI is attached (headless denial is worded in `resolve_shell_gate`).
pub(crate) async fn resolve_shell_approvals(
    calls: &[ToolCall],
    policy: &ShellPolicy,
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
) -> Result<Option<bool>> {
    let commands: Vec<String> = calls
        .iter()
        .filter(|c| c.function.name == "run_command")
        .filter_map(|c| serde_json::from_str::<RunCommandArgs>(&c.function.arguments).ok())
        .map(|a| a.command)
        .filter(|cmd| policy.needs_interactive_approval(cmd))
        .collect();
    if commands.is_empty() {
        return Ok(None);
    }
    let Some(tx) = ui_tx else {
        return Ok(None);
    };
    let (reply, rx) = tokio::sync::oneshot::channel();
    tx.send(AgentEvent::CommandApproval(CommandApproval {
        commands,
        reply,
    }))
    .map_err(|_| anyhow::anyhow!("ui channel closed"))?;
    Ok(Some(rx.await.unwrap_or(false)))
}

/// `verdict`: batch approval result for this call (`Some`) when it was in an
/// approval batch; `None` means the step never prompted (no approval needed or
/// headless). `headless` only picks the denial wording.
pub(crate) fn resolve_shell_gate(
    name: &str,
    args: &str,
    policy: &ShellPolicy,
    verdict: Option<bool>,
    headless: bool,
) -> Result<ShellGate> {
    if name != "run_command" {
        return Ok(ShellGate::NotShell);
    }
    let parsed: RunCommandArgs = serde_json::from_str(args).context("parse run_command JSON")?;
    if !policy.needs_interactive_approval(&parsed.command) {
        return Ok(ShellGate::Run {
            bypass_allowlist: false,
        });
    }
    match verdict {
        Some(true) => Ok(ShellGate::Run {
            bypass_allowlist: policy.is_restrictive() && !policy.is_allowed(&parsed.command),
        }),
        _ => Ok(ShellGate::Denied(
            if headless {
                "error: shell command requires TUI approval (use lightrs TUI or relax [shell] in config)"
            } else {
                "error: user denied shell command"
            }
            .into(),
        )),
    }
}

#[derive(serde::Deserialize)]
struct AskUserArgs {
    question: String,
}

/// `ask_user` — route the question through the TUI and wait for the answer.
/// Headless/ACP have no ask channel: fail closed like shell approvals. The
/// question text is already visible via the `ToolStart` row's arguments.
pub(crate) async fn ask_user_outcome(
    args: &str,
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
    cancel: &watch::Receiver<bool>,
) -> ToolOutcome {
    let Some(tx) = ui_tx else {
        return ToolOutcome::ok(
            "error: ask_user needs an interactive TUI (unavailable in exec/ACP)".to_string(),
        );
    };
    let fut = async {
        match serde_json::from_str::<AskUserArgs>(args).context("parse ask_user JSON") {
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
            Ok(q) => {
                let (reply, rx) = tokio::sync::oneshot::channel();
                let req = AskUserRequest {
                    question: q.question,
                    reply,
                };
                if tx.send(AgentEvent::AskUser(req)).is_err() {
                    return ToolOutcome::ok("error: ui channel closed".to_string());
                }
                ToolOutcome::ok(rx.await.unwrap_or_else(|_| "error: answer channel dropped".to_string()))
            }
        }
    };
    let mut rx = cancel.clone();
    if *rx.borrow() {
        return ToolOutcome::ok(INTERRUPTED.to_string());
    }
    tokio::select! {
        out = fut => out,
        r = rx.changed() => {
            let _ = r;
            ToolOutcome::ok(INTERRUPTED.to_string())
        }
    }
}

#[derive(serde::Deserialize)]
struct SpawnAgentArgs {
    prompt: String,
    #[serde(default)]
    read_only: bool,
}

/// `spawn_agent` — run a delegated prompt in a child `Agent` sharing the
/// parent's client `Arc` (stateless per request). Depth-capped: sub-agents
/// cannot spawn agents. Headless — shell approvals inside the child fail
/// closed like `exec`. Cancel propagates through the shared watch token.
pub(crate) async fn spawn_agent_outcome(
    parent: &std::sync::Arc<tokio::sync::Mutex<super::Agent>>,
    workdir: &std::path::Path,
    args: &str,
    cancel: &watch::Receiver<bool>,
) -> ToolOutcome {
    let parsed: Result<SpawnAgentArgs> =
        serde_json::from_str(args).context("parse spawn_agent JSON");
    let (
        client,
        fallback,
        weak,
        heal,
        fmt,
        catalog,
        mcp,
        lsp,
        mcp_names,
        policy,
        depth,
        parent_read_only,
    ) = {
        let g = parent.lock().await;
        (
            g.client.clone(),
            g.fallback_client.clone(),
            g.weak_model.clone(),
            g.heal.clone(),
            g.format_cmd.clone(),
            g.skill_catalog.clone(),
            g.mcp.clone(),
            g.lsp.clone(),
            g.mcp_names.clone(),
            g.shell_policy.clone(),
            g.depth,
            g.read_only,
        )
    };
    let mut policy = policy;
    // Fresh session cwd — a child's `cd` must not move the parent's session.
    policy.reset_session_cwd();
    if depth >= 1 {
        return ToolOutcome::ok(
            "error: spawn_agent depth limit — sub-agents cannot spawn agents".to_string(),
        );
    }
    let a = match parsed {
        Ok(a) if !a.prompt.trim().is_empty() => a,
        Ok(_) => return ToolOutcome::ok("error: spawn_agent needs a non-empty prompt"),
        Err(e) => return ToolOutcome::ok(format!("error: {e:#}")),
    };
    let mut child =
        super::Agent::new_shared(client, std::sync::Arc::from(workdir), policy, depth + 1);
    // Inherit the model-layer + hook settings so "same tools" is honest:
    // without fallback the child dies where the parent recovers; without
    // format/heal its writes skip the project's checks.
    child.fallback_client = fallback;
    child.weak_model = weak;
    child.heal = heal;
    child.format_cmd = fmt;
    child.skill_catalog = catalog;
    // Shared managers: the child hits the same server processes — the mutex
    // serializes access and no second spawn is needed.
    child.mcp = mcp;
    child.lsp = lsp;
    child.mcp_names = mcp_names;
    // Plan mode inherits: a writable child would bypass the parent's mode.
    if a.read_only || parent_read_only {
        child.set_read_only(true);
    }
    // Rebuild the prompt so the inherited catalog + mode are visible.
    child.ensure_system_prompt();
    let child = std::sync::Arc::new(tokio::sync::Mutex::new(child));
    // Boxed: spawn_agent → run_user_turn → spawn_agent is async recursion.
    let fut = Box::pin(super::Agent::run_user_turn_shared(
        child,
        a.prompt,
        |_| {},
        cancel.clone(),
        None,
    ));
    match fut.await {
        Ok(text) => ToolOutcome::ok(text),
        Err(e) => ToolOutcome::ok(format!("error: sub-agent failed: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{FunctionCall, ToolCall};
    use crate::tools::ShellPolicy;

    fn shell_call(id: &str, command: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "run_command".into(),
                arguments: format!(r#"{{"command":"{command}"}}"#),
            },
        }
    }

    #[test]
    fn headless_read_only_skips_approval_when_required() {
        let policy = ShellPolicy::new(vec![], true);
        let gate = resolve_shell_gate("run_command", r#"{"command":"ls"}"#, &policy, None, true)
            .unwrap();
        assert!(matches!(gate, ShellGate::Run { .. }));
    }

    #[test]
    fn headless_denies_write_under_require_approval() {
        let policy = ShellPolicy::new(vec![], true);
        let gate = resolve_shell_gate("run_command", r#"{"command":"rm foo"}"#, &policy, None, true)
            .unwrap();
        match gate {
            ShellGate::Denied(msg) => assert!(msg.contains("requires TUI approval")),
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn batch_approve_runs_all() {
        let policy = ShellPolicy::new(vec![], true);
        for cmd in ["rm a", "mv b c"] {
            let args = format!(r#"{{"command":"{cmd}"}}"#);
            let gate = resolve_shell_gate("run_command", &args, &policy, Some(true), false).unwrap();
            assert!(matches!(gate, ShellGate::Run { .. }));
        }
    }

    #[test]
    fn batch_deny_denies_all() {
        let policy = ShellPolicy::new(vec![], true);
        let gate =
            resolve_shell_gate("run_command", r#"{"command":"rm a"}"#, &policy, Some(false), false)
                .unwrap();
        match gate {
            ShellGate::Denied(msg) => assert_eq!(msg, "error: user denied shell command"),
            _ => panic!("expected Denied"),
        }
    }

    #[tokio::test]
    async fn approvals_empty_when_nothing_needs_prompt() {
        let policy = ShellPolicy::new(vec![], true);
        let calls = vec![shell_call("1", "ls"), shell_call("2", "git status")];
        let verdict = resolve_shell_approvals(&calls, &policy, None).await.unwrap();
        assert_eq!(verdict, None);
    }

    #[tokio::test]
    async fn approvals_batch_two_commands_one_prompt() {
        let policy = ShellPolicy::new(vec![], true);
        let calls = vec![
            shell_call("1", "rm target/x"),
            shell_call("2", "git status"), // read-only: not in the prompt
            shell_call("3", "mv a b"),
        ];
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let approve = tokio::spawn(async move {
            match rx.recv().await.unwrap() {
                AgentEvent::CommandApproval(req) => {
                    assert_eq!(req.commands, vec!["rm target/x", "mv a b"]);
                    let _ = req.reply.send(true);
                }
                _ => panic!("expected CommandApproval"),
            }
        });
        let verdict = resolve_shell_approvals(&calls, &policy, Some(&tx))
            .await
            .unwrap();
        assert_eq!(verdict, Some(true));
        approve.await.unwrap();
    }

    fn text_reply(text: &str) -> crate::provider::ModelReply {
        (
            crate::chat::ApiMessage {
                role: crate::chat::ApiRole::Assistant,
                content: Some(serde_json::Value::String(text.into())),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            Some("stop".into()),
            None,
        )
    }

    #[tokio::test]
    async fn spawn_agent_runs_child_and_returns_reply() {
        use crate::provider::MockModel;
        let tool_call_msg = crate::chat::ApiMessage {
            role: crate::chat::ApiRole::Assistant,
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "s1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "spawn_agent".into(),
                    arguments: r#"{"prompt":"child task"}"#.into(),
                },
            }]),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        };
        let mock = std::sync::Arc::new(MockModel::new(
            "m",
            vec![
                Ok((tool_call_msg, Some("tool_calls".into()), None)),
                Ok(text_reply("child answer")),
                Ok(text_reply("parent final")),
            ],
        ));
        let agent = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::agent::Agent::new_shared(
                mock.clone(),
                std::sync::Arc::from(std::path::Path::new(".")),
                ShellPolicy::default(),
                0,
            ),
        ));
        let (_tx, rx) = watch::channel(false);
        let reply =
            crate::agent::Agent::run_user_turn_shared(agent, "go".into(), |_| {}, rx, None)
                .await
                .unwrap();
        assert_eq!(reply, "parent final");
        let reqs = mock.requests.lock().unwrap();
        assert_eq!(reqs.len(), 3);
        // Request #2 is the child's — its user message carries the delegated prompt.
        assert!(reqs[1].messages.iter().any(|m| {
            serde_json::to_string(&m.content)
                .unwrap_or_default()
                .contains("child task")
        }));
    }

    #[tokio::test]
    async fn spawn_agent_inherits_fallback_client() {
        use crate::provider::MockModel;
        let tool_call_msg = crate::chat::ApiMessage {
            role: crate::chat::ApiRole::Assistant,
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "s1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "spawn_agent".into(),
                    arguments: r#"{"prompt":"child task"}"#.into(),
                },
            }]),
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        };
        // Primary serves: parent's spawn call, an ERROR for the child's first
        // request, then the parent's final reply.
        let primary = std::sync::Arc::new(MockModel::new(
            "m",
            vec![
                Ok((tool_call_msg, Some("tool_calls".into()), None)),
                Err(anyhow::anyhow!("primary down")),
                Ok(text_reply("parent final")),
            ],
        ));
        // Fallback serves the child's rescued request — only reachable if the
        // child actually inherited it.
        let fallback = std::sync::Arc::new(MockModel::text("fb", "child answer"));
        let parent = {
            let mut a = crate::agent::Agent::new_shared(
                primary.clone(),
                std::sync::Arc::from(std::path::Path::new(".")),
                ShellPolicy::default(),
                0,
            );
            a.fallback_client = Some(fallback.clone());
            std::sync::Arc::new(tokio::sync::Mutex::new(a))
        };
        let (_tx, rx) = watch::channel(false);
        let reply =
            crate::agent::Agent::run_user_turn_shared(parent, "go".into(), |_| {}, rx, None)
                .await
                .unwrap();
        assert_eq!(reply, "parent final");
        assert_eq!(
            fallback.requests.lock().unwrap().len(),
            1,
            "child turn must fall back to the inherited client"
        );
    }

    #[tokio::test]
    async fn spawn_agent_denied_at_depth_one() {
        use crate::provider::MockModel;
        let mock = std::sync::Arc::new(MockModel::text("m", "x"));
        let agent = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::agent::Agent::new_shared(
                mock,
                std::sync::Arc::from(std::path::Path::new(".")),
                ShellPolicy::default(),
                1,
            ),
        ));
        let (_tx, rx) = watch::channel(false);
        let out =
            spawn_agent_outcome(&agent, std::path::Path::new("."), r#"{"prompt":"x"}"#, &rx).await;
        assert!(out.body.contains("depth limit"), "{}", out.body);
    }
}
