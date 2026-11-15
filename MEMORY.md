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
- TUI liveness: `ObserveEvent::ModelWait` fires before each HTTP stream; `ToolStart`/`ToolEnd` map to `AgentEvent::Progress` + `Refresh`. Busy UI shows phase status, elapsed seconds (≥1s), and streamed char count; 16ms poll keeps spinner moving even with no events.
- **Busy UX testing (formal-ish)**: `tui/run/observe.rs::agent_events_from_observe` is the observe→channel spec; `tui/run/events.rs::apply_stream_refresh` is the table-driven status reducer (preserves `calling model…` / tool labels until tokens arrive). Simulated turn trace test chains both. `tests/tui_busy_spec.rs` pins public `streaming_assistant_chars()` invariants.
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

## Tier 4 UX / reliability (2026-11-15)

- `/compact`: `agent::compact_history` → `context::compact_tool_messages` keeps last **6** tool API bodies; older → `[compacted tool result…]`.
- `/undo`: `session/undo.rs` restores paths from last UI `file_diff` rows via `tools/backup::restore_from_backup` (`.lightrs/backups/*.bak`).
- `require_approval`: `shell_readonly::auto_approve_read_only` skips TUI/headless gate for heuristic read-only pipelines; `shell_deny` still blocks first.
- Tool args: `tools/json_repair.rs` — fences, brace slice, trailing commas before `parse` in `execute_tool`.
- Walk: `tools/gitignore.rs` loaded once per `walk::collect_files` (grep/glob respect `.gitignore` patterns; minimal glob).

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

## Competitive intel (2026-11-15)

- Sibling repo `../craftine` (Rust TUI agent, ~39k lines) + its `minicode/` vendored reference surveyed; curated picks live in `TODO.md` "Competitive intel" section.
- Tier A shipped 2026-11-15 (queued input, interrupt keywords, `/cd`, fallback, `todo_write`, heal, `/editor`, skill `trigger`); remaining items in TODO.
- Reusable patterns: `Arc<dyn ModelClient>`/`ToolExecutor` mock injection for tool-loop tests without HTTP; drain-ready-events-then-paint-once event loop.

## Busy input queue (2026-11-15)

- `App.queued: VecDeque<String>` holds lines submitted while `RunState::Busy`; `tui/run/mod.rs::pump_queued` pops one per turn end (Done/Failed/Interrupted) and re-enters `submit_line` so slash commands still work on queued lines.
- Interrupt keywords (`stop`/`wait`/`halt`, `is_interrupt_keyword`) are intercepted in `handle_key` *before* queueing — they set `cancel_tx` like Esc.
- Gotcha: special `KeyCode::Char` arms (e.g. `'o'` tool-toggle) must sit *before* the generic `Char(c)` editing arm or they're unreachable.

## Heal loop (2026-11-15)

- `agent::turn::maybe_run_heal` runs `[shell].test_command` via `sh -c` after successful write/edit tool results only (`HEAL_TOOLS`; skips `error:` bodies and the `heal_steps` cap).
- Failures are pushed to `api_history` as a synthetic `[heal]` user message (truncated via `context::truncate_tool_api`) so the model fixes them next step — success only adds an ephemeral `[sys]` line.
- `Agent.heal_ran` counts per-turn runs; reset in `run_user_turn*` entry. Config: `test_command`, `heal_steps` (default 3) under `[shell]`.

## Fallback model (2026-11-15)

- `Agent.fallback_client` (same `LlmClient`, different model) built from `--fallback-model` / `fallback_model` config; `fetch_model_message` retries the whole step once on `Err`, swapping `request.model`.
- Retry is per-step, not per-token — a mid-stream failure replays the step's request.

## `/cd` workdir switch (2026-11-15)

- `/cd` arm in `run_slash` (`resolve_cd_target` handles `~`, `~/x`, absolute, relative): saves the session under the *old* workdir hash first, then re-anchors `app.config.workdir` + `Agent::set_workdir` (rebuilds system prompt). Session file follows the new workdir on next save.

## Project slash commands (2026-11-15)

- `src/commands.rs` loads `.claude/commands/*.md` (Claude Code layout): filename stem = `/name`, `description:` frontmatter for hints, body → model prompt (`commands::expand` appends args).
- Dispatch lives in `slash.rs` `other` arm → `SlashAction::Prompt`; `input.rs::kick_turn` is the shared send tail (reload/refresh/hint/busy/kick). Composer history stores the typed `/name args`, not the expanded body.
- Built-ins win collisions — `COMMANDS` match runs first; dedup filter in `tab_complete`/`slash_command_hint` keeps colliding names out.
- Rescan points: `App::refresh_skill_catalog` (each send, `/cd`, `/reload`) + `other` arm itself (so a file added mid-session works immediately).

## Context budget (2026-11-15)

- Latency leak found: `api_history` had no cap — resent every request ×16 steps/turn. Biggest offender was assistant `tool_calls[].function.arguments` (full `write_file`/`multi_edit` bodies), never truncated — tool *result* capping alone doesn't bound the wire.
- `context::compact_tool_call_args` stubs args >512 chars to `"{}"` (stays valid JSON) outside the same `keep_recent=6` tool window as `compact_tool_messages`. `/compact` runs both.
- `Agent::auto_compact_context` runs inside `build_chat_request` (per step): ≥32k est → stub tool bodies+args; ≥64k est → `drop_oldest_turn` prunes whole user turns at `User` boundaries (never splits tool_call/tool pairs, refuses to eat the in-flight turn).
- `build_chat_request` is `&mut self` for this — `run_user_turn_shared` locks the mutex for request build anyway.
- Thresholds are `chars/4` estimate, not real tokenizers — cheap and close enough.

## Session rewind + atomic save (2026-11-15)

- `/rewind n` → `Agent::rewind_turns`: truncate `api_history`/`ui_history` at the nth-from-last `User` index (clamps to first user when fewer turns exist). Truncating at a `User` boundary removes whole turns — same protocol-safety rule as `drop_oldest_turn` (never split tool_call pairs).
- Session `save` writes `<hash>.json.tmp` then `fs::rename` — same-dir rename is atomic; a crash mid-write can't leave truncated JSON. Orphaned `.tmp` after a crash is ignored by `load` (only reads `.json`).
- Rewind also clears `active_user_expansion` — a rewound turn must not carry stale `@`/`$` attach state.

## Shell audit log (2026-11-15)

- `src/audit.rs::log_shell` appends `{ts, tool, command, decision, result}` JSONL to `.lightrs/audit.log` — command redacted via `redact_secrets`, `result` is `result_head` (first nonblank line, 160 chars) not full output.
- Decision labels derive in `execute_one_tool` from `needs_interactive_approval` + gate outcome: `auto` / `approved` / `approved_bypass` / `denied` / `denied_headless`.
- Best-effort semantics: `create_dir_all`/`OpenOptions` errors swallowed — audit failure must never break a tool call.

## Prompt-injection scan (2026-11-15)

- `tools/injection.rs::detect` — lowercase substring match against craftine's curated pattern list (low-FP by design; docs about injection will flag, that's fine — warning not block).
- The flag goes **into** `api_history` (prepended `[warning: …]` prefix in `execute_one_tool`) so the model sees it inline, plus an ephemeral `[sys]` line. Scanning UI-only would leave the model blind.
- Scan points: `execute_one_tool` api push (all tools) + `maybe_run_heal` `[heal]` output — any path where external text enters API history needs it.

## Token transparency (2026-11-15)

- Three visibility layers: **header** (always-on `N msg · ~est · session · last in+out`), **`[sys]` lines** (`send · ~N`, per-step `model · in+out`, turn-close `turn · N calls · in+out` — all gated by `/quiet`), **`/context`** (per-role breakdown `sys/user/asst/tool-out/call-args` + top-3 heaviest messages `role #idx ~tok` + turn usage). `/status` shares the tokens line via `context_details`.
- `TokenStats` carries `last_*` (real API), `session_*` (cumulative real), `turn_*` (reset in `begin_user_turn`, accumulated in `apply_usage` — `turn_calls` counts every model call incl. no-usage ones). New fields need `#[serde(default)]` — TokenStats lives in session JSON.
- Estimate is `chars/4` and self-consistent; comparing `est` vs `last_prompt` (real) in `/context` shows drift without a tokenizer dep.

## Agent-mutex discipline / approval deadlock fix (2026-11-15)

- **Audit found**: `run_user_turn_shared` held the `tokio::Mutex<Agent>` guard across `handle_model_message().await`, which contained (a) the approval oneshot wait → **deterministic TUI deadlock** (draw needs `blocking_lock`, which blocks until the mutex frees — the prompt could never render, approval was never functional in the TUI), and (b) synchronous subprocess exec (`run_command`, heal's 120s `run_command`) → full UI freeze per command.
- **Fix**: split `handle_model_message` into sync record helpers (`record_tool_call_step`, `finish_text_turn`, `record_tool_denial`, `record_tool_outcome`, `heal_plan`/`heal_record`). `run_user_turn_shared` now: resolves `resolve_shell_approvals` **before** locking, then `execute_one_tool_shared` locks only for record sections and runs `execute_tool`/heal via `tokio::task::spawn_blocking` with the mutex free.
- **Rule**: never await UI-bound oneshots or run subprocesses while holding `tokio::Mutex<Agent>` — the TUI thread needs `blocking_lock` every frame. Regression test: `tests/approval_turn.rs` asserts `agent.try_lock().is_ok()` inside the approval responder (mock `TcpListener` serving SSE bodies).
- `ui_history` now capped at `MAX_UI_MESSAGES = 500` (`helpers::trim_ui_history` at turn end + session restore) — was unbounded, persisted in session JSON and rebuilt per frame.

## Approval batching (2026-11-15)

- `resolve_shell_approvals` pre-scans a step's `tool_calls` **before** the exec loop: collects every `run_command` needing `needs_interactive_approval`, sends ONE `CommandApproval { commands: Vec<String>, reply }`. Verdict is all-or-nothing for the step — `Ok(None)` = nothing needed approval OR headless (denial wording picked by `ui_tx.is_none()` in the gate).
- `resolve_shell_gate` is now **sync** — the only `.await` in the approval path is the batch oneshot in `handle_model_message`. Per-call gate just maps `verdict: Option<bool>` → Run/Denied; `Some(true)` + allowlist-blocked → `bypass_allowlist`.
- Audit decisions (`auto`/`approved`/`approved_bypass`/`denied`/`denied_headless`) unchanged — batch approval still reads as `approved*` per call.
- Input-area banner: fixed 5-row block → cap display at 3 command lines + `… +n more`.

## Per-project config overlay (2026-11-15)

- `<workdir>/.lightrs/config.toml` reuses `FileConfig`; applied via `file::shell_from_files` + model/fallback precedence (CLI > project > global). Project `[shell]` replaces global wholesale — `allowlist` is `Vec` not `Option`, so presence-vs-empty can't be distinguished; whole-section replace avoids that ambiguity.
- **Security**: `provider`, `base_url`, `keys`, `workdir` are parsed but never applied — a hostile repo file must not redirect API calls or inject keys. AGENTS.md remains a bigger injection surface regardless; this keeps config from adding another.
- mtime watching: `App::watched_config_mtime` = max(global, project) — project file create/delete both trigger the auto `[shell]` reload before send; `/cd` + `/reload` re-sync `agent.heal` via `sync_heal` (latent gap: `/reload` previously never refreshed heal).

## Patterns to harvest next

- Tool-call JSON differences between GLM / Kimi / MiniMax
- Assistant messages with empty `content` but populated `tool_calls`
- Error body shapes (`ApiErrorBody` vs raw HTML)
- `agent-client-protocol-schema` 1.18 blocked on `agent-client-protocol` 5.2 pin to 1.17