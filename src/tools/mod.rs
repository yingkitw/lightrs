mod backup;
mod diff;
mod edit_match;
mod fs;
mod gitignore;
mod glob_pattern;
mod glob_search;
mod grep;
pub(crate) mod injection;
mod json_repair;
mod multi_edit;
pub(crate) mod redact;
pub(crate) mod shell;
mod shell_deny;
mod shell_policy;
mod shell_readonly;
pub(crate) mod todo;
mod walk;
mod outcome;
mod schemas;

pub use backup::restore_from_backup;
pub use shell::RunCommandArgs;
pub use shell_policy::ShellPolicy;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

pub use outcome::ToolOutcome;
pub use schemas::tool_schemas;

fn finish_body(body: String) -> String {
    redact::redact_secrets(&body)
}

/// Byte-bounded `&s[..max]` that never splits a UTF-8 char (walks back to the
/// previous char boundary). `String::truncate` panics on a mid-char index.
/// Read a text file with an upper bound — instruction/skill files are
/// user-supplied: bound the read at the source, not after a full
/// in-memory load (a 1 GB AGENTS.md shouldn't allocate 1 GB to be
/// truncated to 6 KiB).
pub(crate) fn read_to_string_capped(path: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Create `<workdir>/.lightrs/` and (once) a `.gitignore` keeping
/// machine-generated state out of commits — `config.toml` stays
/// committable on purpose (per-project shell/model policy is a team file).
pub(crate) fn ensure_lightrs_dir(workdir: &Path) -> Option<PathBuf> {
    let dir = workdir.join(".lightrs");
    std::fs::create_dir_all(&dir).ok()?;
    let gi = dir.join(".gitignore");
    if !gi.exists() {
        let _ = std::fs::write(&gi, "audit.log*\nbackups/\ntodos.json\n");
    }
    Some(dir)
}

pub(crate) fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn execute_tool(
    workdir: &Path,
    name: &str,
    arguments: &str,
    shell_policy: &ShellPolicy,
    bypass_shell_allowlist: bool,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
) -> ToolOutcome {
    let outcome = execute_tool_raw(
        workdir,
        name,
        arguments,
        shell_policy,
        bypass_shell_allowlist,
        cancel,
    );
    ToolOutcome {
        body: finish_body(outcome.body),
        file_diff: outcome.file_diff,
    }
}

fn execute_tool_raw(
    workdir: &Path,
    name: &str,
    arguments: &str,
    shell_policy: &ShellPolicy,
    bypass_shell_allowlist: bool,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
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
        "patch_file" => match parse::<fs::PatchFileArgs>(arguments).and_then(|a| fs::patch_file(workdir, a)) {
            Ok(out) => ToolOutcome::with_diff(out.summary, out.file_diff),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "multi_edit" => match parse::<multi_edit::MultiEditArgs>(arguments)
            .and_then(|a| multi_edit::multi_edit(workdir, a))
        {
            Ok(out) => {
                let diff = out.diffs.last().and_then(|d| d.file_diff.clone());
                if let Some(fd) = diff {
                    ToolOutcome::with_diff(out.summary, fd)
                } else {
                    ToolOutcome::ok(out.summary)
                }
            }
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "list_dir" => match parse::<fs::ListDirArgs>(arguments).and_then(|a| fs::list_dir(workdir, a)) {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "grep" => match parse::<grep::GrepArgs>(arguments).and_then(|a| grep::grep(workdir, a)) {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "glob_file_search" => match parse::<glob_search::GlobSearchArgs>(arguments)
            .and_then(|a| glob_search::glob_file_search(workdir, a))
        {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        "run_command" => {
            match parse::<shell::RunCommandArgs>(arguments).and_then(|a| {
                shell::run_command(workdir, a, shell_policy, bypass_shell_allowlist, cancel)
            }) {
                Ok(s) => ToolOutcome::ok(s),
                Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
            }
        }
        "todo_write" => match parse::<todo::TodoArgs>(arguments).and_then(|a| todo::todo_write(workdir, a)) {
            Ok(s) => ToolOutcome::ok(s),
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        },
        other => ToolOutcome::ok(format!("error: unknown tool {other}")),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T> {
    json_repair::parse_tool_json(arguments).context("parse tool arguments JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn ensure_lightrs_dir_writes_gitignore_once() {
        let dir = std::env::temp_dir().join(format!("lightrs-gitign-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        ensure_lightrs_dir(&dir).unwrap();
        let gi = dir.join(".lightrs/.gitignore");
        let text = fs::read_to_string(&gi).unwrap();
        assert!(text.contains("audit.log*") && text.contains("backups/"));
        // User's own .gitignore is never clobbered.
        fs::write(&gi, "custom\n").unwrap();
        ensure_lightrs_dir(&dir).unwrap();
        assert_eq!(fs::read_to_string(&gi).unwrap(), "custom\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn capped_read_bounds_large_files() {
        let dir = std::env::temp_dir().join(format!("lightrs-capread-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("big.md");
        fs::write(&p, "x".repeat(512 * 1024)).unwrap();
        let text = read_to_string_capped(&p, 64 * 1024).unwrap();
        assert_eq!(text.len(), 64 * 1024, "read exceeded cap");
        assert!(read_to_string_capped(&dir.join("nope.md"), 1024).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn allowlist_blocks_shell() {
        let policy = ShellPolicy::new(vec!["cargo ".into()], false);
        let args = r#"{"command":"curl http://evil"}"#;
        let out = execute_tool(Path::new("."), "run_command", args, &policy, false, None);
        assert!(out.body.contains("allowlist"));
    }

    #[test]
    fn patch_replaces_unique_snippet() {
        let dir = std::env::temp_dir().join(format!("lightrs-patch-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "alpha\nbeta\n").unwrap();
        let args = r#"{"path":"a.txt","old_string":"beta\n","new_string":"gamma\n"}"#;
        let out = execute_tool(&dir, "patch_file", args, &ShellPolicy::default(), false, None);
        assert!(out.file_diff.is_some());
        let text = fs::read_to_string(dir.join("a.txt")).unwrap();
        assert_eq!(text, "alpha\ngamma\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn patch_flexible_indent() {
        let dir = std::env::temp_dir().join(format!("lightrs-flex-patch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.rs"), "    fn foo() {\n        bar();\n    }\n").unwrap();
        let args = r#"{"path":"a.rs","old_string":"fn foo() {\n    bar();\n}\n","new_string":"fn foo() {\n    baz();\n}\n"}"#;
        let out = execute_tool(&dir, "patch_file", args, &ShellPolicy::default(), false, None);
        assert!(out.body.contains("wrote"));
        let text = fs::read_to_string(dir.join("a.rs")).unwrap();
        assert!(text.contains("baz();"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_edit_applies_two() {
        let dir = std::env::temp_dir().join(format!("lightrs-multi-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "one\n").unwrap();
        fs::write(dir.join("b.txt"), "two\n").unwrap();
        let args = r#"{"edits":[{"path":"a.txt","old_string":"one\n","new_string":"ONE\n"},{"path":"b.txt","old_string":"two\n","new_string":"TWO\n"}]}"#;
        let out = execute_tool(&dir, "multi_edit", args, &ShellPolicy::default(), false, None);
        assert!(out.body.contains("1."));
        assert!(out.body.contains("2."));
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "ONE\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_deny_blocks_root_rm() {
        let dir = std::env::temp_dir();
        let args = r#"{"command":"rm -rf /"}"#;
        let out = execute_tool(&dir, "run_command", args, &ShellPolicy::default(), false, None);
        assert!(out.body.contains("denied"));
    }

    #[test]
    fn parse_repairs_trailing_comma() {
        let dir = std::env::temp_dir().join(format!("lightrs-json-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let args = r#"{"path":"x.txt","content":"hi",}"#;
        let out = execute_tool(&dir, "write_file", args, &ShellPolicy::default(), false, None);
        assert!(out.body.contains("wrote"), "{}", out.body);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_emits_diff() {
        let dir = std::env::temp_dir().join(format!("lightrs-diff-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "old\n").unwrap();
        let args = r#"{"path":"a.txt","content":"new\n"}"#;
        let out = execute_tool(&dir, "write_file", args, &ShellPolicy::default(), false, None);
        assert!(out.file_diff.is_some());
        let fd = out.file_diff.unwrap();
        assert_eq!(fd.lines_added, 1);
        assert_eq!(fd.lines_removed, 1);
    }
}