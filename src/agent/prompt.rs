use std::path::Path;

use crate::tools::ShellPolicy;

pub(crate) fn system_prompt(workdir: &Path, shell: &ShellPolicy) -> String {
    let mut text = format!(
        "lightrs agent. Workdir: {}. Use tools; do not invent file contents. Prefer patch_file for small edits; brief replies.",
        workdir.display()
    );
    if shell.is_restrictive() {
        text.push_str(" Shell allowlist active.");
    }
    if shell.requires_all_commands_approved() {
        text.push_str(" User approves shell commands in TUI.");
    }
    text
}