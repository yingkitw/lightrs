use anyhow::{Context, Result};
use tokio::sync::mpsc;

use crate::chat::ToolCall;
use crate::tools::{RunCommandArgs, ShellPolicy};
use crate::tui::{AgentEvent, CommandApproval};

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
}
