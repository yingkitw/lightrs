use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::config::resolve_work_output_path;

const DEFAULT_NAME: &str = "lightrs-transcript.md";

pub fn export_ui_markdown(
    agent: &Agent,
    workdir: &Path,
    rel_path: Option<&str>,
) -> Result<PathBuf> {
    let rel = rel_path.unwrap_or(DEFAULT_NAME);
    let path = resolve_work_output_path(workdir, Path::new(rel))?;
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
    use std::fs;
    use std::sync::Arc;

    use super::*;
    use crate::agent::Agent;
    use crate::chat::{FileDiffUi, UiMessage, UiRole};
    use crate::provider::LlmClient;
    use crate::tools::ShellPolicy;

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

    #[test]
    fn markdown_includes_tool_and_diff() {
        let msgs = vec![UiMessage::tool_result(
            "grep",
            r#"{"pattern":"fn main"}"#,
            "found 1 match",
            Some(FileDiffUi {
                path: "src/main.rs".into(),
                unified: "+fn main() {}\n".into(),
                lines_added: 1,
                lines_removed: 0,
                is_new_file: false,
            }),
        )];
        let md = ui_messages_to_markdown(&msgs);
        assert!(md.contains("**tool:** `grep`"));
        assert!(md.contains("```json"));
        assert!(md.contains("fn main"));
        assert!(md.contains("### diff: `src/main.rs` (+1 −0)"));
        assert!(md.contains("```diff"));
    }

    #[test]
    fn export_writes_default_file_under_workdir() {
        let dir = std::env::temp_dir().join(format!("lightrs-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let client = LlmClient::new(
            "http://127.0.0.1".into(),
            "test".into(),
            "fixture".into(),
        );
        let mut agent = Agent::new(client, Arc::from(dir.as_path()), ShellPolicy::default());
        agent
            .ui_history
            .lock()
            .unwrap()
            .push(UiMessage::with_role(UiRole::User, "export me"));

        let path = export_ui_markdown(&agent, &dir, None).unwrap();
        assert_eq!(path, dir.join("lightrs-transcript.md"));
        let md = fs::read_to_string(&path).unwrap();
        assert!(md.contains("export me"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_rejects_path_escape() {
        let dir = std::env::temp_dir().join(format!("lightrs-export-guard-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let client = LlmClient::new(
            "http://127.0.0.1".into(),
            "test".into(),
            "fixture".into(),
        );
        let agent = Agent::new(client, Arc::from(dir.as_path()), ShellPolicy::default());
        let err = export_ui_markdown(&agent, &dir, Some("../outside.md"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("escapes workdir") || err.contains("path not found"));

        let _ = fs::remove_dir_all(&dir);
    }
}