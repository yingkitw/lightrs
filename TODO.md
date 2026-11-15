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
- [x] `/export` — markdown transcript under workdir (`session/export.rs`)
- [x] `/export` tests — tool/diff markdown, workdir write, path escape guard
- [x] Fix `/export` default path — `resolve_work_output_path` (file need not exist pre-write)
- [x] Tier 1 sibling carry: shell 64 KiB cap, HTTP retry/timeouts, flexible `patch_file`, context pressure `[sys]`
- [x] Tier 2: `glob_file_search`, grep→`rg` fallback, `exec`, `doctor --check`, `.lightrs/backups`
- [x] Lean API context — composer-only history, one-turn `@`/`$` attach, tool API cap, slimmer skills/rules system block
- [x] Tier 3 (minicode): shell deny, secret redaction, workdir `AGENTS.md`, `multi_edit`
- [x] Tier 4: `/undo` from backups, `/compact`, read-only shell auto-approve, tool JSON repair, `.gitignore` walk
- [x] Tier A craftine batch 1: queued input while `Busy` + interrupt keywords (`stop`/`wait`/`halt`)
- [x] `/cd <path>` — mid-session workdir switch (session saved under old dir, skills/AGENTS.md rescanned)
- [x] Fallback model — `--fallback-model` / `fallback_model` retries once on provider error
- [x] `todo_write` tool + `/todo` — `.lightrs/todo.md` round-trip, `[todo n/done]` status chip
- [x] Heal loop — `[shell].test_command` runs after write/patch/multi_edit; failures fed back to model, `heal_steps` cap (default 3)
- [x] `/editor` — compose prompt in `$EDITOR`, loads into composer on save
- [x] Skill `trigger` hint — frontmatter keyword prints a non-blocking `tip:` line when input matches (no auto-attach)
- [x] `.claude/commands/*.md` → `/name` project slash commands (`src/commands.rs`; body sent to model, args appended)
- [x] Context auto-compact — `tool_calls` arg stubbing + oldest-turn drop (32k/64k est thresholds); `/compact` now stubs args too
- [x] Clippy cleanup — zero warnings on `cargo clippy --all-targets` (collapsed ifs, let-chains, dead `streaming` branch)
- [x] Atomic session save — temp file + same-dir rename (`session/mod.rs`)
- [x] `/rewind [n]` — drop last n user turns from API+UI history at user boundaries, resave session
- [x] `.lightrs/audit.log` — JSONL record per `run_command` decision (`src/audit.rs`, redacted, best-effort)
- [x] Prompt-injection scan — tool output flagged inline for the model + `[sys]` warning (`tools/injection.rs`)
- [x] Per-project config overlay — `.lightrs/config.toml` applies model/fallback/`[shell]` only (provider/base_url/keys ignored); `/reload`, auto-mtime, and `/cd` all re-sync (heal included)
- [x] Approval batching — one y/n prompt covers all `run_command` calls needing approval in a single model step (`agent/approval.rs::resolve_shell_approvals`)
- [x] Audit fix: agent-mutex deadlock + UI freeze — approval/tool exec no longer run under `tokio::Mutex<Agent>` (`execute_one_tool_shared` + `spawn_blocking`); `ui_history` capped at 500 (`helpers::trim_ui_history`); e2e regression `tests/approval_turn.rs`
- [x] Token transparency — `TokenStats` gains per-turn totals (`turn_prompt/completion/calls`, `begin_turn`); `/context` adds per-role breakdown + top-3 consumers + turn in/out; `turn · N calls · X in + Y out` closes each turn; `/status` now shows the tokens line; `/quiet` gates all routine token sys lines
- [x] Wire context window — `messages_for_api` trims the request payload to last **12** user turns / ≤**48k est** (non-destructive; `/rewind`+session keep full history); stored-history auto-compact unchanged (stub 32k / drop 64k raw); one-shot `[sys]` note + `/context` `wire: X of Y turns`

## Backlog

- [x] MCP import documented (stored `mcp.json`; no in-process server in v0.1)

## Scorecard (2026-11-15)

| Dimension | Score | Note |
|-----------|-------|------|
| Correctness & coverage | 5 | 152 tests; queue/interrupt/heal unit coverage + SSE fixtures |
| Agent reliability | 5 | Step cap, interrupt, heal loop, fallback model retry |
| UX (TUI) | 5 | Queued busy input, `/editor`, `/cd`, `/todo`, trigger hints |
| Maintainability | 5 | `agent/`, `tui/run/`, `skills/`, shared tool path |
| Docs alignment | 5 | `docs_sync` + root MDs audited to code |
| Footprint | 4 | `--no-default-features` drops ACP + syntect; syntax highlighting optional |
| Token efficiency | 4 | Auto-compact at 32k/64k est; 6 KiB tool cap; `tool_calls` args stubbed; estimate is `chars/4` (no real tokenizer) |

**Weakest next:** MCP wiring (out of v0.1 scope) or optional live SSE capture into `tests/fixtures/`.

## Brainstorm (unprioritized)

- Release binary size badge in CI (`--no-default-features` vs default)

## Competitive intel — craftine + vendored minicode (2026-11-15)

Sourced from `../craftine` (sibling Rust TUI agent, ~39k lines) and its `craftine/minicode/` vendored reference. Filtered for the lightweight charter — MCP, tree-sitter, vim mode, semantic index, sub-agents, worktrees, pricing tables, browser tools, modes, SDD, hooks system, OS sandbox, keyring deliberately skipped.

### Tier A — cheap, high value

Shipped 2026-11-15 (see Done): queued input while `Busy`, interrupt keywords, `/cd`, fallback model, `todo_write` + `/todo`, heal loop (`[shell].test_command` — config only, no auto-detect), `/editor`, skill `trigger` hints, `.claude/commands/*.md` project slash commands. **Tier A complete.**

### Tier B — moderate weight

- ~~**Approval batching**~~ — shipped: `resolve_shell_approvals` pre-scans the step, one prompt, all-or-nothing verdict.
- ~~**Audit log**~~ — shipped: `.lightrs/audit.log` JSONL per `run_command` decision.
- **Weak model routing** — read-only tools served by a cheaper `[provider].weak_model` until first write/exec; token-frugal (craftine `--weak-model`).
- ~~**Per-project config overlay**~~ — shipped: safe-subset `.lightrs/config.toml` (model/fallback/`[shell]`).
- ~~**Prompt-injection warning**~~ — shipped: `tools/injection.rs` scans tool results; flagged prefix goes into API history + `[sys]` notice.
- ~~**Atomic session save**~~ — shipped: temp file + same-dir rename.

### Patterns worth borrowing (not features)

- **Trait seam for tests** — craftine/minicode inject `Arc<dyn ModelClient>` + `Arc<dyn ToolExecutor>` mocks → tool-loop unit tests with no HTTP. lightrs uses SSE fixtures today; a `LlmClient` trait would enable in-memory turn tests.
- **Event loop batching** — craftine drains all ready events, paints once per wakeup; batches streamed `TextDelta`s. Verify lightrs does the same in `tui/run`.