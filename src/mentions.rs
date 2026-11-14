use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::config::validate_work_path;

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
pub fn api_user_content(workdir: &Path, composer: &str) -> String {
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
        let Ok(full) = validate_work_path(workdir, Path::new(&rel)) else {
            continue;
        };
        if !full.is_file() {
            continue;
        }
        let Ok(data) = fs::read(&full) else {
            continue;
        };
        let cap = MENTION_ATTACH_FILE_MAX.min(MENTION_ATTACH_TOTAL_MAX - used);
        let take = data.len().min(cap);
        let body = String::from_utf8_lossy(&data[..take]);
        extra.push_str(&format!("\n--- @{rel} ({take}B) ---\n{body}"));
        if data.len() > take {
            extra.push_str(&format!("\n[truncated; file {} bytes]", data.len()));
        }
        used += take;
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
        let api = api_user_content(&dir, "see @a.txt");
        assert!(api.contains("hello"));
        assert!(api.contains("--- @a.txt"));
        let _ = fs::remove_dir_all(&dir);
    }
}