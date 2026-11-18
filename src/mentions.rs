use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Per-file cap when attaching `@path` to the API user message (UI stays unchanged).
const MENTION_ATTACH_FILE_MAX: usize = 12 * 1024;
/// Total attachment budget per send.
const MENTION_ATTACH_TOTAL_MAX: usize = 36 * 1024;

/// Paths from `@token` in the composer (deduped, order preserved).
pub fn collect_file_mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            i += 1;
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            let path: String = chars[start..i].iter().collect();
            if !path.is_empty() && seen.insert(path.clone()) {
                out.push(path);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Lean API payload: composer text plus capped file bodies for `@` mentions.
/// `roots` = `ShellPolicy.read_roots` — files under `--add-dir`/`extra_read_dirs`
/// are mentionable too (same consent boundary as `read_file`).
pub fn api_user_content(workdir: &Path, composer: &str, roots: &[std::path::PathBuf]) -> String {
    let mentions = collect_file_mentions(composer);
    if mentions.is_empty() {
        return composer.to_string();
    }

    let mut extra = String::new();
    let mut used = 0usize;
    for rel in mentions {
        if used >= MENTION_ATTACH_TOTAL_MAX {
            extra.push_str("\n[attachments truncated: total budget]");
            break;
        }
        let Ok(full) = crate::config::validate_read_path(workdir, roots, Path::new(&rel)) else {
            continue;
        };
        if !full.is_file() {
            continue;
        }
        let Ok(file) = fs::File::open(&full) else {
            continue;
        };
        let cap = MENTION_ATTACH_FILE_MAX.min(MENTION_ATTACH_TOTAL_MAX - used);
        // Bound at the source — metadata gives the real size for the
        // truncation note without loading the whole file.
        let size = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
        use std::io::Read;
        let mut data = Vec::new();
        if file.take(cap as u64).read_to_end(&mut data).is_err()
        {
            continue;
        }
        let body = String::from_utf8_lossy(&data);
        extra.push_str(&format!("\n--- @{rel} ({}B) ---\n{body}", data.len()));
        if size > data.len() {
            extra.push_str(&format!("\n[truncated; file {size} bytes]"));
        }
        used += data.len();
    }

    if extra.is_empty() {
        composer.to_string()
    } else {
        format!("{composer}\n\n[attached]{extra}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn api_attach_is_capped() {
        let dir = std::env::temp_dir().join(format!("lightrs-attach-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "hello").unwrap();
        let api = api_user_content(&dir, "see @a.txt", &[]);
        assert!(api.contains("hello"));
        assert!(api.contains("--- @a.txt"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// `@` mentions honor `read_roots` (`--add-dir`) — same consent boundary
    /// as `read_file`; outside workdir AND outside roots stays silent.
    #[test]
    fn api_attach_reaches_extra_root() {
        let tag = std::process::id();
        let dir = std::env::temp_dir().join(format!("lightrs-att-{tag}"));
        let root = std::env::temp_dir().join(format!("lightrs-att-root-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("shared.rs"), "pub fn s(){}").unwrap();
        let roots = vec![root.canonicalize().unwrap()];
        // By absolute path under the extra root.
        let api = api_user_content(
            &dir,
            &format!("check @{}", root.join("shared.rs").display()),
            &roots,
        );
        assert!(api.contains("pub fn s()"), "extra-root mention: {api}");
        // Without the root the same mention is silently skipped.
        let api = api_user_content(
            &dir,
            &format!("check @{}", root.join("shared.rs").display()),
            &[],
        );
        assert!(!api.contains("pub fn s"), "no-root mention must skip: {api}");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&root);
    }
}