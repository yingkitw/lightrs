use std::path::Path;

use super::workdir_context::load_workdir_context;
use crate::skills::{SkillCatalog, system_appendix};
use crate::tools::ShellPolicy;

pub(crate) fn system_prompt(
    workdir: &Path,
    shell: &ShellPolicy,
    skills: &SkillCatalog,
    read_only: bool,
) -> String {
    let mut text = format!(
        "lightrs agent. Workdir: {}. Use tools for filesystem facts; grep/glob to find paths; patch_file or multi_edit for edits; concise replies.",
        workdir.display()
    );
    if shell.is_restrictive() {
        text.push_str(" Shell allowlist active.");
    }
    if shell.requires_all_commands_approved() {
        text.push_str(" User approves shell commands in TUI.");
    }
    if read_only {
        text.push_str(
            " Plan/read-only mode: file edits and shell commands are disabled — explore, analyze, and propose a plan instead.",
        );
    }
    text.push_str(&load_workdir_context(workdir));
    text.push_str(&system_appendix(skills));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ShellPolicy;

    #[test]
    fn includes_agents_md() {
        let dir = std::env::temp_dir().join(format!("lightrs-prompt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "always run clippy").unwrap();
        let p = system_prompt(&dir, &ShellPolicy::default(), &SkillCatalog::default(), false);
        assert!(p.contains("always run clippy"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}