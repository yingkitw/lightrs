use crate::agent::{Agent, context_usage_text};
use crate::session;
use crate::tui::App;
use crate::tui::info::{context_details, session_details};

/// Built-in TUI commands (leading `/`); never sent to the model.
const COMMANDS: &[(&str, &str)] = &[
    ("help", "list slash commands"),
    ("clear", "reset conversation"),
    ("cwd", "show working directory in transcript"),
    ("quit", "exit (same as q)"),
    ("status", "show model, path, and context"),
    ("context", "message count and token usage"),
    ("reload", "re-read config.toml [shell] policy"),
    ("quiet", "toggle per-step token sys lines"),
    ("skills", "list discovered agent skills"),
    ("export", "write UI transcript to markdown in workdir"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashAction {
    Handled,
    Quit,
}

pub fn is_slash_input(input: &str) -> bool {
    input.starts_with('/')
}

/// Comma-separated hint for the status bar while typing a slash command.
pub fn slash_command_hint(input: &str) -> Option<String> {
    if !is_slash_input(input) {
        return None;
    }
    let prefix = input.get(1..).unwrap_or("").split_whitespace().next().unwrap_or("");
    let mut parts: Vec<String> = COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(name, desc)| format!("/{name} — {desc}"))
        .collect();
    if parts.is_empty() {
        Some("unknown slash command — try /help".into())
    } else if parts.len() <= 3 {
        Some(parts.join(" · "))
    } else {
        parts.truncate(3);
        Some(format!("{} · …", parts.join(" · ")))
    }
}

/// Tab completes when there is a unique matching command name.
pub fn tab_complete(input: &str) -> Option<String> {
    if !input.starts_with('/') {
        return None;
    }
    let rest = input.get(1..).unwrap_or("");
    let prefix = rest.split_whitespace().next().unwrap_or(rest);
    let matches: Vec<_> = COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .collect();
    if matches.len() == 1 {
        Some(format!("/{}", matches[0].0))
    } else if matches.len() > 1 && !prefix.is_empty() {
        let mut common = matches[0].0.to_string();
        for (name, _) in matches.iter().skip(1) {
            common = common_prefix(&common, name);
        }
        if common.len() > prefix.len() {
            Some(format!("/{common}"))
        } else {
            None
        }
    } else {
        None
    }
}

fn common_prefix(a: &str, b: &str) -> String {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c)
        .collect()
}

/// Run a full input line that starts with `/`.
pub fn run_slash(line: &str, agent: &mut Agent, app: &mut App) -> SlashAction {
    let name = line
        .trim()
        .strip_prefix('/')
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("");

    match name {
        "" => {
            agent.push_system(slash_help_text());
            SlashAction::Handled
        }
        "help" | "?" => {
            agent.push_system(slash_help_text());
            SlashAction::Handled
        }
        "clear" => {
            agent.clear_conversation();
            app.clear_tool_expansion();
            let _ = session::remove(app.workdir());
            SlashAction::Handled
        }
        "cwd" => {
            let wd = app.workdir().display().to_string();
            agent.push_system(format!("cwd: {wd}"));
            SlashAction::Handled
        }
        "status" => {
            let n = agent.api_context_len();
            agent.push_system(session_details(&app.config, n, "status"));
            SlashAction::Handled
        }
        "context" => {
            let n = agent.api_context_len();
            let usage = context_usage_text(agent.token_summary(), n);
            agent.push_system(context_details(&app.config, n, &usage, "context"));
            SlashAction::Handled
        }
        "quiet" => {
            let on = !agent.quiet_token_sys();
            agent.set_quiet_token_sys(on);
            let state = if on { "hidden" } else { "shown" };
            agent.push_system(format!(
                "per-step token lines: {state} (header and /context unchanged; /quiet toggles)"
            ));
            SlashAction::Handled
        }
        "skills" => {
            app.refresh_skill_catalog(agent);
            let n = agent.skill_catalog().skills.len();
            if n == 0 {
                agent.push_system(
                    "No skills found. Paths: ~/.config/lightrs/skills, ~/.cursor/skills, ~/.claude/skills, ~/.agents/skills, ~/.pi/agent/skills, ~/.codex/skills — or `lightrs import opencode`.",
                );
            } else {
                let mut lines: Vec<String> = agent
                    .skill_catalog()
                    .skills
                    .iter()
                    .map(|s| format!("  ${} — {}", s.id, s.description))
                    .collect();
                lines.truncate(24);
                let extra = if agent.skill_catalog().skills.len() > 24 {
                    "\n  … (use /skills after adding more)"
                } else {
                    ""
                };
                agent.push_system(format!(
                    "{n} skill(s). Invoke with $id on your message:\n{}{extra}",
                    lines.join("\n")
                ));
            }
            SlashAction::Handled
        }
        "export" => {
            let rest = line.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
            let rel = if rest.is_empty() {
                None
            } else {
                Some(rest.as_str())
            };
            match session::export_ui_markdown(agent, app.workdir(), rel) {
                Ok(path) => {
                    agent.push_system(format!(
                        "exported transcript → {}",
                        path.strip_prefix(app.workdir())
                            .unwrap_or(&path)
                            .display()
                    ));
                }
                Err(e) => agent.push_error(format!("export failed: {e:#}")),
            }
            SlashAction::Handled
        }
        "reload" => match app.config.reload_shell_from_file() {
            Ok(msg) => {
                agent.set_shell_policy(app.config.shell_policy.clone());
                app.refresh_skill_catalog(agent);
                agent.push_system(format!(
                    "{msg}\n(model/provider/base URL still need restart; skills rescanned)"
                ));
                SlashAction::Handled
            }
            Err(e) => {
                agent.push_error(format!("reload failed: {e:#}"));
                SlashAction::Handled
            }
        },
        "quit" | "exit" => SlashAction::Quit,
        other => {
            agent.push_system(format!(
                "unknown slash command `/{other}`. Type /help for built-ins."
            ));
            SlashAction::Handled
        }
    }
}

fn slash_help_text() -> String {
    let lines: Vec<String> = COMMANDS
        .iter()
        .map(|(name, desc)| format!("  /{name} — {desc}"))
        .collect();
    format!("Slash commands:\n{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_slash_prefix() {
        assert!(is_slash_input("/clear"));
        assert!(!is_slash_input("hello"));
    }

    #[test]
    fn tab_unique_complete() {
        assert_eq!(tab_complete("/cl").as_deref(), Some("/clear"));
    }

    #[test]
    fn quiet_listed_for_docs_sync() {
        assert!(COMMANDS.iter().any(|(n, _)| *n == "quiet"));
    }
}