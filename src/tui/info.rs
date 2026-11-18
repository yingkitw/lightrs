use crate::config::{AppConfig, Provider, config_path};
use crate::tui::app::{App, RunState};

pub fn provider_name(p: Provider) -> &'static str {
    match p {
        Provider::Glm => "glm",
        Provider::Kimi => "kimi",
        Provider::Minimax => "minimax",
    }
}

pub fn session_start_message(cfg: &AppConfig) -> String {
    let path = cfg.workdir.display().to_string();
    format!(
        "Ready — {} · {} · {}\n/help · /status for details",
        provider_name(cfg.provider),
        cfg.model,
        truncate_middle(&path, 48),
    )
}

pub fn session_resume_message(cfg: &AppConfig, ui_messages: usize, api_messages: usize) -> String {
    format!(
        "Resumed previous session — {} · {} · {} · {ui_messages} UI lines · {api_messages} API msgs · /clear starts fresh",
        provider_name(cfg.provider),
        cfg.model,
        truncate_middle(&cfg.workdir.display().to_string(), 40),
    )
}

pub fn context_details(
    cfg: &AppConfig,
    api_messages: usize,
    context_usage: &str,
    state: &str,
) -> String {
    let base = session_details(cfg, api_messages, state);
    format!("{base}\ntokens: {context_usage}")
}

pub fn session_details(cfg: &AppConfig, api_messages: usize, state: &str) -> String {
    let config = config_path();
    let config_line = if config.is_file() {
        format!("config: {}", config.display())
    } else {
        "config: none (flags / env only)".into()
    };
    let key_hint = if cfg.api_key.is_empty() {
        format!("API key: {} (missing)", cfg.provider.env_var())
    } else {
        format!("API key: {} (set)", cfg.provider.env_var())
    };
    format!(
        "Session {state}\n\
         provider: {}\n\
         model: {}\n\
         workdir: {}\n\
         api base: {}\n\
         {config_line}\n\
         context: {api_messages} API message(s) in history\n\
         {key_hint}",
        provider_name(cfg.provider),
        cfg.model,
        cfg.workdir.display(),
        cfg.base_url,
    )
}

/// Two-line header: identity + path/context (truncated to `width` chars per line).
pub fn header_lines(cfg: &AppConfig, context_usage: &str, app: &App, width: u16) -> [String; 2] {
    let w = usize::from(width.max(1));
    let state_label = match app.state {
        RunState::Ready => "ready".into(),
        RunState::Busy => app.busy_label(),
    };
    let line1 = truncate_end(
        &format!(
            "lightrs {} · {} · {} · {}",
            env!("CARGO_PKG_VERSION"),
            provider_name(cfg.provider),
            cfg.model,
            state_label,
        ),
        w,
    );
    let api_host = api_host_label(&cfg.base_url);
    let path = cfg.workdir.display().to_string();
    let path_short = truncate_middle(&path, w.saturating_sub(24).max(16));
    let branch = app
        .git_branch
        .as_deref()
        .map(|b| format!(":{b}"))
        .unwrap_or_default();
    let line2 = truncate_end(
        &format!("path: {path_short}{branch} · {context_usage} · {api_host}"),
        w,
    );
    [line1, line2]
}

pub fn header_compact(cfg: &AppConfig, context_usage: &str, app: &App, width: u16) -> String {
    let w = usize::from(width.max(1));
    let state_label = match app.state {
        RunState::Ready => "ready".into(),
        RunState::Busy => app.busy_label(),
    };
    let branch = app
        .git_branch
        .as_deref()
        .map(|b| format!(":{b}"))
        .unwrap_or_default();
    let path_branch = format!(
        "{}{branch}",
        truncate_middle(&cfg.workdir.display().to_string(), 20)
    );
    truncate_end(
        &format!(
            "lightrs · {} · {} · {} · {} · {}",
            provider_name(cfg.provider),
            cfg.model,
            state_label,
            context_usage,
            path_branch,
        ),
        w,
    )
}

/// Header text lines — 1 (compact) below 22 rows, else 2 (title + `path:`).
/// The caller must add one more row for the header block's bottom border.
pub fn header_row_count(term_height: u16) -> u16 {
    if term_height < 22 { 1 } else { 2 }
}

fn api_host_label(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed)
        .to_string()
}

pub fn truncate_end(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

pub fn truncate_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    if max <= 3 {
        return "…".repeat(max);
    }
    let left = (max - 1) / 2;
    let right = max - 1 - left;
    let start: String = s.chars().take(left).collect();
    let end: String = s.chars().skip(n - right).collect();
    format!("{start}…{end}")
}

/// Current git branch for the header path label (`work:main`). Reads
/// `.git/HEAD` directly — handles linked worktrees (`.git` file) and detached
/// HEAD (short sha). `None` outside a repo.
pub fn git_branch_label(dir: &std::path::Path) -> Option<String> {
    let git = dir.join(".git");
    let head = if git.is_file() {
        // Worktree/submodule: `.git` is a file pointing at the real gitdir.
        let text = std::fs::read_to_string(&git).ok()?;
        let gd = text.trim().strip_prefix("gitdir:")?.trim();
        let gd = if std::path::Path::new(gd).is_absolute() {
            std::path::PathBuf::from(gd)
        } else {
            dir.join(gd)
        };
        gd.join("HEAD")
    } else {
        git.join("HEAD")
    };
    let head = std::fs::read_to_string(head).ok()?;
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref: refs/heads/") {
        return Some(r.to_string());
    }
    // Detached HEAD — short sha.
    Some(head.chars().take(7).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::tools::ShellPolicy;

    fn cfg() -> AppConfig {
        AppConfig {
            provider: Provider::Kimi,
            model: "k2".into(),
            fallback_model: None,
            weak_model: None,
            thinking: None,
            base_url: "https://api.moonshot.ai/v1".into(),
            api_key: String::new(),
            workdir: std::path::PathBuf::from("/work/proj"),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            format_command: None,
            read_only: false,
            heal_steps: 3,
            endpoints: Default::default(),
            mcp: Default::default(),
            lsp: Default::default(),
            untrusted_project: false,
            trust_project: false,
        }
    }

    fn app() -> App {
        App::new(cfg())
    }

    #[test]
    fn provider_labels() {
        assert_eq!(provider_name(Provider::Glm), "glm");
        assert_eq!(provider_name(Provider::Kimi), "kimi");
        assert_eq!(provider_name(Provider::Minimax), "minimax");
    }

    #[test]
    fn end_truncation() {
        assert_eq!(truncate_end("short", 10), "short");
        assert_eq!(truncate_end("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn host_label_strips_scheme_and_slash() {
        assert_eq!(api_host_label("https://api.z.ai/api/paas/v4/"), "api.z.ai/api/paas/v4");
        assert_eq!(api_host_label("http://127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(api_host_label("naked-host"), "naked-host");
    }

    #[test]
    fn row_count_threshold() {
        assert_eq!(header_row_count(21), 1);
        assert_eq!(header_row_count(22), 2);
        assert_eq!(header_row_count(80), 2);
    }

    #[test]
    fn start_and_resume_messages_identify_setup() {
        let c = cfg();
        let start = session_start_message(&c);
        assert!(start.contains("kimi") && start.contains("k2") && start.contains("/help"));

        let resume = session_resume_message(&c, 7, 3);
        assert!(resume.contains("Resumed") && resume.contains("7 UI lines") && resume.contains("3 API msgs"));
    }

    #[test]
    fn details_report_key_state() {
        let mut c = cfg();
        let missing = session_details(&c, 4, "ready");
        assert!(missing.contains("MOONSHOT_API_KEY (missing)"));
        assert!(missing.contains("provider: kimi") && missing.contains("model: k2"));
        assert!(missing.contains("4 API message(s)"));

        c.api_key = "secret".into();
        assert!(session_details(&c, 4, "ready").contains("MOONSHOT_API_KEY (set)"));

        let ctx = context_details(&c, 4, "~2k", "ready");
        assert!(ctx.ends_with("tokens: ~2k"));
    }

    #[test]
    fn header_lines_include_path_branch_and_ctx() {
        let c = cfg();
        let mut a = app();
        a.git_branch = Some("main".into());
        let [l1, l2] = header_lines(&c, "~1k", &a, 120);
        assert!(l1.contains("kimi") && l1.contains("ready"));
        assert!(l2.starts_with("path: ") && l2.contains(":main") && l2.contains("~1k"));
        assert!(l2.contains("api.moonshot.ai/v1"));

        let compact = header_compact(&c, "~1k", &a, 120);
        assert!(compact.contains("lightrs") && compact.contains(":main"));
    }

    #[test]
    fn middle_truncation() {
        let s = truncate_middle("/very/long/path/to/project", 20);
        assert!(s.contains('…'));
        assert!(s.chars().count() <= 20);
    }

    #[test]
    fn git_branch_from_head_and_worktree_file() {
        let dir = std::env::temp_dir().join(format!("lb-branch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(super::git_branch_label(&dir).as_deref(), Some("feature/x"));

        // Linked worktree: `.git` is a file → gitdir/HEAD.
        let wt = dir.join("wt");
        let gd = dir.join("gitdir-x");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&gd).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}", gd.display())).unwrap();
        std::fs::write(gd.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        assert_eq!(super::git_branch_label(&wt).as_deref(), Some("wt-branch"));

        // Detached HEAD → short sha.
        let det = dir.join("det");
        std::fs::create_dir_all(det.join(".git")).unwrap();
        std::fs::write(det.join(".git/HEAD"), "0123456789abcdef").unwrap();
        assert_eq!(super::git_branch_label(&det).as_deref(), Some("0123456"));

        assert_eq!(super::git_branch_label(&dir.join("nope")), None);
    }
}

