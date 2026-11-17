//! Project slash commands from `.claude/commands/*.md` (Claude Code layout).
//!
//! The filename stem becomes the command name; optional `---` frontmatter may
//! carry `description`. The markdown body is the prompt sent to the model —
//! `/name args...` appends the args after the body. Built-in `/` commands win
//! on name collisions (a project file can never shadow them).

use std::path::Path;

/// One project command discovered under `.claude/commands/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashDef {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// Scan `workdir/.claude/commands/*.md`; sorted by name, empty when absent.
pub fn load(workdir: &Path) -> Vec<SlashDef> {
    let dir = workdir.join(".claude").join("commands");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<SlashDef> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .filter_map(|p| parse(&p).ok())
        .filter(|d| !d.name.is_empty() && !d.body.is_empty())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn parse(path: &Path) -> std::io::Result<SlashDef> {
    let text = crate::tools::read_to_string_capped(path, 64 * 1024).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, path.display().to_string())
    })?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let (description, body) = parse_md(&text);
    Ok(SlashDef {
        name,
        description,
        body,
    })
}

/// `description` frontmatter value + trimmed markdown body.
fn parse_md(text: &str) -> (String, String) {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (String::new(), trimmed.trim().to_string());
    };
    let rest = rest.trim_start_matches('-').trim_start();
    let Some(end) = rest.find("\n---") else {
        return (String::new(), trimmed.trim().to_string());
    };
    let mut description = String::new();
    for line in rest[..end].lines() {
        if let Some(v) = line.trim().strip_prefix("description:") {
            description = v.trim().trim_matches('"').to_string();
        }
    }
    (description, rest[end + 4..].trim().to_string())
}

/// Full prompt for a dispatched command: body plus trailing args.
pub fn expand(def: &SlashDef, args: &str) -> String {
    if args.is_empty() {
        def.body.clone()
    } else {
        format!("{}\n\n{args}", def.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        let d = dir.join(".claude/commands");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), text).unwrap();
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let (desc, body) =
            parse_md("---\ndescription: review the diff\n---\n\nReview staged changes.\n");
        assert_eq!(desc, "review the diff");
        assert_eq!(body, "Review staged changes.");
    }

    #[test]
    fn plain_markdown_has_no_frontmatter() {
        let (desc, body) = parse_md("Just do the thing.\n");
        assert!(desc.is_empty());
        assert_eq!(body, "Just do the thing.");
    }

    #[test]
    fn load_sorted_and_skips_junk() {
        let dir = std::env::temp_dir().join(format!("lightrs-cmds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "b.md", "body b");
        write(&dir, "a.md", "---\ndescription: first\n---\nbody a");
        write(&dir, "note.txt", "ignored");
        let defs = load(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "a");
        assert_eq!(defs[0].description, "first");
        assert_eq!(defs[1].name, "b");
    }

    #[test]
    fn load_missing_dir_is_empty() {
        assert!(load(Path::new("/nonexistent-lightrs-dir")).is_empty());
    }

    #[test]
    fn expand_appends_args() {
        let def = SlashDef {
            name: "x".into(),
            description: String::new(),
            body: "Do it".into(),
        };
        assert_eq!(expand(&def, ""), "Do it");
        assert_eq!(expand(&def, "src/main.rs"), "Do it\n\nsrc/main.rs");
    }
}
