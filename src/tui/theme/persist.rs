use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::lightrs_config_dir;

use super::palettes;

#[derive(Debug, Deserialize, Serialize)]
struct TuiPrefs {
    theme: String,
}

fn prefs_path() -> std::path::PathBuf {
    lightrs_config_dir().join("tui.toml")
}

pub fn load_theme_id() -> String {
    let path = prefs_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return "mocha".into();
    };
    let Ok(prefs) = toml::from_str::<TuiPrefs>(&text) else {
        return "mocha".into();
    };
    if palettes::by_id(&prefs.theme).is_some() {
        prefs.theme
    } else {
        "mocha".into()
    }
}

pub fn save_theme_id(id: &str) -> Result<()> {
    let dir = lightrs_config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = prefs_path();
    let prefs = TuiPrefs {
        theme: id.to_string(),
    };
    let text = toml::to_string_pretty(&prefs).context("encode tui.toml")?;
    fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_when_missing() {
        let id = load_theme_id();
        assert!(!id.is_empty());
    }
}