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
    ("src/session.rs", "session.rs"),
    ("src/paths.rs", "paths.rs"),
    ("src/provider/stream.rs", "stream.rs"),
    ("src/tools/diff.rs", "diff"),
    ("src/tui/theme.rs", "theme.rs"),
    ("src/tui/composer.rs", "composer.rs"),
    ("src/tui/mentions.rs", "mentions"),
    ("src/tui/highlight/syntect.rs", "syntect"),
    ("src/tui/highlight/diff.rs", "highlight/diff"),
];

const TOOLS: &[&str] = &["read_file", "write_file", "list_dir", "run_command"];

const SLASH: &[&str] = &["help", "clear", "cwd", "status", "context", "quit"];

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
    let slash = read("src/tui/slash.rs");
    for cmd in SLASH {
        assert!(
            spec.contains(&format!("/{cmd}")),
            "SPEC.md missing /{cmd}"
        );
        assert!(
            slash.contains(&format!("\"{cmd}\"")),
            "slash.rs missing handler for {cmd}"
        );
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