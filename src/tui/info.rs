use crate::config::{AppConfig, Provider, config_path};
use crate::tui::app::RunState;

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
pub fn header_lines(
    cfg: &AppConfig,
    context_usage: &str,
    state: RunState,
    width: u16,
) -> [String; 2] {
    let w = usize::from(width.max(1));
    let state_label = match state {
        RunState::Ready => "ready",
        RunState::Busy => "busy",
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
    let line2 = truncate_end(
        &format!("path: {path_short} · {context_usage} · {api_host}"),
        w,
    );
    [line1, line2]
}

pub fn header_compact(
    cfg: &AppConfig,
    context_usage: &str,
    state: RunState,
    width: u16,
) -> String {
    let w = usize::from(width.max(1));
    let state_label = match state {
        RunState::Ready => "ready",
        RunState::Busy => "busy",
    };
    truncate_end(
        &format!(
            "lightrs · {} · {} · {} · {} · {}",
            provider_name(cfg.provider),
            cfg.model,
            state_label,
            context_usage,
            truncate_middle(&cfg.workdir.display().to_string(), 20),
        ),
        w,
    )
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_truncation() {
        let s = truncate_middle("/very/long/path/to/project", 20);
        assert!(s.contains('…'));
        assert!(s.chars().count() <= 20);
    }
}