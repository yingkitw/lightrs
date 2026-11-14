# MEMORY.md

Institutional knowledge for lightrs. Agents should **consult this file before starting work** and **append harvest notes after completing features** (see `AGENTS.md`).

## Providers

_(Empty — add dated entries when you confirm behavior against live or fixture APIs.)_

- **GLM (Z.AI)**: default base `https://api.z.ai/api/paas/v4`; keys `ZAI_API_KEY` / `GLM_API_KEY`.
- **Kimi (Moonshot)**: default `https://api.moonshot.ai/v1`; China endpoint `https://api.moonshot.cn/v1` via `--base-url`.
- **MiniMax**: default `https://api.minimax.io/v1`; key `MINIMAX_API_KEY`.

## Concurrency

- Agent state lives behind `tokio::sync::Mutex<Agent>`; async worker uses `.lock().await`.
- TUI thread uses `blocking_lock()` for draw and slash commands — do not use `std::sync::Mutex` across `.await` in `tokio::spawn`.

## Build features (2026-11-15)

- **Default features**: `syntect` only (TUI + highlighting). ACP is opt-in: `--features full` or `acp,acp-v2`.
- `syntect` uses `regex-fancy` (pure Rust) instead of `regex-onig` to drop the `onig` native dependency.
- `pulldown-cmark` and `clap` built with `default-features = false` where applicable.
- `glob_file_search`: `tools/glob_pattern.rs` (`*`, `?`, `**`); no `glob` crate.
- `cargo build --release --no-default-features` — no syntect (smallest TUI binary).

## grep tool (2026-11-14)

- `tools/grep.rs`: recursive literal search under `validate_work_path`; skip `.git`, `target`, `node_modules`, `.cursor`.
- Limits: 80 matches default (200 max), 2000 files, 256 KiB read per file; `path:line:col:text` output.
- Uses **`rg -F`** when `rg` is on PATH (literal fixed string); else in-tree walk (`tools/walk.rs`).
- **`glob_file_search`**: in-tree glob matcher, max 200 paths, same skip dirs as grep.
- **Backups**: `write_file`/`patch_file` snapshot prior bytes to `.lightrs/backups/<rel>_<ts>.bak` under workdir.
- **CLI**: `lightrs exec -p` (optional `--json`, `--approve-shell`); `lightrs doctor --check` for setup health.

## Tools & security

- Paths must stay under `workdir` after `canonicalize` (`config::validate_work_path` for reads).
- New files / `/export`: `config::resolve_work_output_path` (no pre-existing file; rejects `..` walks).
- `run_command` uses `sh -c` with timeout (default 120s, max 600s); stdout/stderr capped at 64 KiB each (`tools/shell.rs`).
- `patch_file` single replace: `tools/edit_match.rs` after exact match fails (`replace_all` stays exact-only).
- Provider HTTP: `provider/http.rs` — 120s / 30s connect, 3× retry with backoff on 429/5xx and transport errors.
- Context pressure: `CONTEXT_WARN_TOKENS` (= 20_000 est.) → `[sys]` hint via `maybe_context_pressure_sys` (re-warn every +4k est.; `/quiet` skips).
- **Tier 3 (minicode)**: `shell_deny` (hard block), `redact` in `truncate_tool_api`, workdir `AGENTS.md`/`AIDER.md` in system prompt, `multi_edit` (≤16 patches).
- `read_file` default cap 64 KiB (max 512 KiB per request).
- Optional `[shell].allowlist` in config → `ShellPolicy` prefix match on `run_command` (empty list = allow all). Not a substitute for OS sandboxing; blocks obvious out-of-policy commands.
- **Shell deny** (`shell_deny.rs`): catastrophic patterns always rejected before spawn (not approval-bypassable).
- **Redaction** (`redact.rs`): tool API bodies scrubbed in `agent/context::truncate_tool_api`.
- **Workdir prompt**: `AGENTS.md` / `AIDER.md` / `.lightrs/AGENTS.md` in `agent/prompt.rs`.
- **`multi_edit`**: up to 16 `patch_file` ops in one tool call.
- `[shell].require_approval` or allowlist block → TUI **y/n** before `execute_tool`; `agent/approval.rs` + `agent/turn.rs::execute_one_tool`; `AgentEvent::CommandApproval` + oneshot. No `ui_tx` (ACP) → tool error *requires TUI approval*. Approve on blocked allowlist sets `bypass_shell_allowlist` for that call only.

## Session persistence (2026-03-19)

- TUI saves conversation per canonical **workdir** to `~/.config/lightrs/sessions/<hash>.json` after each turn, on exit, and on slash actions (except `/clear`).
- Reload on startup restores API + UI history and token stats; system prompt is refreshed to current workdir.
- `/clear` deletes the session file for the project.

## Streaming (2026-03-19)

- Fixture tests: `tests/fixtures/stream_tool_*.sse` — Kimi may stream `arguments` before `function.name`; GLM may emit assistant `content` then tool deltas; MiniMax may interleave multiple `index` values in one chunk.
- `StreamAccumulator` merges by `tool_calls[].index`; always run `into_assistant_message()` after full SSE replay.

- LLM turns use OpenAI-style SSE (`stream: true` on `/chat/completions`); deltas append to the last assistant `UiMessage` and emit `ObserveEvent::AssistantDelta`.
- If the stream request fails (provider without SSE), the agent falls back to a non-streaming `chat()` call.
- Tool calls are reassembled from streamed `delta.tool_calls` chunks before the tool loop runs.

## Tool transcript UX (2026-11-14)

- One `UiMessage::tool_result` per tool round-trip (`tool_call` metadata); legacy `→ name(...)` rows still render.
- Expand/collapse: `App.expanded_tools`, key `o` (bottom-visible tool) or mouse click on tool line span (`tool_hit_spans` updated each draw).
- `execute_tool` returns `ToolOutcome { body, file_diff }`; API history gets `body` only; UI may attach `FileDiffUi`.

## patch_file (2026-11-14)

- `patch_file`: `old_string` / `new_string` on existing files; unique match unless `replace_all`; same `FileDiffUi` path as `write_file` (`tools/fs.rs`).

## write_file diff (2026-11-14)

- Before write, read prior file text (if any); `tools/diff.rs` builds unified diff via `similar` (3-line context).
- New file: synthetic `--- /dev/null` / all `+` lines.
- Render: `tui/highlight/diff.rs` (+ green, − red, @@ blue); collapsed ~14 lines; theme `palette::DIFF_*`.
- Do not send full diff to the model (keeps context small).

## TUI rendering (2026-11-14)

- Markdown: `pulldown-cmark` in `tui/highlight/markdown.rs`; `mermaid` blocks → ASCII in `mermaid.rs`.
- Code fences: `syntect` (`base16-ocean.dark`), span-preserving wrap.
- Colors: single palette `tui/theme.rs` (Catppuccin-like).

## Provider fixtures (2026-11-14)

- Crate exposes `lib.rs`; binary is thin `main` → `lightrs::run()`.
- `tests/fixtures/*.json` + `.sse` — recorded OpenAI-shaped bodies; no live API in CI.
- `provider::replay_sse_body` replays `data: …` lines the same way as `post_chat_stream` line parsing.
- Extend fixtures when providers return new shapes (tool stream fragmentation, usage-only chunks).

## @ file mentions (2026-11-14)

- Composer: type `@relative/path` (no spaces in fragment); status bar lists matches; `Tab` completes unique or common prefix (`tui/mentions.rs`).
- `list_workdir_paths` walks workdir (cap 4000), skips `.git`, `target`, `node_modules`, `.cursor`.
- On send: `mentions::api_user_content` attaches file bodies to **API** user message only (12 KiB/file, 36 KiB total); transcript shows composer text unchanged.

## Agent module (2026-11-14)

- `src/agent/`: `mod.rs` (lifecycle), `turn.rs` (stream loop + `execute_one_tool`), `approval.rs`, `prompt.rs`, `helpers.rs`, `tokens.rs`.
- Single tool path for TUI (`run_user_turn_shared`) and ACP (`run_user_turn_observed`) via optional `ui_tx`.
- `approval.rs`: `resolve_shell_gate(..., None)` → `ShellGate::Denied` when `require_approval` (headless/ACP fail-closed); covered by `headless_resolve_denies_require_approval` tokio test.

## Markdown transcript (2026-11-14)

- `tui/highlight/markdown.rs`: all message bodies parsed; inline spans preserved through `wrap_styled_lines` (bold/italic/strike/link).
- Blockquotes: `▏` + muted; hard breaks flush paragraph.
- **GFM tables (pulldown-cmark 0.13)**: header cells live under `Tag::TableHead` **without** `TableRow` — collect row on `End(TableHead)`; body rows still use `TableRow`. Alignment rows filtered in `is_alignment_row`. Header rows rendered bold.
- `pad_display_cell`: truncate only when `n > width` (not `>=`) so exact-width cells don’t get ellipsized.

## Skills (2026-11-14)

- `skills/mod.rs`: roots = `~/.config/lightrs/skills`, `~/.cursor/skills`, `~/.cursor/skills-cursor`, `~/.claude/skills`, `~/.agents/skills`, `~/.pi/agent/skills`, `~/.codex/skills` + `[skills].paths`.
- OpenCode `lightrs import opencode` still links into `~/.config/lightrs/skills`; runtime scan picks them up without restart.
- System prompt: compact skill catalog + `rules/*.md` (skip `INDEX.md`). User invokes with `$skill-id`; API message gets `[skills]` blocks + existing `@` attachments (`skills::api_user_content` wraps `mentions::api_user_content`).
- Rescan on send, `/reload`, `/skills` via `App::refresh_skill_catalog`.

## API context trimming (2026-11-15)

- `agent/context.rs` builds wire messages via `messages_for_request`: composer-only user rows in `api_history`; `@` / `$` expansions live in `active_user_expansion` for the **current turn only** (re-sent on each tool step until the assistant replies).
- Older user turns (and legacy sessions): attachment blocks stripped on send; note `[attachments omitted from history]`.
- Tool bodies to API capped at **6144** chars (`truncate_tool_api`); UI tool blocks may show up to **8000**.
- Session save / restore slims user messages with `slim_api_history_for_storage`.
- Skills system appendix: catalog **3500** chars, rules summary **4096** chars; shorter system prompt line.

## Token UX (2026-11-14)

- Per-turn `[sys]` lines: `send · ~N ctx` and model in/out from `agent/turn.rs` via `record_send_estimate` / `record_model_usage`.
- `/quiet` toggles `Agent.quiet_token_sys` (not persisted in session); header + `/context` still show usage.
- Render: `tui/highlight/sys.rs` — `▏ sys` bars; compact token/cwd styling; multi-line slash output with `/cmd` highlights.

## Export (2026-11-14)

- `/export` [path] → `session::export_ui_markdown`; default `lightrs-transcript.md` under workdir; `[sys]` omitted (same as session save filter).
- `session/export.rs` tests: markdown tool/diff sections; disk write; `resolve_work_output_path` rejects `..` escapes (same sandbox as file tools).

## Provider SSE fixtures (2026-11-14)

- Offline replay via `provider::replay_sse_body` in `tests/provider_fixtures.rs` + `tests/fixtures/*.sse`.
- To extend: save provider SSE verbatim, add a focused `replay_sse_body` assertion test (see module doc on `provider_fixtures.rs`).

## Session UI (2026-11-14)

- `session::conversation_ui_history` strips `UiRole::System` on save; `restore_session` filters on load — `[sys]` is live-transcript only, not conversation history on disk.

## Config reload (2026-11-14)

- `/reload` in TUI → `AppConfig::reload_shell_from_file()` + `Agent::set_shell_policy()` (refreshes system message).
- Before each user message send, `App::maybe_reload_shell` compares `config_file_mtime()` and applies `[shell]` when the file changed.
- Model, provider, base URL, and workdir still come from startup CLI/env (restart required).

## Docs sync tests (2026-11-14)

- `tests/docs_sync.rs` checks tool names in `schemas.rs` + SPEC/README, slash commands in `slash.rs` + SPEC, key module paths in ARCHITECTURE, AGENTS doc set.
- Extend `ARCHITECTURE_PATHS` when adding major modules; keeps the loop’s “docs are code” rule testable.

## Patterns to harvest next

- Tool-call JSON differences between GLM / Kimi / MiniMax
- Assistant messages with empty `content` but populated `tool_calls`
- Error body shapes (`ApiErrorBody` vs raw HTML)
- `agent-client-protocol-schema` 1.18 blocked on `agent-client-protocol` 5.2 pin to 1.17