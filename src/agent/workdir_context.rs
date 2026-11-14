use std::fs;
use std::path::Path;

const MAX_CHARS: usize = 6_144;

const CANDIDATE_NAMES: &[&str] = &["AGENTS.md", "AIDER.md", ".lightrs/AGENTS.md"];

/// Load project-local agent hints from the workdir (Aider/Codex-style).
pub fn load_workdir_context(workdir: &Path) -> String {
    let mut sections = Vec::new();
    for rel in CANDIDATE_NAMES {
        let path = workdir.join(rel);
        let Ok(data) = fs::read_to_string(&path) else {
            continue;
        };
        let body = data.trim();
        if body.is_empty() {
            continue;
        }
        let chunk = if body.len() > MAX_CHARS {
            format!(
                "{}\n[truncated at {MAX_CHARS} bytes]",
                &body[..body.floor_char_boundary(MAX_CHARS)]
            )
        } else {
            body.to_string()
        };
        sections.push(format!("--- {rel} ---\n{chunk}"));
    }
    if sections.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\nWorkdir agent context:\n");
    for sec in sections {
        if out.len() + sec.len() > MAX_CHARS * 2 {
            out.push_str("\n[workdir context truncated]");
            break;
        }
        out.push_str(&sec);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn loads_agents_md() {
        let dir = std::env::temp_dir().join(format!("lightrs-wd-ctx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("AGENTS.md"), "use cargo test").unwrap();
        let ctx = load_workdir_context(&dir);
        assert!(ctx.contains("use cargo test"));
        let _ = fs::remove_dir_all(&dir);
    }
}