use std::fs;
use std::path::Path;

/// Parsed `.gitignore` rules (minimal; no nested includes).
#[derive(Clone, Debug, Default)]
pub struct Gitignore {
    patterns: Vec<String>,
}

impl Gitignore {
    pub fn load(workdir: &Path) -> Self {
        let path = workdir.join(".gitignore");
        let Ok(body) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let patterns = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.trim_start_matches('/').to_string())
            .collect();
        Self { patterns }
    }

    pub fn is_ignored(&self, rel: impl AsRef<str>, is_dir: bool) -> bool {
        let rel = rel.as_ref().trim_start_matches("./");
        if rel.is_empty() {
            return false;
        }
        for pat in &self.patterns {
            if pattern_matches(pat, rel, is_dir) {
                return true;
            }
        }
        false
    }
}

fn pattern_matches(pat: &str, rel: &str, is_dir: bool) -> bool {
    if pat.ends_with('/') {
        let p = pat.trim_end_matches('/');
        return rel == p || rel.starts_with(&format!("{p}/"));
    }
    let _ = is_dir;
    if pat.contains('*') {
        return glob_simple(pat, rel);
    }
    rel == pat
        || rel.ends_with(&format!("/{pat}"))
        || rel.starts_with(&format!("{pat}/"))
}

fn glob_simple(pat: &str, rel: &str) -> bool {
    if let Some(suffix) = pat.strip_prefix('*') {
        return rel.ends_with(suffix);
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return rel.starts_with(prefix);
    }
    pat == rel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_target_dir() {
        let gi = Gitignore {
            patterns: vec!["target/".into()],
        };
        assert!(gi.is_ignored("target/debug/foo", true));
        assert!(gi.is_ignored("target/debug/foo", false));
    }

    #[test]
    fn star_suffix() {
        let gi = Gitignore {
            patterns: vec!["*.log".into()],
        };
        assert!(gi.is_ignored("tmp/out.log", false));
        assert!(!gi.is_ignored("tmp/out.txt", false));
    }
}