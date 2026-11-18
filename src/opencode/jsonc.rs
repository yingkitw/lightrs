use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

/// `opencode.json` can be project-level (repo-controlled) — cap the read.
const JSONC_MAX: u64 = 1024 * 1024;

pub fn load_jsonc(path: &Path) -> Result<Value> {
    let text = crate::tools::read_to_string_capped(path, JSONC_MAX)
        .ok_or_else(|| anyhow::anyhow!("{} unreadable or over {JSONC_MAX} B", path.display()))?;
    let stripped = strip_jsonc_comments(&text);
    serde_json::from_str(&stripped).with_context(|| format!("parse {}", path.display()))
}

pub fn merge_json_docs(docs: &[Value]) -> Value {
    let mut out = Value::Object(Default::default());
    for doc in docs {
        merge_into(&mut out, doc);
    }
    out
}

fn merge_into(base: &mut Value, overlay: &Value) {
    match overlay {
        Value::Object(b) if base.is_object() => {
            let a = base.as_object_mut().expect("object");
            for (k, v) in b {
                if let Some(existing) = a.get_mut(k) {
                    merge_into(existing, v);
                } else {
                    a.insert(k.clone(), v.clone());
                }
            }
        }
        _ => *base = overlay.clone(),
    }
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == '/' && matches!(chars.peek(), Some('/')) {
            while matches!(chars.next(), Some(c) if c != '\n') {}
            continue;
        }
        out.push(ch);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_jsonc_rejects_oversized() {
        let dir = std::env::temp_dir().join(format!("lb-jsonc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("opencode.json");
        std::fs::write(&path, vec![b' '; JSONC_MAX as usize + 1]).unwrap();
        assert!(load_jsonc(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
