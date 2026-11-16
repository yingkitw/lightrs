use std::fs;
use std::path::Path;

const MAX_CHARS: usize = 6_144;

const CANDIDATE_NAMES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    "AIDER.md",
    ".cursorrules",
    ".windsurfrules",
    ".github/copilot-instructions.md",
    ".lightrs/AGENTS.md",
];

/// Cursor / Windsurf rule directories — every `*.md`/`*.mdc` inside, sorted by
/// name so injection order is stable.
const CANDIDATE_RULE_DIRS: &[&str] = &[".cursor/rules", ".windsurf/rules"];

/// Load project-local agent hints from the workdir (Aider/Codex/Claude/Gemini-style).
pub fn load_workdir_context(workdir: &Path) -> String {
    let mut sections = Vec::new();
    for rel in CANDIDATE_NAMES {
        push_section(&mut sections, rel, &workdir.join(rel), false);
    }
    for dir in CANDIDATE_RULE_DIRS {
        let Ok(rd) = fs::read_dir(workdir.join(dir)) else {
            continue;
        };
        let mut names: Vec<_> = rd.flatten().map(|e| e.file_name()).collect();
        names.sort();
        for name in names {
            let rel = format!("{dir}/{}", name.to_string_lossy());
            if !(rel.ends_with(".md") || rel.ends_with(".mdc")) {
                continue;
            }
            push_section(&mut sections, &rel, &workdir.join(&rel), true);
        }
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

fn push_section(sections: &mut Vec<String>, rel: &str, path: &Path, strip_meta: bool) {
    let Ok(data) = fs::read_to_string(path) else {
        return;
    };
    let mut body = data.trim();
    if strip_meta {
        body = strip_frontmatter(body);
    }
    if body.is_empty() {
        return;
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

/// Drop a leading `---\n…\n---` YAML block (Cursor `.mdc` rule metadata:
/// description/globs/alwaysApply carry no instruction value for the model).
fn strip_frontmatter(body: &str) -> &str {
    let Some(rest) = body.strip_prefix("---\n") else {
        return body;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim(),
        None => body,
    }
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

    #[test]
    fn loads_claude_and_gemini_md() {
        let dir = std::env::temp_dir().join(format!("lightrs-wd-ctx2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("CLAUDE.md"), "claude rule").unwrap();
        fs::write(dir.join("GEMINI.md"), "gemini rule").unwrap();
        let ctx = load_workdir_context(&dir);
        assert!(ctx.contains("claude rule"), "ctx: {ctx}");
        assert!(ctx.contains("gemini rule"), "ctx: {ctx}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_legacy_dotfiles_and_copilot_instructions() {
        let dir = std::env::temp_dir().join(format!("lightrs-wd-ctx3-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".github")).unwrap();
        fs::write(dir.join(".cursorrules"), "cursor dotfile").unwrap();
        fs::write(dir.join(".windsurfrules"), "windsurf dotfile").unwrap();
        fs::write(dir.join(".github/copilot-instructions.md"), "copilot rules").unwrap();
        let ctx = load_workdir_context(&dir);
        for needle in ["cursor dotfile", "windsurf dotfile", "copilot rules"] {
            assert!(ctx.contains(needle), "missing {needle} in: {ctx}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_rule_dirs_sorted_and_strips_mdc_frontmatter() {
        let dir = std::env::temp_dir().join(format!("lightrs-wd-ctx4-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".cursor/rules")).unwrap();
        fs::create_dir_all(dir.join(".windsurf/rules")).unwrap();
        fs::write(
            dir.join(".cursor/rules/rust.mdc"),
            "---\ndescription: rust rules\nalwaysApply: true\n---\nprefer thiserror",
        )
        .unwrap();
        fs::write(dir.join(".windsurf/rules/style.md"), "no wildcard imports").unwrap();
        // Non-md files in rule dirs are ignored.
        fs::write(dir.join(".cursor/rules/notes.txt"), "ignore me").unwrap();

        let ctx = load_workdir_context(&dir);
        assert!(ctx.contains("prefer thiserror"), "ctx: {ctx}");
        assert!(ctx.contains("no wildcard imports"), "ctx: {ctx}");
        assert!(!ctx.contains("alwaysApply"), "ctx: {ctx}");
        assert!(!ctx.contains("ignore me"), "ctx: {ctx}");
        let _ = fs::remove_dir_all(&dir);
    }
}
