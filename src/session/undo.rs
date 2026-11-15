use std::path::Path;

use anyhow::Result;

use crate::chat::{UiMessage, UiRole};
use crate::tools::restore_from_backup;

/// Files touched by tools since the latest user message (deduped, stable order).
pub fn paths_from_last_turn(ui: &[UiMessage]) -> Vec<String> {
    let start = ui
        .iter()
        .rposition(|m| m.role == UiRole::User)
        .unwrap_or(0);
    let mut out = Vec::new();
    for m in &ui[start..] {
        if let Some(diff) = &m.file_diff {
            if !out.contains(&diff.path) {
                out.push(diff.path.clone());
            }
        }
    }
    out
}

/// Restore backups for files edited on the last agent turn.
pub fn undo_last_turn(workdir: &Path, ui: &[UiMessage]) -> Result<Vec<String>> {
    let paths = paths_from_last_turn(ui);
    if paths.is_empty() {
        anyhow::bail!("no file edits to undo on the last turn");
    }
    let mut notes = Vec::new();
    for rel in paths {
        notes.push(restore_from_backup(workdir, &rel)?);
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::FileDiffUi;

    #[test]
    fn collects_diff_paths_after_last_user() {
        let ui = vec![
            UiMessage::with_role(UiRole::User, "go"),
            UiMessage::tool_result(
                "patch_file",
                "{}",
                "ok",
                Some(FileDiffUi {
                    path: "a.rs".into(),
                    unified: String::new(),
                    lines_added: 1,
                    lines_removed: 0,
                    is_new_file: false,
                }),
            ),
        ];
        assert_eq!(paths_from_last_turn(&ui), vec!["a.rs".to_string()]);
    }
}