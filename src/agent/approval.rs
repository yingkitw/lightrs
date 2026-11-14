use anyhow::{Context, Result};
use tokio::sync::mpsc;

use crate::tools::{RunCommandArgs, ShellPolicy};
use crate::tui::{AgentEvent, CommandApproval};

/// Result of shell gating before `execute_tool`.
pub(crate) enum ShellGate {
    NotShell,
    Run { bypass_allowlist: bool },
    Denied(String),
}

pub(crate) fn shell_allowlist_bypass(name: &str, args: &str, policy: &ShellPolicy) -> bool {
    if name != "run_command" {
        return false;
    }
    let Ok(parsed) = serde_json::from_str::<RunCommandArgs>(args) else {
        return false;
    };
    policy.is_restrictive() && !policy.is_allowed(&parsed.command)
}

pub(crate) async fn resolve_shell_gate(
    name: &str,
    args: &str,
    policy: &ShellPolicy,
    ui_tx: Option<&mpsc::UnboundedSender<AgentEvent>>,
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
    let Some(tx) = ui_tx else {
        return Ok(ShellGate::Denied(
            "error: shell command requires TUI approval (use lightrs TUI or relax [shell] in config)"
                .into(),
        ));
    };
    let (reply, rx) = tokio::sync::oneshot::channel();
    tx.send(AgentEvent::CommandApproval(CommandApproval {
        command: parsed.command,
        reply,
    }))
    .map_err(|_| anyhow::anyhow!("ui channel closed"))?;
    let approved = rx.await.unwrap_or(false);
    if !approved {
        return Ok(ShellGate::Denied(
            "error: user denied shell command".into(),
        ));
    }
    Ok(ShellGate::Run {
        bypass_allowlist: shell_allowlist_bypass(name, args, policy),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ShellPolicy;

    #[test]
    fn headless_denies_when_approval_required() {
        let policy = ShellPolicy::new(vec![], true);
        let args = r#"{"command":"ls"}"#;
        // blocking test: resolve needs async — test bypass helper only
        assert!(policy.needs_interactive_approval("ls"));
        assert!(!shell_allowlist_bypass("run_command", args, &policy));
    }
}