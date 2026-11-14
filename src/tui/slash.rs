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
pub fn run_slash(line: &str, agent: &mut Agent, app: &App) -> SlashAction {
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
}