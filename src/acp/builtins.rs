//! ACP-safe built-in slash commands — carried over the wire so clients get
//! the same `/` surface as the TUI.
//!
//! The TUI keeps commands that need a terminal (`/editor`, `/theme`, `/copy`,
//! `/export`) or the session lifecycle a client owns (`/resume`, `/cd`,
//! `/rewind`/`/branch`) local. The rest run against agent + session state
//! only: their reply text ships as one `agent_message_chunk` and the prompt
//! round ends immediately (no model turn). `/init` is the exception — it
//! expands into a canned prompt in [`crate::acp::engine::expand_slash`] and
//! runs as a normal turn.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use clap::ValueEnum;
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::config::{AppConfig, Provider};

/// One built-in command carried over ACP.
pub struct Builtin {
    pub name: &'static str,
    pub description: &'static str,
    /// `true` → expands into a model prompt (`/init`); `false` → handled
    /// agent-side without a turn ([`run`]).
    pub prompt: bool,
}

const fn builtin(name: &'static str, description: &'static str) -> Builtin {
    Builtin { name, description, prompt: false }
}

/// Advertised after `session/new` alongside `.claude/commands/*.md` commands.
/// Keep sorted by name; every non-prompt entry must have an arm in [`run`] —
/// `carried_builtins_are_sorted_and_dispatchable` enforces it.
pub const BUILTINS: &[Builtin] = &[
    builtin("clear", "wipe the agent's context — next prompt starts fresh"),
    builtin("compact", "shrink context by dropping older tool results"),
    builtin("context", "context window report"),
    builtin("cwd", "show the session's working directory"),
    builtin("diff", "summarize file changes made this session"),
    builtin("doctor", "run local environment checks"),
    builtin("fork", "checkpoint this session as a named fork (/fork [name])"),
    Builtin {
        name: "init",
        description: "draft AGENTS.md for this project",
        prompt: true,
    },
    builtin("model", "show or switch the model (/model [name|provider [name]])"),
    builtin("skills", "list installed skills ($id to invoke)"),
    builtin("status", "session + context summary"),
    builtin("todo", "dump the session todo list"),
    builtin("undo", "restore files edited on the last turn from .lightrs/backups"),
];

/// `/name …` prompt → the carried built-in (if any) plus its raw args.
/// Built-ins shadow `.claude/commands` of the same name, matching the TUI.
pub fn dispatch(text: &str) -> Option<(&'static Builtin, String)> {
    let rest = text.strip_prefix('/')?;
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (rest, ""),
    };
    let b = BUILTINS.iter().find(|b| b.name == name && !b.prompt)?;
    Some((b, args.to_string()))
}

/// Everything the ACP handlers need. `cfg` is the session-local view —
/// `/model` mutates it instead of the global config.
pub struct Ctx<'a> {
    pub agent: &'a Arc<Mutex<Agent>>,
    pub workdir: &'a Path,
    pub cfg: &'a Arc<Mutex<AppConfig>>,
}

/// Run a carried built-in: the reply text for the wire. Handlers mirror their
/// TUI arms in `tui/slash/mod.rs` (including `push_system`) minus the parts
/// that need a terminal.
pub async fn run(b: &Builtin, args: &str, ctx: Ctx<'_>) -> Result<String> {
    let text = match b.name {
        "cwd" => format!("cwd: {}", ctx.workdir.display()),
        "todo" => {
            let items = crate::tools::todo::load(ctx.workdir);
            if items.is_empty() {
                "no todos — the model can set them with todo_write".to_string()
            } else {
                crate::tools::todo::render(&items)
            }
        }
        "status" => session_summary(&ctx, "status").await?,
        "context" => session_summary(&ctx, "context").await?,
        "skills" => skills_text(&ctx).await?,
        "doctor" => {
            let cfg = ctx.cfg.lock().await.clone();
            crate::doctor::report(&cfg)?
        }
        "diff" => diff_text(&ctx).await?,
        "compact" => {
            let mut agent = ctx.agent.lock().await;
            let n = agent.compact_history();
            if n == 0 {
                "nothing to compact — tool history already short".to_string()
            } else {
                format!(
                    "compacted {n} older tool result(s); ~{} tokens in context now",
                    agent.token_summary().estimate
                )
            }
        }
        "undo" => {
            let agent = ctx.agent.lock().await;
            crate::session::undo_last_turn(ctx.workdir, &agent.ui_messages())?
                .join("\n")
        }
        "fork" => {
            let name = args.split_whitespace().next().unwrap_or("fork");
            let agent = ctx.agent.lock().await;
            let file = crate::session::fork(&agent, ctx.workdir, name)?;
            let slug = file.trim_end_matches(".json");
            format!("session forked → {slug}")
        }
        "model" => model_cmd(args, &ctx).await?,
        "clear" => {
            ctx.agent.lock().await.clear_conversation();
            "agent context cleared — the next prompt starts fresh (your client transcript is unchanged)".to_string()
        }
        other => anyhow::bail!("`/{other}` is not handled agent-side — this is a bug"),
    };
    // Transcript parity with the TUI: the reply lands in ui_history so
    // `session/load` and the TUI see it.
    ctx.agent.lock().await.push_system(text.clone());
    Ok(text)
}

/// `status`/`context` — a session line, a tokens line, and for `context`
/// the live context report (TUI parity, minus config-path cosmetics).
async fn session_summary(ctx: &Ctx<'_>, tag: &str) -> Result<String> {
    let cfg = ctx.cfg.lock().await.clone();
    let agent = ctx.agent.lock().await;
    let n = agent.api_context_len();
    let window = crate::config::model_context_window(&cfg.model);
    let usage = crate::agent::context_usage_text(agent.token_summary(), n, window);
    let changed: Vec<&String> = agent.changed_files().collect();
    let mut text = format!(
        "session {tag}\nprovider: {} · model: {} · workdir: {}\napi base: {}\ncontext: {n} API message(s) in history\n{usage}",
        provider_name(cfg.provider),
        cfg.model,
        ctx.workdir.display(),
        cfg.base_url,
    );
    if tag == "context" {
        text.push('\n');
        text.push_str(&agent.context_report());
        return Ok(text);
    }
    if !changed.is_empty() {
        let names: Vec<&str> = changed.iter().take(5).map(|s| s.as_str()).collect();
        let more = changed.len().saturating_sub(5);
        let suffix = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        let names = names.join(", ");
        text.push_str(&format!("\nfiles changed: {} ({names}{suffix})", changed.len()));
    }
    Ok(text)
}

async fn skills_text(ctx: &Ctx<'_>) -> Result<String> {
    let catalog = crate::skills::load(&crate::skills::load_settings());
    ctx.agent.lock().await.set_skill_catalog(catalog);
    let skills = ctx.agent.lock().await.skill_catalog().skills.clone();
    if skills.is_empty() {
        return Ok(
            "No skills found. Paths: ~/.config/lightrs/skills, ~/.cursor/skills, ~/.claude/skills, ~/.agents/skills, ~/.pi/agent/skills, ~/.codex/skills — or `lightrs import opencode`."
                .to_string(),
        );
    }
    let mut lines: Vec<String> = skills
        .iter()
        .map(|s| format!("  ${} — {}", s.id, s.description))
        .collect();
    let total = lines.len();
    lines.truncate(24);
    let extra = if total > 24 { "\n  … (use /skills after adding more)" } else { "" };
    Ok(format!(
        "{total} skill(s). Invoke with $id on your message:\n{}{extra}",
        lines.join("\n")
    ))
}

async fn diff_text(ctx: &Ctx<'_>) -> Result<String> {
    let agent = ctx.agent.lock().await;
    let diffs = crate::session::collect_file_diffs(&agent.ui_messages());
    if diffs.is_empty() {
        return Ok("no file changes this session".to_string());
    }
    let files: Vec<String> = diffs
        .iter()
        .map(|d| d.path.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let adds: usize = diffs.iter().map(|d| d.lines_added).sum();
    let rems: usize = diffs.iter().map(|d| d.lines_removed).sum();
    Ok(format!(
        "session diff · {} file(s) · +{adds} −{rems} across {} write(s) — {}",
        files.len(),
        diffs.len(),
        files.iter().take(8).cloned().collect::<Vec<_>>().join(", ")
    ))
}

/// `/model` — bare prints the session model; `name` or `provider [name]`
/// switches it (session-local; the global config is untouched). The TUI's
/// numbered `/model <n>` picks need its fetched list and stay TUI-only.
async fn model_cmd(args: &str, ctx: &Ctx<'_>) -> Result<String> {
    let argv: Vec<&str> = args.split_whitespace().collect();
    let cfg = ctx.cfg.lock().await.clone();
    let Some(first) = argv.first() else {
        return Ok(format!(
            "model: {} · {} · {}",
            provider_name(cfg.provider),
            cfg.model,
            cfg.base_url
        ));
    };
    // `/model <provider> [name]` — provider default model when name omitted;
    // otherwise `<name>` keeps the current provider, base URL and key.
    let switch = match Provider::from_str(first, true).ok() {
        Some(p) => model_switch(
            p,
            argv.get(1).copied().unwrap_or_else(|| p.default_model()),
            &cfg,
        )?,
        None => model_switch(cfg.provider, first, &cfg)?,
    };
    ctx.agent.lock().await.set_client(crate::provider::LlmClient::new(
        switch.base_url.clone(),
        switch.api_key.clone(),
        switch.model.clone(),
    ));
    let mut cfg = ctx.cfg.lock().await;
    cfg.provider = switch.provider;
    cfg.model = switch.model.clone();
    cfg.base_url = switch.base_url;
    cfg.api_key = switch.api_key;
    Ok(format!(
        "model → {} · {} · {} (session-local)",
        provider_name(switch.provider),
        switch.model,
        cfg.base_url
    ))
}

/// Provider switch resolution — mirrors the TUI's `model_switch` without
/// reaching into the private `tui::slash` module.
fn model_switch(provider: Provider, model: &str, cfg: &AppConfig) -> Result<ModelSwitch> {
    use anyhow::Context;
    if provider == cfg.provider {
        return Ok(ModelSwitch {
            provider,
            model: model.to_string(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
        });
    }
    let api_key = AppConfig::resolve_runtime_api_key(provider)
        .with_context(|| format!("cannot switch to {}", provider_name(provider)))?;
    Ok(ModelSwitch {
        provider,
        model: model.to_string(),
        base_url: cfg.endpoint_for(provider),
        api_key,
    })
}

struct ModelSwitch {
    provider: Provider,
    model: String,
    base_url: String,
    api_key: String,
}

fn provider_name(p: Provider) -> &'static str {
    match p {
        Provider::Glm => "glm",
        Provider::Kimi => "kimi",
        Provider::Minimax => "minimax",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    async fn test_ctx(
        tag: &str,
    ) -> (std::path::PathBuf, Arc<Mutex<AppConfig>>, Arc<Mutex<Agent>>) {
        let dir = std::env::temp_dir()
            .join(format!("lightrs-builtins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = Arc::new(Mutex::new(test_config()));
        let agent = Arc::new(Mutex::new(
            crate::acp::engine::make_agent(&cfg.lock().await.clone(), dir.clone()).unwrap(),
        ));
        (dir, cfg, agent)
    }

    fn test_config() -> AppConfig {
        let mut cfg = AppConfig::load(&Default::default(), false).expect("default config");
        cfg.api_key = "test-key".into();
        cfg
    }

    #[tokio::test]
    async fn carried_builtins_are_sorted_and_dispatchable() {
        let mut names: Vec<&str> = BUILTINS.iter().map(|b| b.name).collect();
        let sorted = names.clone();
        names.sort_unstable();
        assert_eq!(names, sorted, "BUILTINS must stay sorted");
        assert_eq!(names.len(), BUILTINS.len(), "no duplicates");
        for b in BUILTINS.iter().filter(|b| !b.prompt) {
            assert!(
                dispatch(&format!("/{}", b.name)).is_some(),
                "/{} advertises a handler run() lacks",
                b.name
            );
        }
    }

    #[test]
    fn dispatch_needs_slash_and_splits_args() {
        let (b, args) = dispatch("/todo two  args ").unwrap();
        assert_eq!(b.name, "todo");
        assert_eq!(args, "two  args");
        assert!(dispatch("plain").is_none());
        assert!(dispatch("/init").is_none(), "/init is prompt expansion");
        assert!(dispatch("/unknown").is_none(), "unknowns pass through to the model");
        assert!(dispatch("/editor").is_none(), "TUI-only commands stay local");
        assert!(dispatch("/export").is_none(), "TUI-only commands stay local");
    }

    #[tokio::test]
    async fn cwd_and_status_reply_without_a_provider() {
        let (dir, cfg, agent) = test_ctx("cwd").await;
        let reply = run(
            dispatch("/cwd").unwrap().0,
            "",
            Ctx { agent: &agent, workdir: &dir, cfg: &cfg },
        )
        .await
        .unwrap();
        assert_eq!(reply, format!("cwd: {}", dir.display()));

        let reply = run(
            dispatch("/status").unwrap().0,
            "",
            Ctx { agent: &agent, workdir: &dir, cfg: &cfg },
        )
        .await
        .unwrap();
        assert!(reply.contains("session status"), "got {reply}");
        assert!(reply.contains("API message(s) in history"), "got {reply}");
    }

    #[tokio::test]
    async fn model_bare_prints_current() {
        let (dir, cfg, agent) = test_ctx("model").await;
        let reply = run(
            dispatch("/model").unwrap().0,
            "",
            Ctx { agent: &agent, workdir: &dir, cfg: &cfg },
        )
        .await
        .unwrap();
        assert!(reply.starts_with("model: "), "got {reply}");
    }
}
