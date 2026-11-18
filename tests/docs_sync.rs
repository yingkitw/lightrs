//! Guard rails: core docs and tree stay aligned with the binary layout.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// `(source path, substring expected in ARCHITECTURE.md)`.
const ARCHITECTURE_PATHS: &[(&str, &str)] = &[
    ("src/session/mod.rs", "session"),
    ("src/session/export.rs", "export"),
    ("src/paths.rs", "paths.rs"),
    ("src/provider/stream.rs", "stream.rs"),
    ("src/tools/diff.rs", "diff"),
    ("src/tools/grep.rs", "grep"),
    ("src/tui/theme/mod.rs", "theme"),
    ("src/tui/composer.rs", "composer.rs"),
    ("src/tui/mentions.rs", "mentions"),
    ("src/mentions.rs", "mentions"),
    ("src/skills/mod.rs", "skills"),
    ("src/commands.rs", "commands.rs"),
    ("src/audit.rs", "audit.rs"),
    ("src/tools/injection.rs", "injection"),
    ("src/agent/mod.rs", "agent/"),
    ("src/agent/context/mod.rs", "context"),
    ("src/agent/context/wire.rs", "wire"),
    ("src/agent/context/compact.rs", "compact"),
    ("src/agent/context/tokens.rs", "tokens"),
    ("src/agent/turn.rs", "turn"),
    ("src/tui/run/events.rs", "run/"),
    ("src/tui/run/observe.rs", "observe"),
    ("src/tui/editor.rs", "editor.rs"),
    ("src/tools/todo.rs", "todo"),
    ("src/mcp/mod.rs", "mcp"),
    ("src/lsp/mod.rs", "lsp"),
    ("src/tools/sandbox.rs", "sandbox"),
    ("src/tui/highlight/markdown.rs", "markdown"),
    ("src/tui/highlight/syntect.rs", "syntect"),
    ("src/tui/highlight/diff.rs", "highlight/diff"),
    ("src/tui/highlight/sys.rs", "sys.rs"),
];

const TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "patch_file",
    "list_dir",
    "grep",
    "glob_file_search",
    "multi_edit",
    "run_command",
    "todo_write",
    "web_fetch",
    "ask_user",
    "spawn_agent",
    "mcp_call",
    "lsp",
];

const SLASH: &[&str] = &[
    "help", "clear", "cwd", "cd", "todo", "editor", "status", "model", "context", "reload", "quiet",
    "skills", "export", "copy", "diff", "theme", "compact", "rewind", "branch", "undo", "plan",
    "fork", "resume", "quit", "trust",
];

#[test]
fn architecture_paths_exist_and_are_documented() {
    let arch = read("ARCHITECTURE.md");
    for (rel, needle) in ARCHITECTURE_PATHS {
        assert!(
            root().join(rel).is_file(),
            "missing source file {rel}"
        );
        assert!(
            arch.contains(needle),
            "ARCHITECTURE.md should mention `{needle}` (for {rel})"
        );
    }
}

#[test]
fn spec_and_readme_list_tools() {
    let spec = read("SPEC.md");
    let readme = read("README.md");
    let schemas = read("src/tools/schemas.rs");
    for name in TOOLS {
        assert!(schemas.contains(&format!("\"{name}\"")), "schemas.rs missing {name}");
        assert!(spec.contains(name), "SPEC.md missing tool `{name}`");
        assert!(
            readme.contains(name) || readme.contains("read/write"),
            "README should mention tool `{name}` or read/write"
        );
    }
}

#[test]
fn slash_commands_in_spec_and_slash_rs() {
    let spec = read("SPEC.md");
    let slash = read("src/tui/slash/mod.rs");
    for cmd in SLASH {
        assert!(
            spec.contains(&format!("/{cmd}")),
            "SPEC.md missing /{cmd}"
        );
        assert!(
            slash.contains(&format!("\"{cmd}\"")),
            "slash/mod.rs missing handler for {cmd}"
        );
    }
}

/// The ARCHITECTURE slash row and docs/context.md budgets must not drift —
/// both lapsed while this suite was green.
#[test]
fn architecture_lists_all_slash_commands() {
    let arch = read("ARCHITECTURE.md");
    for cmd in SLASH {
        assert!(
            arch.contains(&format!("/{cmd}")),
            "ARCHITECTURE.md slash list missing /{cmd}"
        );
    }
}

/// Pull a `const NAME: usize = N;` numeric value out of source.
fn const_value(src: &str, name: &str) -> u64 {
    for line in src.lines() {
        if line.contains("const ") && line.contains(name) && line.contains('=') {
            let tail = line
                .split('=')
                .nth(1)
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim();
            return tail.replace('_', "").parse().unwrap_or_else(|_| {
                panic!("cannot parse const value for {name}: `{tail}`")
            });
        }
    }
    panic!("const {name} not found in source");
}

#[test]
fn context_doc_matches_source_constants() {
    let doc = read("docs/context.md");
    const CONSTS: &[(&str, &str)] = &[
        ("src/agent/context/wire.rs", "MAX_RESENT_TURNS"),
        ("src/agent/context/wire.rs", "WIRE_BUDGET_TOKENS"),
        ("src/agent/context/mod.rs", "AUTO_COMPACT_TOKENS"),
        ("src/agent/context/mod.rs", "HISTORY_HARD_CAP_TOKENS"),
        ("src/agent/context/compact.rs", "TOOL_API_MAX"),
        ("src/agent/context/compact.rs", "COMPACT_KEEP_TOOLS"),
    ];
    for (rel, name) in CONSTS {
        let val = const_value(&read(rel), name);
        assert!(
            doc.contains(name),
            "docs/context.md should name {name} (from {rel})"
        );
        // The value must appear next to the const name — a bare `12` anywhere
        // in the doc is not proof. Accept `NAME = 48_000` / `NAME = 48000` /
        // `NAME (48k` / `NAME = 48k` forms.
        let raw = val.to_string();
        let underscored = {
            let digits: Vec<char> = raw.chars().collect();
            let mut out = String::new();
            for (i, c) in digits.iter().enumerate() {
                if i > 0 && (digits.len() - i).is_multiple_of(3) {
                    out.push('_');
                }
                out.push(*c);
            }
            out
        };
        let short = format!("{}k", val / 1_000);
        let matched = [
            format!("{name} = {underscored}"),
            format!("{name} = {raw}"),
            format!("{name} ({short}"),
            format!("{name} = {short}"),
        ]
        .iter()
        .any(|needle| doc.contains(needle));
        assert!(
            matched,
            "docs/context.md should show {name} = {underscored} (or {short}) from {rel}"
        );
    }
}

/// Every `tests/*.rs` file must be listed in AGENTS.md's test tree.
#[test]
fn agents_lists_every_integration_test() {
    let agents = read("AGENTS.md");
    for entry in std::fs::read_dir(root().join("tests")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            assert!(
                agents.contains(&name),
                "AGENTS.md test tree missing {name}"
            );
        }
    }
}

#[test]
fn agents_and_spec_reference_same_doc_set() {
    let agents = read("AGENTS.md");
    for doc in ["README.md", "SPEC.md", "ARCHITECTURE.md", "TODO.md", "MEMORY.md"] {
        assert!(agents.contains(doc), "AGENTS.md should reference {doc}");
    }
}

#[test]
fn todo_done_section_not_empty() {
    let todo = read("TODO.md");
    assert!(todo.contains("## Done"), "TODO.md needs ## Done");
    assert!(todo.contains("- [x]"), "TODO.md should track completed items");
}