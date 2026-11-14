use std::path::Path;

use crate::skills::{SkillCatalog, system_appendix};
use crate::tools::ShellPolicy;

pub(crate) fn system_prompt(workdir: &Path, shell: &ShellPolicy, skills: &SkillCatalog) -> String {
    let mut text = format!(
        "lightrs agent. Workdir: {}. Use tools; do not invent file contents. Use grep or glob_file_search to locate files; prefer patch_file for small edits; brief replies.",
        workdir.display()
    );
    if shell.is_restrictive() {
        text.push_str(" Shell allowlist active.");
    }
    if shell.requires_all_commands_approved() {
        text.push_str(" User approves shell commands in TUI.");
    }
    text.push_str(&system_appendix(skills));
    text
}