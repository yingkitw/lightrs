use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::config::validate_work_path;

const DEFAULT_NAME: &str = "lightrs-transcript.md";

pub fn export_ui_markdown(
    agent: &Agent,
    workdir: &Path,
    rel_path: Option<&str>,
) -> Result<PathBuf> {
    let rel = rel_path.unwrap_or(DEFAULT_NAME);
    let path = validate_work_path(workdir, Path::new(rel))?;
    let md = ui_messages_to_markdown(&agent.export_ui_history());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&path, md).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn ui_messages_to_markdown(messages: &[UiMessage]) -> String {
    let mut out = String::from("# lightrs transcript\n\n");
    for m in messages {
        if m.role == UiRole::System {
            continue;
        }
        let heading = role_heading(m.role);
        out.push_str(&format!("## {heading}\n\n"));
        if let Some(tc) = &m.tool_call {
            out.push_str(&format!("**tool:** `{}`\n\n", tc.name));
            if !tc.arguments.trim().is_empty() {
                out.push_str("### arguments\n\n```json\n");
                out.push_str(tc.arguments.trim());
                out.push_str("\n```\n\n");
            }
        }
        if !m.content.trim().is_empty() {
            out.push_str(&m.content);
            if !m.content.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        if let Some(fd) = &m.file_diff {
            out.push_str(&format!(
                "### diff: `{}` (+{} −{})\n\n",
                fd.path, fd.lines_added, fd.lines_removed
            ));
            out.push_str("```diff\n");
            out.push_str(fd.unified.trim());
            out.push_str("\n```\n\n");
        }
    }
    out
}

fn role_heading(role: UiRole) -> &'static str {
    match role {
        UiRole::User => "You",
        UiRole::Assistant => "Assistant",
        UiRole::Tool => "Tool",
        UiRole::System => "System",
        UiRole::Error => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::UiMessage;

    #[test]
    fn markdown_skips_sys_and_formats_user() {
        let msgs = vec![
            UiMessage::with_role(UiRole::User, "hello"),
            UiMessage::with_role(UiRole::System, "send · ~1k"),
            UiMessage::with_role(UiRole::Assistant, "hi there"),
        ];
        let md = ui_messages_to_markdown(&msgs);
        assert!(md.contains("## You"));
        assert!(md.contains("hello"));
        assert!(!md.contains("send ·"));
        assert!(md.contains("## Assistant"));
    }
}