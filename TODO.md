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
- [x] Unified diff view for `write_file` (`similar`, `FileDiffUi`)
- [x] Markdown transcript + syntect + Mermaid ASCII
- [x] Central TUI theme (`tui/theme.rs`)
- [x] Module split (`config/`, `tools/`, `tui/highlight/`, `provider/stream.rs`)
- [x] `tests/docs_sync.rs` — doc/tool/slash alignment
- [x] Recorded HTTP/SSE fixtures (`tests/fixtures/`, `lib.rs` + `provider_fixtures.rs`)
- [x] Doc audit (ARCHITECTURE, SPEC, TODO, README, MEMORY, AGENTS)
- [x] Codex-style `@` file mentions in composer (`tui/mentions.rs`, Tab complete)
- [x] `run_command` prefix allowlist (`[shell].allowlist` in config)
- [x] Codex-style approval overlay for `run_command` (`require_approval`, allowlist block → **y/n**)
- [x] Split `agent/` (`turn`, `approval`, `prompt`, `helpers`); unified tool execution + ACP fail-closed without TUI
- [x] Markdown transcript: styled inline spans (bold/link/strike), blockquotes, tables, always-on parser
- [x] `/reload` — hot-reload `[shell]` from `config.toml`
- [x] `patch_file` search/replace tool with unified diff in transcript
- [x] Split `tui/run/` (`events`, `input`)
- [x] Auto-reload `[shell]` when `config.toml` mtime changes (before send; `/reload` still manual)
- [x] GFM table headers in markdown transcript (pulldown `TableHead` cells; bold header row)
- [x] `/quiet` — hide per-step token `[sys]` lines (header + `/context` unchanged)
- [x] Agent skills — discover SKILL.md (OpenCode/Cursor/Claude/pi/Codex), system catalog, `$id` attach, `/skills`

## Backlog

- [x] Track `MEMORY.md` in git (removed from `.gitignore`; no secrets in file)
- [ ] `grep` / ripgrep-style search tool (workdir-scoped, size caps)
- [ ] Provider-specific tool-call stream fixtures (GLM/Kimi/MiniMax)

## Scorecard (2026-11-14)

| Dimension | Score | Note |
|-----------|-------|------|
| Correctness & coverage | 5 | 56 tests; skills + fixtures |
| Agent reliability | 4 | Step cap, interrupt, headless shell policy |
| UX (TUI) | 5 | Tools, diff, markdown, @mentions, approvals |
| Maintainability | 5 | `agent/`, `tui/run/`, shared tool path |
| Docs alignment | 5 | SPEC/README/slash sync; MEMORY tracked |
| Footprint | 3 | ~8.5 MB release (--no-default-features); syntect weight |

**Weakest next:** footprint (optional syntect-less build) or docs (track MEMORY in git).