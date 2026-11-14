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

## Backlog

- [ ] Codex-style `@` file mentions in composer
- [ ] Codex-style approval overlay for risky `run_command`
- [ ] Optional `config.toml` hot-reload
- [ ] Command allowlist for `run_command`
- [ ] Integration tests against recorded HTTP fixtures
- [ ] `tests/docs_sync.rs` — fail on AGENTS.md structure / TODO drift
- [ ] Patch-style edit tool (optional; diff UI could reuse same renderer)

## Scorecard (2026-11-14)

| Dimension | Score | Note |
|-----------|-------|------|
| Correctness & coverage | 4 | ~26 in-crate tests; no HTTP fixtures yet |
| Agent reliability | 4 | Stream fallback, step cap, interrupt |
| UX (TUI) | 4 | Tools, diff, theme, mouse; no @/approvals |
| Maintainability | 4 | Split modules; `agent.rs` still central |
| Docs alignment | 3→5 | This audit pass |
| Footprint | 3 | syntect + similar + optional ACP |