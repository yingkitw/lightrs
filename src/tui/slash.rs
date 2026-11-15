use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::agent::{Agent, context_usage_text};
use crate::commands::SlashDef;
use crate::session;
use crate::tui::clipboard::{copy_payload_from_markdown, set_text};
use crate::tui::theme;
use crate::tui::App;
use crate::tui::info::context_details;

/// Built-in TUI commands (leading `/`); never sent to the model.
const COMMANDS: &[(&str, &str)] = &[
    ("help", "list slash commands"),
    ("clear", "reset conversation"),
    ("cwd", "show working directory in transcript"),
    ("cd", "change working directory (/cd <path>)"),
    ("todo", "show the task list (.lightrs/todos.json)"),
    ("editor", "compose the prompt in $EDITOR"),
    ("quit", "exit (same as q)"),
    ("status", "show model, path, and context"),
    ("context", "context breakdown and token usage"),
    ("reload", "re-read config.toml [shell] policy"),
    ("quiet", "toggle per-step token sys lines"),
    ("skills", "list discovered agent skills"),
    ("export", "write UI transcript to markdown in workdir"),
    ("copy", "copy last reply (or last ``` fence with /copy code) to clipboard"),
    ("theme", "list or set TUI color theme (mocha, hc)"),
    ("compact", "compact old tool results in API history (no model call)"),
    ("rewind", "drop the last n user turns from history (/rewind [n])"),
    ("undo", "restore files edited on the last turn from .lightrs/backups"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashAction {
    Handled,
    /// `/editor` — suspend the TUI and open `$EDITOR`.
    Editor,
    /// A `.claude/commands/*.md` command expanded to a model prompt.
    Prompt(String),
    Quit,
}

pub fn is_slash_input(input: &str) -> bool {
    input.starts_with('/')
}

/// Comma-separated hint for the status bar while typing a slash command.
pub fn slash_command_hint(input: &str, commands: &[SlashDef]) -> Option<String> {
    if !is_slash_input(input) {
        return None;
    }
    let prefix = input.get(1..).unwrap_or("").split_whitespace().next().unwrap_or("");
    let mut parts: Vec<String> = COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(name, desc)| format!("/{name} — {desc}"))
        .collect();
    // Project commands can't shadow built-ins — skip colliding names.
    parts.extend(
        commands
            .iter()
            .filter(|c| {
                c.name.starts_with(prefix) && !COMMANDS.iter().any(|(n, _)| *n == c.name)
            })
            .map(|c| format!("/{} — {}", c.name, c.description)),
    );
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
pub fn tab_complete(input: &str, commands: &[SlashDef]) -> Option<String> {
    if !input.starts_with('/') {
        return None;
    }
    let rest = input.get(1..).unwrap_or("");
    let prefix = rest.split_whitespace().next().unwrap_or(rest);
    let mut names: Vec<&str> = COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(name, _)| *name)
        .collect();
    names.extend(
        commands
            .iter()
            .filter(|c| {
                c.name.starts_with(prefix) && !COMMANDS.iter().any(|(n, _)| *n == c.name)
            })
            .map(|c| c.name.as_str()),
    );
    if names.len() == 1 {
        Some(format!("/{}", names[0]))
    } else if names.len() > 1 && !prefix.is_empty() {
        let mut common = names[0].to_string();
        for name in names.iter().skip(1) {
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
        "cd" => {
            let arg = line.split_whitespace().nth(1).unwrap_or("");
            if arg.is_empty() {
                let wd = app.workdir().display().to_string();
                agent.push_system(format!("cwd: {wd}"));
                return SlashAction::Handled;
            }
            match resolve_cd_target(app.workdir(), arg) {
                Ok(dir) => {
                    let _ = session::save(agent, app.workdir());
                    app.config.workdir = dir.clone();
                    agent.set_workdir(std::sync::Arc::from(dir.as_path()));
                    // Pick up the new workdir's .lightrs/config.toml overlay.
                    let reload = app.config.reload_shell_from_file().ok();
                    agent.set_shell_policy(app.config.shell_policy.clone());
                    sync_heal(agent, app);
                    agent.push_system(format!("cwd → {}", dir.display()));
                    if let Some(msg) = reload
                        && msg.contains("project")
                    {
                        agent.push_system(msg);
                    }
                }
                Err(e) => agent.push_error(format!("{e:#}")),
            }
            SlashAction::Handled
        }
        "todo" => {
            let items = crate::tools::todo::load(app.workdir());
            if items.is_empty() {
                agent.push_system("no todos — the model can set them with todo_write");
            } else {
                agent.push_system(crate::tools::todo::render(&items));
            }
            SlashAction::Handled
        }
        "editor" => SlashAction::Editor,
        "status" => {
            let n = agent.api_context_len();
            let usage = context_usage_text(agent.token_summary(), n);
            agent.push_system(context_details(&app.config, n, &usage, "status"));
            SlashAction::Handled
        }
        "context" => {
            let n = agent.api_context_len();
            let usage = context_usage_text(agent.token_summary(), n);
            let mut msg = context_details(&app.config, n, &usage, "context");
            msg.push('\n');
            msg.push_str(&agent.context_report());
            agent.push_system(msg);
            SlashAction::Handled
        }
        "quiet" => {
            let on = !agent.quiet_token_sys();
            agent.set_quiet_token_sys(on);
            let state = if on { "hidden" } else { "shown" };
            agent.push_system(format!(
                "token sys lines (send/model/turn): {state} (header and /context unchanged; /quiet toggles)"
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
        "theme" => {
            let args: Vec<&str> = line.split_whitespace().skip(1).collect();
            if args.is_empty() {
                agent.push_system(theme::list_message(&app.theme));
            } else {
                match theme::apply(args[0], app) {
                    Ok(id) => agent.push_system(format!(
                        "theme set to `{id}` ([tui] in config.toml)"
                    )),
                    Err(e) => agent.push_error(e),
                }
            }
            SlashAction::Handled
        }
        "copy" => {
            let code_only = line
                .split_whitespace()
                .nth(1)
                .is_some_and(|a| a == "code");
            let Some(markdown) = agent.last_assistant_text() else {
                agent.push_system("nothing to copy — no assistant reply yet");
                return SlashAction::Handled;
            };
            let payload = copy_payload_from_markdown(&markdown, code_only);
            if payload.trim().is_empty() {
                agent.push_system(if code_only {
                    "no fenced ``` block in the latest reply"
                } else {
                    "latest reply is empty"
                });
                return SlashAction::Handled;
            }
            match set_text(&payload) {
                Ok(()) => agent.push_system(format!(
                    "copied {} byte(s) to clipboard{}",
                    payload.len(),
                    if code_only { " (last code fence)" } else { "" }
                )),
                Err(e) => agent.push_error(format!("copy failed: {e:#}")),
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
        "compact" => {
            let n = agent.compact_history();
            if n == 0 {
                agent.push_system("nothing to compact — tool history already short");
            } else {
                let est = agent.token_summary().estimate;
                agent.push_system(format!(
                    "compacted {n} older tool result(s); ~{est} tokens in context now"
                ));
            }
            SlashAction::Handled
        }
        "rewind" => {
            let n: usize = line
                .split_whitespace()
                .nth(1)
                .and_then(|a| a.parse().ok())
                .unwrap_or(1);
            let dropped = agent.rewind_turns(n);
            if dropped == 0 {
                agent.push_system("nothing to rewind");
            } else {
                let _ = session::save(agent, app.workdir());
                agent.push_system(format!(
                    "rewound {dropped} turn(s) — history and session file updated"
                ));
            }
            SlashAction::Handled
        }
        "undo" => {
            let ui = agent.ui_messages();
            match crate::session::undo_last_turn(app.workdir(), &ui) {
                Ok(notes) => agent.push_system(notes.join("\n")),
                Err(e) => agent.push_error(format!("undo failed: {e:#}")),
            }
            SlashAction::Handled
        }
        "reload" => match app.config.reload_shell_from_file() {
            Ok(msg) => {
                agent.set_shell_policy(app.config.shell_policy.clone());
                sync_heal(agent, app);
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
            // Project commands from `.claude/commands/*.md` — rescan so a
            // file added this session works without /reload.
            app.commands = crate::commands::load(app.workdir());
            if let Some(def) = app.commands.iter().find(|c| c.name == other) {
                let args = line.split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
                return SlashAction::Prompt(crate::commands::expand(def, &args));
            }
            agent.push_system(format!(
                "unknown slash command `/{other}`. Type /help for built-ins."
            ));
            SlashAction::Handled
        }
    }
}

/// Keep `agent.heal` in sync with the (possibly project-overlaid) `[shell]` config.
fn sync_heal(agent: &mut Agent, app: &App) {
    agent.set_heal(app.config.test_command.clone().map(|command| {
        crate::agent::HealSetting {
            command,
            max_steps: app.config.heal_steps,
        }
    }));
}

/// `/cd` target: `~`, `~/x`, absolute, or relative to the current workdir. Must be a directory.
fn resolve_cd_target(cwd: &Path, arg: &str) -> Result<PathBuf> {
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf())
    };
    let expanded = if arg == "~" {
        home()
    } else if let Some(rest) = arg.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(arg)
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let canon = joined
        .canonicalize()
        .with_context(|| format!("cd: cannot resolve {arg}"))?;
    if !canon.is_dir() {
        bail!("cd: not a directory: {arg}");
    }
    Ok(canon)
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
        assert_eq!(tab_complete("/cl", &[]).as_deref(), Some("/clear"));
    }

    #[test]
    fn tab_complete_project_command() {
        let cmds = vec![crate::commands::SlashDef {
            name: "review".into(),
            description: String::new(),
            body: "body".into(),
        }];
        assert_eq!(tab_complete("/rev", &cmds).as_deref(), Some("/review"));
    }

    #[test]
    fn quiet_listed_for_docs_sync() {
        assert!(COMMANDS.iter().any(|(n, _)| *n == "quiet"));
    }

    #[test]
    fn theme_listed_for_docs_sync() {
        assert!(COMMANDS.iter().any(|(n, _)| *n == "theme"));
    }

    #[test]
    fn project_command_expands_to_prompt() {
        use std::sync::Arc;

        use crate::config::{AppConfig, Provider};
        use crate::provider::LlmClient;
        use crate::tools::ShellPolicy;

        let dir = std::env::temp_dir().join(format!("lightrs-slash-{}", std::process::id()));
        let cmd_dir = dir.join(".claude/commands");
        std::fs::create_dir_all(&cmd_dir).unwrap();
        std::fs::write(cmd_dir.join("review.md"), "Review the diff.").unwrap();

        let mut app = App::new(AppConfig {
            provider: Provider::Kimi,
            model: "m".into(),
            fallback_model: None,
            weak_model: None,
            base_url: "http://localhost".into(),
            api_key: String::new(),
            workdir: dir.clone(),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            heal_steps: 3,
        });
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let mut agent = Agent::new(
            client,
            Arc::from(dir.as_path()),
            ShellPolicy::default(),
        );

        let action = run_slash("/review src/lib.rs", &mut agent, &mut app);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            action,
            SlashAction::Prompt("Review the diff.\n\nsrc/lib.rs".into())
        );
    }
}