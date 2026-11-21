use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::agent::{Agent, context_usage_text};
use crate::commands::SlashDef;
use crate::provider::LlmClient;
use crate::session;
use crate::tui::clipboard::{copy_payload_from_markdown, set_text};
use crate::tui::theme;
use crate::tui::App;
use crate::tui::info::{context_details, provider_name};

mod model;
pub(crate) use model::{format_model_list, model_choices_from};
use model::{resolve_model_args, start_model_fetch};

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
    ("model", "list/switch model — fetches /models per keyed provider"),
    ("context", "context breakdown and token usage"),
    ("reload", "re-read config.toml [shell] policy"),
    ("trust", "trust this workdir's .lightrs/config.toml (persisted)"),
    ("quiet", "toggle per-step token sys lines"),
    ("skills", "list discovered agent skills"),
    ("export", "write UI transcript to markdown in workdir"),
    ("copy", "copy last reply (or last ``` fence with /copy code) to clipboard"),
    ("diff", "replay this session's file changes as expandable diff rows"),
    ("theme", "list or set TUI color theme (mocha, hc)"),
    ("compact", "compact old tool results in API history (no model call)"),
    ("rewind", "drop the last n user turns into a restorable branch (/rewind [n])"),
    ("branch", "list or swap rewind branches back in (/branch [n])"),
    ("undo", "restore files edited on the last turn from .lightrs/backups"),
    ("plan", "toggle plan mode — read-only tools only (no writes/shell)"),
    ("fork", "checkpoint this session as a named fork (/fork [name])"),
    ("resume", "list sessions or restore one (/resume <n|name>)"),
    ("doctor", "run local environment checks"),
    ("init", "draft AGENTS.md for this project"),
];

/// `/init` — canned prompt, defined in `crate::commands` so the TUI and
/// the ACP prompt-expansion seam share one source of truth.
use crate::commands::INIT_PROMPT;

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
                    app.git_branch = crate::tui::info::git_branch_label(&dir);
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
            let usage = context_usage_text(
                agent.token_summary(),
                n,
                crate::config::model_context_window(&app.config.model),
            );
            let mut text = context_details(&app.config, n, &usage, "status");
            let changed: Vec<&String> = agent.changed_files().collect();
            if !changed.is_empty() {
                let names = changed
                    .iter()
                    .take(5)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let more = changed.len().saturating_sub(5);
                let suffix = if more > 0 { format!(", +{more} more") } else { String::new() };
                text.push_str(&format!(
                    "\nfiles changed: {} ({names}{suffix})",
                    changed.len()
                ));
            }
            agent.push_system(text);
            SlashAction::Handled
        }
        "model" => {
            let args: Vec<&str> = line.split_whitespace().skip(1).collect();
            if args.is_empty() {
                match start_model_fetch(app) {
                    Some(note) => agent.push_system(note),
                    None => agent.push_system(format!(
                        "model: {} · {} · {}\n/model <name> · /model <provider> [name] — providers: glm, kimi, minimax",
                        provider_name(app.config.provider),
                        app.config.model,
                        app.config.base_url
                    )),
                }
                return SlashAction::Handled;
            }
            match resolve_model_args(&args, app) {
                Ok(Some(sw)) => {
                    agent.set_client(LlmClient::new(
                        sw.base_url.clone(),
                        sw.api_key.clone(),
                        sw.model.clone(),
                    ));
                    app.config.provider = sw.provider;
                    app.config.model = sw.model.clone();
                    app.config.base_url = sw.base_url.clone();
                    app.config.api_key = sw.api_key;
                    // Unit tests share the dev's real config.toml — skip the
                    // write under cfg(test); `save_model_selection_to` covers
                    // the file logic and integration binaries persist for real.
                    let saved = if cfg!(test) {
                        ""
                    } else {
                        match crate::config::save_model_selection(
                            sw.provider,
                            &sw.model,
                            &sw.base_url,
                        ) {
                            Ok(()) => " · saved (restored next launch)",
                            Err(e) => {
                                agent
                                    .push_error(format!("could not save model selection: {e:#}"));
                                ""
                            }
                        }
                    };
                    agent.push_system(format!(
                        "model → {} · {} · {}{saved}",
                        provider_name(sw.provider),
                        sw.model,
                        sw.base_url
                    ));
                }
                Ok(None) => agent.push_system(format!(
                    "model: {} · {} · {}",
                    provider_name(app.config.provider),
                    app.config.model,
                    app.config.base_url
                )),
                Err(e) => agent.push_error(format!("{e:#}")),
            }
            SlashAction::Handled
        }
        "context" => {
            let n = agent.api_context_len();
            let usage = context_usage_text(
                agent.token_summary(),
                n,
                crate::config::model_context_window(&app.config.model),
            );
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
                // Expansion indices point into ui_history — dropped turns
                // leave dead entries (prefix indices stay valid since
                // rewind truncates the tail only).
                let len = agent.ui_messages().len();
                app.expanded_tools.retain(|i| *i < len);
                agent.push_system(format!(
                    "rewound {dropped} turn(s) — tail kept as a branch (/branch to restore)"
                ));
            }
            SlashAction::Handled
        }
        "branch" => {
            let arg = line.split_whitespace().nth(1);
            match arg {
                None => {
                    let branches = agent.branches();
                    if branches.is_empty() {
                        agent.push_system("no branches — /rewind keeps dropped tails here");
                    } else {
                        let mut out =
                            String::from("branches (tails detached by /rewind):");
                        for (i, b) in branches.iter().enumerate() {
                            let turns = b
                                .api_tail
                                .iter()
                                .filter(|m| m.role == crate::chat::ApiRole::User)
                                .count();
                            let preview = b
                                .api_tail
                                .iter()
                                .find_map(|m| m.text_content())
                                .map(|t| crate::tools::shell::escape_controls(&t.chars().take(60).collect::<String>()))
                                .unwrap_or_default();
                            out.push_str(&format!("\n  {}. {} turn(s) — {}", i + 1, turns, preview));
                        }
                        out.push_str("\n/branch <n> swaps the current tail in for that branch");
                        agent.push_system(out);
                    }
                }
                Some(a) => match a.parse::<usize>().ok().and_then(|n| n.checked_sub(1)) {
                    None => agent.push_system(format!("unknown branch '{a}' — /branch lists them")),
                    Some(i) => match agent.restore_branch(i) {
                        None => agent.push_system("no such branch — /branch lists them"),
                        Some(turns) => {
                            let _ = session::save(agent, app.workdir());
                            let len = agent.ui_messages().len();
                            app.expanded_tools.retain(|i| *i < len);
                            agent.push_system(format!(
                                "branch restored — {turns} user turn(s) now in history (replaced tail kept as a branch)"
                            ));
                        }
                    },
                },
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
        "plan" => {
            agent.set_read_only(!agent.read_only());
            SlashAction::Handled
        }
        "fork" => {
            let name = line.split_whitespace().nth(1).unwrap_or("fork");
            match session::fork(agent, app.workdir(), name) {
                Ok(file) => {
                    let slug = file.trim_end_matches(".json");
                    agent.push_system(format!("session forked → {slug} (/resume {slug} to restore)"));
                }
                Err(e) => agent.push_error(format!("fork failed: {e:#}")),
            }
            SlashAction::Handled
        }
        "resume" => {
            let arg = line.split_whitespace().nth(1).unwrap_or("");
            let metas = session::list();
            if arg.is_empty() {
                if metas.is_empty() {
                    agent.push_system("no saved sessions");
                } else {
                    let cwd = app.workdir();
                    let rows = metas
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            let here = if m.workdir == *cwd { "*" } else { " " };
                            format!(
                                "{here} {}. {} — {} user turns — {}",
                                i + 1,
                                m.file_name.trim_end_matches(".json"),
                                m.user_turns,
                                m.workdir.display()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    agent.push_system(format!("sessions (* = this workdir):\n{rows}\nresume with /resume <n>"));
                }
                return SlashAction::Handled;
            }
            // `/resume <n>` picks by list number; `/resume <name>` matches the
            // exact or fork-suffixed filename.
            let file_name = arg.parse::<usize>().ok().and_then(|n| {
                metas.get(n.wrapping_sub(1)).filter(|_| n >= 1).map(|m| m.file_name.clone())
            }).or_else(|| {
                let exact = format!("{arg}.json");
                metas
                    .iter()
                    .find(|m| m.file_name == exact || m.file_name.ends_with(&format!("-{exact}")))
                    .map(|m| m.file_name.clone())
            });
            let Some(file_name) = file_name else {
                agent.push_system(format!("no session matching {arg} — see /resume"));
                return SlashAction::Handled;
            };
            match session::load_named(&file_name) {
                Ok(Some(file)) => {
                    let _ = session::save(agent, app.workdir());
                    let foreign = file.workdir != *app.workdir();
                    if foreign {
                        let dir = file.workdir.clone();
                        app.config.workdir = dir.clone();
                        app.git_branch = crate::tui::info::git_branch_label(&dir);
                        let reload = app.config.reload_shell_from_file().ok();
                        agent.set_shell_policy(app.config.shell_policy.clone());
                        sync_heal(agent, app);
                        if let Some(msg) = reload
                            && msg.contains("project")
                        {
                            agent.push_system(msg);
                        }
                    }
                    let wd = file.workdir.clone();
                    agent.restore_session(file);
                    if foreign {
                        agent.set_workdir(std::sync::Arc::from(wd.as_path()));
                    }
                    agent.push_system(format!("resumed {}", file_name.trim_end_matches(".json")));
                }
                Ok(None) => agent.push_system(format!("no session named {file_name}")),
                Err(e) => agent.push_error(format!("resume failed: {e:#}")),
            }
            SlashAction::Handled
        }
        "doctor" => {
            match crate::doctor::report(&app.config) {
                Ok(text) => agent.push_system(text),
                Err(e) => agent.push_error(format!("doctor failed: {e:#}")),
            }
            SlashAction::Handled
        }
        "init" => SlashAction::Prompt(INIT_PROMPT.into()),
        "diff" => {
            let diffs = crate::session::collect_file_diffs(&agent.ui_messages());
            if diffs.is_empty() {
                agent.push_system("no file changes this session");
            } else {
                let files = diffs
                    .iter()
                    .map(|d| d.path.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                let adds: usize = diffs.iter().map(|d| d.lines_added).sum();
                let rems: usize = diffs.iter().map(|d| d.lines_removed).sum();
                agent.push_system(format!(
                    "session diff · {files} file(s) · +{adds} −{rems} across {} write(s) — expand rows below",
                    diffs.len()
                ));
                for fd in diffs {
                    agent.push_ui_message(crate::chat::UiMessage::tool_result(
                        "diff",
                        fd.path.clone(),
                        "stored write diff",
                        Some(fd),
                    ));
                }
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
        "trust" => {
            // cfg(test) gate: unit tests share the dev's real config.toml —
            // `save_trust_dir_to` covers the write logic.
            if !cfg!(test)
                && let Err(e) = crate::config::save_trust_dir(app.workdir())
            {
                agent.push_error(format!("trust failed: {e:#}"));
                return SlashAction::Handled;
            }
            agent.push_system(format!(
                "trusted {} — project .lightrs/config.toml applies next launch \
                 (/reload applies its [shell] now; model/mcp/lsp need restart)",
                app.workdir().display()
            ));
            SlashAction::Handled
        }
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
/// Target of `/model` — everything needed to rebuild the primary client.
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

        use crate::config::AppConfig;
        use crate::provider::LlmClient;
        use crate::tools::ShellPolicy;

        let dir = std::env::temp_dir().join(format!("lightrs-slash-{}", std::process::id()));
        let cmd_dir = dir.join(".claude/commands");
        std::fs::create_dir_all(&cmd_dir).unwrap();
        std::fs::write(cmd_dir.join("review.md"), "Review the diff.").unwrap();

        let mut app = App::new(AppConfig {
                                   model: "m".into(),
                                   workdir: dir.clone(),
                                   ..AppConfig::test()
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

    #[test]
    fn model_bare_shows_current() {
        let (dir, mut app, mut agent) = fixture::build("model-show");
        run_slash("/model", &mut agent, &mut app);
        let msg = last_ui_content(&agent);
        assert!(msg.contains("model: kimi · m") && msg.contains("providers:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_switches_same_provider_and_client() {
        let (dir, mut app, mut agent) = fixture::build("model-switch");
        run_slash("/model kimi-k2-thinking", &mut agent, &mut app);
        assert_eq!(app.config.model, "kimi-k2-thinking");
        assert_eq!(app.config.provider, crate::config::Provider::Kimi);
        assert_eq!(app.config.base_url, "http://localhost");
        assert_eq!(agent.client.model(), "kimi-k2-thinking");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_provider_keeps_base_when_same() {
        let (dir, mut app, mut agent) = fixture::build("model-prov");
        run_slash("/model kimi other-model", &mut agent, &mut app);
        assert_eq!(app.config.model, "other-model");
        assert_eq!(app.config.base_url, "http://localhost");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_number_picks_fetched_choice() {
        let (dir, mut app, mut agent) = fixture::build("model-num");
        app.model_choices = vec![
            (crate::config::Provider::Glm, "glm-5.3".into()),
            (crate::config::Provider::Kimi, "kimi-k2-0905-preview".into()),
        ];
        run_slash("/model 2", &mut agent, &mut app);
        assert_eq!(app.config.model, "kimi-k2-0905-preview");
        assert_eq!(app.config.provider, crate::config::Provider::Kimi);
        // Same provider keeps the session base_url + key.
        assert_eq!(app.config.base_url, "http://localhost");
        assert_eq!(agent.client.model(), "kimi-k2-0905-preview");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_number_without_fetched_list_errors() {
        let (dir, mut app, mut agent) = fixture::build("model-num-err");
        run_slash("/model 9", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("run /model first"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_list_formats_rows_and_choices() {
        let (_dir, app, _agent) = fixture::build("model-fmt");
        let entries = vec![
            (
                crate::config::Provider::Glm,
                Ok(vec!["glm-4.6".into(), "glm-5.3".into()]),
            ),
            (
                crate::config::Provider::Kimi,
                Ok(vec!["m".into()]),
            ),
            (
                crate::config::Provider::Minimax,
                Err("no key — set MINIMAX_API_KEY".into()),
            ),
        ];
        let text = format_model_list(&entries, &app.config);
        assert!(text.contains("1. glm-4.6 · glm"), "text: {text}");
        // Fixture model is "m" on provider Kimi → current marker on row 3.
        assert!(text.contains("3. m · kimi  ← current"), "text: {text}");
        assert!(text.contains("minimax: no key"), "text: {text}");
        assert!(text.contains("/model <n>"), "text: {text}");
        let choices = model_choices_from(&entries);
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[2].1, "m");
        let _ = std::fs::remove_dir_all(_dir);
    }

    #[test]
    fn model_rejects_extra_arg_and_unknown_provider_key() {
        let (dir, mut app, mut agent) = fixture::build("model-err");
        run_slash("/model k2 extra", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("unexpected argument"));

        // Different provider: either the env/config key exists (switch
        // succeeds with the provider default endpoint) or it fails fast
        // naming the missing key — never silently half-switches.
        run_slash("/model glm glm-4.6", &mut agent, &mut app);
        if app.config.provider == crate::config::Provider::Glm {
            assert_eq!(app.config.base_url, "https://api.z.ai/api/paas/v4");
            assert_eq!(agent.client.model(), "glm-4.6");
        } else {
            let msg = last_ui_content(&agent);
            assert!(msg.contains("ZAI_API_KEY") || msg.contains("GLM_API_KEY"));
            assert_eq!(app.config.provider, crate::config::Provider::Kimi);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    mod fixture {
        use std::path::PathBuf;
        use std::sync::Arc;

        use crate::agent::Agent;
        use crate::config::AppConfig;
        use crate::provider::LlmClient;
        use crate::tools::ShellPolicy;
        use crate::tui::App;

        pub(super) fn build(tag: &str) -> (PathBuf, App, Agent) {
            let dir = std::env::temp_dir()
                .join(format!("lightrs-slash-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let app = App::new(AppConfig {
                                   model: "m".into(),
                                   workdir: dir.clone(),
                                   ..AppConfig::test()
                               });
            let agent = Agent::new(
                LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into()),
                Arc::from(dir.as_path()),
                ShellPolicy::default(),
            );
            (dir, app, agent)
        }
    }

    fn last_ui_content(agent: &crate::agent::Agent) -> String {
        agent
            .ui_messages()
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default()
    }

    #[test]
    fn quit_action() {
        let (dir, mut app, mut agent) = fixture::build("quit");
        assert_eq!(run_slash("/quit", &mut agent, &mut app), SlashAction::Quit);
        assert_eq!(run_slash("/exit", &mut agent, &mut app), SlashAction::Quit);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_command_mentions_help() {
        let (dir, mut app, mut agent) = fixture::build("unk");
        assert_eq!(
            run_slash("/bogus", &mut agent, &mut app),
            SlashAction::Handled
        );
        assert!(last_ui_content(&agent).contains("unknown slash command"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_empty_session_notes_no_changes() {
        let (dir, mut app, mut agent) = fixture::build("diff0");
        assert_eq!(
            run_slash("/diff", &mut agent, &mut app),
            SlashAction::Handled
        );
        assert!(last_ui_content(&agent).contains("no file changes"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_replays_stored_write_diffs() {
        use crate::chat::{FileDiffUi, UiMessage};

        let (dir, mut app, mut agent) = fixture::build("diff1");
        agent.push_ui_message(UiMessage::tool_result(
            "patch_file",
            "a.rs",
            "ok",
            Some(FileDiffUi {
                path: "a.rs".into(),
                unified: "@@ x".into(),
                lines_added: 3,
                lines_removed: 1,
                is_new_file: false,
            }),
        ));
        assert_eq!(
            run_slash("/diff", &mut agent, &mut app),
            SlashAction::Handled
        );
        let ui = agent.ui_messages();
        let summary = &ui[ui.len() - 2];
        assert!(summary.content.contains("session diff · 1 file(s) · +3 −1"));
        let row = ui.last().unwrap();
        assert_eq!(row.tool_call.as_ref().unwrap().name, "diff");
        assert!(row.file_diff.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewind_empty_history_reports_nothing() {
        let (dir, mut app, mut agent) = fixture::build("rew");
        assert_eq!(
            run_slash("/rewind", &mut agent, &mut app),
            SlashAction::Handled
        );
        assert!(last_ui_content(&agent).contains("nothing to rewind"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewind_prunes_expanded_tool_indices() {
        let (dir, mut app, mut agent) = fixture::build("rew-exp");
        for i in 0..2 {
            agent
                .api_history
                .push(crate::chat::ApiMessage::user(format!("u{i}")));
            agent
                .api_history
                .push(crate::chat::ApiMessage::tool_result(
                    format!("t{i}"),
                    "read_file",
                    "b",
                ));
        }
        for i in 0..3 {
            agent.push_ui_message(crate::chat::UiMessage::with_role(
                if i == 2 {
                    crate::chat::UiRole::User
                } else {
                    crate::chat::UiRole::Tool
                },
                format!("m{i}"),
            ));
        }
        // Expand index 1 (survives) and 2 (dropped by the rewind).
        app.expanded_tools.insert(1);
        app.expanded_tools.insert(2);
        assert_eq!(
            run_slash("/rewind 1", &mut agent, &mut app),
            SlashAction::Handled
        );
        let len = agent.ui_messages().len();
        assert!(app.expanded_tools.contains(&1));
        assert!(app.expanded_tools.iter().all(|i| *i < len));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_without_edits_reports_error() {
        let (dir, mut app, mut agent) = fixture::build("undo");
        agent.push_ui_message(crate::chat::UiMessage::with_role(
            crate::chat::UiRole::User,
            "hi",
        ));
        assert_eq!(
            run_slash("/undo", &mut agent, &mut app),
            SlashAction::Handled
        );
        assert!(last_ui_content(&agent).contains("undo failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cd_to_missing_dir_reports_error() {
        let (dir, mut app, mut agent) = fixture::build("cd");
        assert_eq!(
            run_slash("/cd /nonexistent-path-xyz", &mut agent, &mut app),
            SlashAction::Handled
        );
        let ui = agent.ui_messages();
        assert!(ui.iter().any(|m| m.content.contains("cd:")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn help_lists_builtin_commands() {
        for line in ["/help", "/?"] {
            let (dir, mut app, mut agent) = fixture::build("help");
            assert_eq!(run_slash(line, &mut agent, &mut app), SlashAction::Handled);
            let msg = last_ui_content(&agent);
            for cmd in ["/clear", "/model", "/diff", "/undo", "/quit"] {
                assert!(msg.contains(cmd), "help missing {cmd}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn cwd_prints_workdir() {
        let (dir, mut app, mut agent) = fixture::build("cwd");
        run_slash("/cwd", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("cwd: "));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cd_success_updates_workdir_and_agent() {
        let (dir, mut app, mut agent) = fixture::build("cdok");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        run_slash(&format!("/cd {}", sub.display()), &mut agent, &mut app);
        assert_eq!(
            app.config.workdir,
            sub.canonicalize().unwrap()
        );
        assert!(last_ui_content(&agent).contains("cwd →"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn todo_empty_and_rendered() {
        let (dir, mut app, mut agent) = fixture::build("todo");
        run_slash("/todo", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("no todos"));

        std::fs::create_dir_all(dir.join(".lightrs")).unwrap();
        std::fs::write(
            dir.join(".lightrs/todos.json"),
            r#"[{"content":"ship it","status":"in_progress"}]"#,
        )
        .unwrap();
        run_slash("/todo", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("ship it"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_and_context_print_details() {
        let (dir, mut app, mut agent) = fixture::build("stat");
        run_slash("/status", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("provider: kimi"));
        run_slash("/context", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("limits:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_lists_changed_files() {
        let (dir, mut app, mut agent) = fixture::build("statfiles");
        run_slash("/status", &mut agent, &mut app);
        assert!(
            !last_ui_content(&agent).contains("files changed"),
            "no files yet — line must not appear"
        );
        agent.changed_files.insert("src/a.rs".into());
        agent.changed_files.insert("src/b.rs".into());
        run_slash("/status", &mut agent, &mut app);
        let text = last_ui_content(&agent);
        assert!(
            text.contains("files changed: 2") && text.contains("src/a.rs, src/b.rs"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quiet_toggles_and_reports() {
        let (dir, mut app, mut agent) = fixture::build("quiet");
        run_slash("/quiet", &mut agent, &mut app);
        assert!(agent.quiet_token_sys());
        assert!(last_ui_content(&agent).contains("hidden"));
        run_slash("/quiet", &mut agent, &mut app);
        assert!(!agent.quiet_token_sys());
        assert!(last_ui_content(&agent).contains("shown"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_resets_conversation() {
        let (dir, mut app, mut agent) = fixture::build("clear");
        agent.push_ui_message(crate::chat::UiMessage::with_role(
            crate::chat::UiRole::User,
            "hi",
        ));
        agent.push_ui_message(crate::chat::UiMessage::with_role(
            crate::chat::UiRole::Assistant,
            "hello",
        ));
        run_slash("/clear", &mut agent, &mut app);
        let ui = agent.ui_messages();
        assert!(ui.iter().all(|m| m.role == crate::chat::UiRole::System));
        assert!(ui.iter().any(|m| m.content.contains("Conversation cleared")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_reports_count_or_nothing() {
        use crate::chat::ApiMessage;

        let (dir, mut app, mut agent) = fixture::build("compact");
        run_slash("/compact", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("nothing to compact"));

        for i in 0..8 {
            agent
                .api_history
                .push(ApiMessage::tool_result(format!("id{i}"), "read_file", "x".repeat(400)));
        }
        run_slash("/compact", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("compacted"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewind_drops_last_user_turn() {
        use crate::chat::ApiMessage;

        let (dir, mut app, mut agent) = fixture::build("rew2");
        agent.api_history.push(ApiMessage::user("turn one"));
        agent.api_history.push(ApiMessage::user("turn two"));
        run_slash("/rewind", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("rewound"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_restores_file_from_backup() {
        use crate::chat::{FileDiffUi, UiMessage};

        let (dir, mut app, mut agent) = fixture::build("undo2");
        std::fs::create_dir_all(dir.join(".lightrs/backups")).unwrap();
        std::fs::write(dir.join(".lightrs/backups/a.rs.1700000000.bak"), "old body").unwrap();
        std::fs::write(dir.join("a.rs"), "new body").unwrap();
        agent.push_ui_message(UiMessage::with_role(crate::chat::UiRole::User, "go"));
        agent.push_ui_message(UiMessage::tool_result(
            "write_file",
            "a.rs",
            "ok",
            Some(FileDiffUi {
                path: "a.rs".into(),
                unified: "@@ x".into(),
                lines_added: 1,
                lines_removed: 1,
                is_new_file: false,
            }),
        ));
        run_slash("/undo", &mut agent, &mut app);
        assert_eq!(
            std::fs::read_to_string(dir.join("a.rs")).unwrap(),
            "old body"
        );
        assert!(last_ui_content(&agent).contains("restored"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_writes_markdown_under_workdir() {
        use crate::chat::{UiMessage, UiRole};

        let (dir, mut app, mut agent) = fixture::build("export");
        agent.push_ui_message(UiMessage::with_role(UiRole::User, "question"));
        agent.push_ui_message(UiMessage::with_role(UiRole::Assistant, "answer"));
        run_slash("/export", &mut agent, &mut app);
        let msg = last_ui_content(&agent);
        assert!(msg.contains("exported"), "{msg}");
        let out = dir.join("lightrs-transcript.md");
        assert!(out.is_file(), "export path from {msg}");
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("question") && text.contains("answer"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_lists_and_rejects_unknown() {
        let (dir, mut app, mut agent) = fixture::build("theme");
        run_slash("/theme", &mut agent, &mut app);
        let msg = last_ui_content(&agent);
        assert!(msg.contains("mocha") && msg.contains("hc"), "{msg}");

        run_slash("/theme bogus", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("unknown theme"));
        // `/theme <id>` writes config.toml — deliberately not exercised here.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_without_reply_reports_nothing() {
        let (dir, mut app, mut agent) = fixture::build("copy");
        run_slash("/copy", &mut agent, &mut app);
        assert!(last_ui_content(&agent).contains("nothing to copy"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skills_reports_catalog_or_empty() {
        let (dir, mut app, mut agent) = fixture::build("skills");
        run_slash("/skills", &mut agent, &mut app);
        let msg = last_ui_content(&agent);
        // Ambient skill dirs differ per machine — both branches are valid.
        assert!(msg.contains("skill"), "{msg}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn plan_toggles_read_only() {
        let (dir, mut app, mut agent) = fixture::build("plan");
        assert!(!agent.read_only());
        run_slash("/plan", &mut agent, &mut app);
        assert!(agent.read_only(), "first /plan enables plan mode");
        assert!(last_ui_content(&agent).contains("Plan mode ON"));
        run_slash("/plan", &mut agent, &mut app);
        assert!(!agent.read_only(), "second /plan disables it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_returns_canned_prompt() {
        let (dir, mut app, mut agent) = fixture::build("init");
        let action = run_slash("/init", &mut agent, &mut app);
        let SlashAction::Prompt(p) = action else {
            panic!("/init should return a prompt, got {action:?}")
        };
        assert!(p.contains("AGENTS.md"), "{p}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn doctor_pushes_report() {
        let (dir, mut app, mut agent) = fixture::build("doctor");
        let action = run_slash("/doctor", &mut agent, &mut app);
        assert_eq!(action, SlashAction::Handled);
        assert!(last_ui_content(&agent).contains("provider:"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}