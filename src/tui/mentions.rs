use std::path::Path;

const MAX_LIST: usize = 4_000;
const HINT_SHOW: usize = 3;

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".cursor"];

/// Active `@path` token at end of input (no spaces in the path fragment).
pub fn active_mention(input: &str) -> Option<(&str, usize)> {
    crate::mentions::active_token(input, '@')
}

pub fn mention_hint(input: &str, workdir: &Path) -> Option<String> {
    let (prefix, _) = active_mention(input)?;
    let mut matches: Vec<String> = list_workdir_paths(workdir)
        .into_iter()
        .filter(|p| prefix.is_empty() || p.starts_with(prefix))
        .collect();
    if matches.is_empty() {
        return Some("@mention — no matching files".into());
    }
    matches.sort();
    if matches.len() <= HINT_SHOW {
        return Some(matches.iter().map(|p| format!("@{p}")).collect::<Vec<_>>().join(" · "));
    }
    let head: Vec<String> = matches
        .iter()
        .take(HINT_SHOW)
        .map(|p| format!("@{p}"))
        .collect();
    Some(format!("{} · …", head.join(" · ")))
}

pub fn tab_complete_mention(input: &str, workdir: &Path) -> Option<String> {
    let (prefix, at) = active_mention(input)?;
    let mut matches: Vec<String> = list_workdir_paths(workdir)
        .into_iter()
        .filter(|p| prefix.is_empty() || p.starts_with(prefix))
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    if matches.len() == 1 {
        let head = &input[..at];
        return Some(format!("{head}@{}", matches[0]));
    }
    if !prefix.is_empty() {
        let common = common_prefix_iter(matches.iter().map(String::as_str));
        if common.len() > prefix.len() {
            let head = &input[..at];
            return Some(format!("{head}@{common}"));
        }
    }
    None
}

fn common_prefix_iter<'a>(mut names: impl Iterator<Item = &'a str>) -> String {
    let first = names.next().unwrap_or("");
    let mut prefix: Vec<char> = first.chars().collect();
    for name in names {
        let chars: Vec<char> = name.chars().collect();
        let len = prefix
            .iter()
            .zip(chars.iter())
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(len);
        if prefix.is_empty() {
            break;
        }
    }
    prefix.iter().collect()
}

pub fn list_workdir_paths(workdir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk_rel(workdir, workdir, &mut out);
    out
}

fn walk_rel(base: &Path, dir: &Path, out: &mut Vec<String>) {
    if out.len() >= MAX_LIST {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in entries.flatten() {
        if out.len() >= MAX_LIST {
            break;
        }
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        if ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(base) {
                let rel_s = rel_path_string(rel);
                out.push(format!("{rel_s}/"));
            }
            walk_rel(base, &path, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel_path_string(rel));
        }
    }
}

fn rel_path_string(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_active_mention() {
        assert_eq!(active_mention("hi @src/mai"), Some(("src/mai", 3)));
        assert!(active_mention("hi @src foo").is_none());
    }

    #[test]
    fn completes_unique_file() {
        let dir = std::env::temp_dir().join(format!("lightrs-mention-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
        let out = tab_complete_mention("explain @src/l", &dir).unwrap();
        assert!(out.contains("src/lib.rs"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn fixture(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lightrs-mention-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join(".git/objects")).unwrap();
        fs::create_dir_all(dir.join("target/debug")).unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
        fs::write(dir.join("src/main.rs"), "").unwrap();
        fs::write(dir.join("README.md"), "").unwrap();
        fs::write(dir.join(".git/HEAD"), "").unwrap();
        fs::write(dir.join("target/debug/x"), "").unwrap();
        dir
    }

    #[test]
    fn list_skips_vcs_and_build_dirs() {
        let dir = fixture("skip");
        let paths = list_workdir_paths(&dir);
        assert!(paths.iter().any(|p| p == "src/lib.rs"));
        assert!(paths.iter().any(|p| p == "src/"));
        assert!(!paths.iter().any(|p| p.contains(".git") || p.contains("target")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn common_prefix_extends_partial_match() {
        let dir = fixture("common");
        // "src/" alone matches src/ dir + both files → common prefix stays "src/".
        assert_eq!(tab_complete_mention("@src/", &dir), None);
        // "@src/ma" uniquely narrows to main.rs.
        assert_eq!(
            tab_complete_mention("@src/ma", &dir).as_deref(),
            Some("@src/main.rs")
        );
        // No match at all → None.
        assert!(tab_complete_mention("@zzz", &dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hint_lists_matches_or_reports_none() {
        let dir = fixture("hint");
        let hint = mention_hint("@src/", &dir).unwrap();
        // src/ + 2 files = 3 matches → all listed.
        assert!(hint.contains("@src/lib.rs") && hint.contains("@src/main.rs"));

        let hint = mention_hint("@nope", &dir).unwrap();
        assert!(hint.contains("no matching files"));

        // Not a mention → None.
        assert!(mention_hint("hello world", &dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}