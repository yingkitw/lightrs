# TODO

## Done

- [x] Scaffold Rust TUI agent with GLM / Kimi / MiniMax
- [x] ACP v1/v2 agent mode (`lightrs acp`, optional via features)
- [x] OpenCode settings import + machine migrate scan
- [x] Streaming assistant tokens in TUI (SSE + live transcript)
- [x] Session resume per workdir (`session.rs`)
- [x] Token usage in header + `/context`
- [x] Codex-style composer (multi-line, history, interrupt, busy spinner)
- [x] Collapsible tool-call transcript (`tool_call` UI; click / `o`)
- [x] Unified diff view for `write_file` / `patch_file` (`similar`, `FileDiffUi`)
- [x] Markdown transcript + syntect + Mermaid ASCII + GFM tables
- [x] Central TUI theme (`tui/theme.rs`)
- [x] Module split (`config/`, `tools/`, `tui/highlight/`, `provider/stream.rs`, `agent/`, `skills/`)
- [x] `tests/docs_sync.rs` — doc/tool/slash alignment
- [x] Recorded HTTP/SSE fixtures (`tests/fixtures/`, `provider_fixtures.rs`)
- [x] Codex-style `@` file mentions + `$skill` attach; `/skills` catalog
- [x] `run_command` allowlist + TUI **y/n** approval (`agent/approval.rs`)
- [x] `/reload`, auto `[shell]` mtime reload, `/quiet` token lines
- [x] Track `MEMORY.md` in git
- [x] `[sys]` transcript styling (`tui/highlight/sys.rs`); ephemeral in session JSON

- [x] `grep` tool — literal workdir search (`tools/grep.rs`; caps on matches/files/bytes)
- [x] Doc audit sync (README/SPEC/ARCHITECTURE/TODO/MEMORY + `docs_sync`)
- [x] Provider stream fixtures (Kimi / GLM / MiniMax SSE shapes in `tests/fixtures/`)

- [x] Optional `syntect` feature — plain code fences when off (`Cargo.toml` `syntect`)

## Backlog

- [ ] Headless `grep` in CI: `cargo test --no-default-features --lib`
- [ ] MCP config stub → document-only or future `lightrs mcp` (no in-process server)
- [ ] Session export slash (`/export` → markdown transcript of user/agent)

## Scorecard (2026-11-14)

| Dimension | Score | Note |
|-----------|-------|------|
| Correctness & coverage | 5 | 64 tests; grep + stream fixtures |
| Agent reliability | 4 | Step cap, interrupt, headless shell policy |
| UX (TUI) | 5 | Tools, diff, markdown, skills, approvals, sys layout |
| Maintainability | 5 | `agent/`, `tui/run/`, `skills/`, shared tool path |
| Docs alignment | 5 | `docs_sync` + root MDs audited to code |
| Footprint | 4 | `--no-default-features` drops ACP + syntect; syntax highlighting optional |

**Weakest next:** record live provider SSE into fixtures (optional) or MCP wiring.