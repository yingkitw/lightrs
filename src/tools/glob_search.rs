use std::path::Path;

use anyhow::{Result, bail};
use serde::Deserialize;



use super::glob_pattern;
use super::walk;

const MAX_RESULTS: usize = 200;

#[derive(Debug, Deserialize)]
pub struct GlobSearchArgs {
    pub pattern: String,
    pub path: Option<String>,
}

pub fn glob_file_search(
    workdir: &Path,
    args: GlobSearchArgs,
    roots: &[std::path::PathBuf],
) -> Result<String> {
    let pattern = args.pattern.trim();
    if pattern.is_empty() {
        bail!("pattern must not be empty");
    }
    if pattern.bytes().any(|b| b == b'{' || b == b'}') {
        bail!("glob patterns with `{{`/`}}` are not supported");
    }
    let root = match args.path.as_deref() {
        None | Some(".") | Some("") => workdir.to_path_buf(),
        Some(p) => crate::config::validate_read_path(workdir, roots, Path::new(p))?,
    };
    let mut files = Vec::new();
    // Filter by the validated root, not workdir — extra read roots (--add-dir)
    // must not drop their own children.
    walk::collect_files(&root, &root, &mut files)?;
    let mut matches: Vec<String> = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(workdir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if glob_pattern::glob_matches(pattern, &rel) {
            matches.push(rel);
        }
    }
    matches.sort();
    matches.dedup();
    let truncated = matches.len() > MAX_RESULTS;
    matches.truncate(MAX_RESULTS);
    if matches.is_empty() {
        return Ok(format!(
            "no paths match `{pattern}` under {}",
            display_root(&root, workdir)
        ));
    }
    let mut out = matches.join("\n");
    if truncated {
        out.push_str(&format!("\n\n[truncated at {MAX_RESULTS} paths]"));
    }
    Ok(out)
}

fn display_root(root: &Path, workdir: &Path) -> String {
    root.strip_prefix(workdir)
        .map(|p| {
            if p.as_os_str().is_empty() {
                ".".into()
            } else {
                p.to_string_lossy().replace('\\', "/")
            }
        })
        .unwrap_or_else(|_| root.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_rust_glob() {
        let dir = std::env::temp_dir().join(format!("lightrs-glob-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
        fs::write(dir.join("README.md"), "").unwrap();
        let out = glob_file_search(
            &dir,
            GlobSearchArgs {
                pattern: "**/*.rs".into(),
                path: None,
            },
            &[],
        )
        .unwrap();
        assert!(out.contains("src/lib.rs"));
        assert!(!out.contains("README"));
        let _ = fs::remove_dir_all(&dir);
    }
}