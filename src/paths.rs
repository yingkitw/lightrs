use std::path::PathBuf;

/// `~/.config/<app>` (or `%USERPROFILE%\.config\<app>` on Windows).
pub fn config_dir(app: &str) -> PathBuf {
    let base = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".config").join(app)
}

pub fn config_file(app: &str, name: &str) -> PathBuf {
    config_dir(app).join(name)
}

pub fn lightrs_config_dir() -> PathBuf {
    config_dir("lightrs")
}

pub fn lightrs_config_toml() -> PathBuf {
    config_file("lightrs", "config.toml")
}

pub fn lightrs_sessions_dir() -> PathBuf {
    config_dir("lightrs").join("sessions")
}

/// Common OpenCode global config locations without pulling `directories`.
pub fn opencode_global_config_candidates() -> Vec<PathBuf> {
    let mut paths = vec![
        config_file("opencode", "opencode.json"),
        config_file("opencode", "opencode.jsonc"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".config/opencode/opencode.json"));
        paths.push(home.join(".config/opencode/opencode.jsonc"));
    }
    paths
}

/// `opencode auth login` credential store: `{ "<provider>": {"type":"api","key":"…"} }`.
/// OpenCode uses `~/.local/share/opencode` (or `$XDG_DATA_HOME/opencode`) on all
/// platforms — it does not use `~/Library/Application Support` on macOS.
pub fn opencode_auth_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        paths.push(PathBuf::from(xdg).join("opencode/auth.json"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".local/share/opencode/auth.json"));
    }
    paths
}