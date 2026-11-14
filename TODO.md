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
- [x] GitHub CI — full tests + `cargo test --no-default-features --lib` (`.github/workflows/ci.yml`)
- [x] `/export` — markdown transcript under workdir (`session/export.rs`)
- [x] `/export` tests — tool/diff markdown, workdir write, path escape guard
- [x] Fix `/export` default path — `resolve_work_output_path` (file need not exist pre-write)
- [x] Tier 1 sibling carry: shell 64 KiB cap, HTTP retry/timeouts, flexible `patch_file`, context pressure `[sys]`
- [x] Tier 2: `glob_file_search`, grep→`rg` fallback, `exec`, `doctor --check`, `.lightrs/backups`
- [x] Lean API context — composer-only history, one-turn `@`/`$` attach, tool API cap, slimmer skills/rules system block
- [x] Tier 3 (minicode): shell deny, secret redaction, workdir `AGENTS.md`, `multi_edit`

## Backlog

- [x] MCP import documented (stored `mcp.json`; no in-process server in v0.1)

## Scorecard (2026-11-14)

| Dimension | Score | Note |
|-----------|-------|------|
| Correctness & coverage | 5 | 71 tests; CI + export path fix + SSE fixture playbook |
| Agent reliability | 4 | Step cap, interrupt, headless shell policy |
| UX (TUI) | 5 | Tools, diff, markdown, skills, approvals, sys layout |
| Maintainability | 5 | `agent/`, `tui/run/`, `skills/`, shared tool path |
| Docs alignment | 5 | `docs_sync` + root MDs audited to code |
| Footprint | 4 | `--no-default-features` drops ACP + syntect; syntax highlighting optional |

**Weakest next:** MCP wiring (out of v0.1 scope) or optional live SSE capture into `tests/fixtures/`.

## Brainstorm (unprioritized)

- Clippy cleanup batch (27 lib warnings; not CI-blocking yet)
- Release binary size badge in CI (`--no-default-features` vs default)