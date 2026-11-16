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
- **Redaction** (`redact.rs`): tool API bodies scrubbed in `agent/context/compact::truncate_tool_api`.
- **Workdir prompt**: `AGENTS.md` / `AIDER.md` / `.lightrs/AGENTS.md` in `agent/prompt.rs`.
- **`multi_edit`**: up to 16 `patch_file` ops in one tool call.
- `[shell].require_approval` or allowlist block → TUI **y/n** before `execute_tool`; `agent/approval.rs` + `agent/turn.rs::execute_one_tool_shared`; `AgentEvent::CommandApproval` + oneshot. No `ui_tx` (ACP) → tool error *requires TUI approval*. Approve on blocked allowlist sets `bypass_shell_allowlist` for that call only.

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

- `src/agent/`: `mod.rs` (lifecycle), `turn.rs` (stream loop + `execute_one_tool_shared`), `approval.rs`, `prompt.rs`, `helpers.rs`, `tokens.rs`.
- Single turn loop `run_user_turn_shared` for TUI + `run_user_turn_headless` for exec/ACP via optional `ui_tx`.
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

- `agent/context/` (`mod`/`wire`/`compact`/`tokens`) builds wire messages via `messages_for_request`: composer-only user rows in `api_history`; `@` / `$` expansions live in `active_user_expansion` for the **current turn only** (re-sent on each tool step until the assistant replies).
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

- `agent::turn` heal stage (`heal_plan`/`heal_record`) runs `[shell].test_command` via `sh -c` after successful write/edit tool results only (`HEAL_TOOLS`; skips `error:` bodies and the `heal_steps` cap).
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
- **Two-layer context model**: `messages_for_api` bounds the *wire* copy — last **12** user turns (`MAX_RESENT_TURNS`) then `drop_oldest_turn` until ≤ **48k est** (`WIRE_BUDGET_TOKENS`); `api_history` is never trimmed there so `/rewind` + session save keep everything. `auto_compact_context` shrinks *stored* history (stub 32k / drop 64k) and must measure `api_history` **raw** — `refresh_context_estimate` reports the wire view, so tests asserting raw size need `estimate_context_tokens(&agent.api_history)` directly.
- Wire-window transparency: one-shot `[sys]` via `wire_window_noted` flag (reset in `clear_conversation`); `/context` leads with `wire: X of Y turns` — windowing is visible, not silent.

- **Weak-model routing** (craftine pattern, adapted): `Agent.weak_model` + `weak_active` (reset per `begin_user_turn`); `build_chat_request` picks `weak_model` while `weak_active`; `escalate_from_weak(&calls)` flips to primary when a step's tool_calls include `write_file`/`patch_file`/`multi_edit`/`run_command` — the weak-decided write still executes, only later reasoning escalates. Same-provider model-name swap (no provider split — all lightrs providers are OpenAI-compatible). Included in the `.lightrs/config.toml` safe subset.

- **Tool body rendering**: shell output is literal text — render via `highlight::render_pre_body` (plain wrap + `strip_ansi`), never markdown (mangles `#`/`|`/`---`/backticks). `tools::shell::strip_ansi` also runs at capture → ANSI junk never reaches API history (token savings). When inserting a `#[test]` before an existing test fn, keep the attribute on BOTH.
- **Live tool rows**: `UiMessage.pending` (`#[serde(default, skip_serializing_if=is_false)]`) + `Agent.pending_tool = (ui_idx, Instant)` — pushed at ToolStart, replaced **in place** by `record_tool_denial`/`record_tool_outcome` (keeps `expanded_tools` idx stable). Three unstick paths needed: normal replace, `finish_user_turn_api` sweep (interrupt/error → `(interrupted)` + elapsed), `restore_session` sweep (crash mid-tool would restore `◌` forever). `elapsed_ms` renders via `chat::fmt_elapsed`; `turn_started` feeds the `· Ns` close-line suffix.
- **`ModelClient` seam** (`provider.rs`): object-safe trait (`model()` + `send(request, on_delta, cancel) -> BoxFuture<Result<ModelReply>>`) — hand-rolled `Pin<Box<dyn Future>>`, no `async-trait` dep. `Agent::new`/`set_fallback_client` take `impl ModelClient + 'static` and wrap in `Arc<dyn>` — call sites unchanged, tests inject `MockModel` (canned FIFO replies + `requests` log). SSE→non-stream fallback lives inside `LlmClient::send`; UI wiring (`ModelWait`, `append_assistant_delta`) stays in `turn.rs`. Interrupt sentinel `provider::INTERRUPTED` shared by stream.rs/turn.rs/events.rs — check via `provider::is_interrupted`. `on_delta` needs `+ Send` (future is awaited under tokio tasks); `FnMut(ObserveEvent)` params need `+ Send` too; no-op test observers pass `&mut |_| {}` inline so closure inference picks the HRTB.

## Context module extraction (2026-11-15)

- `agent/context.rs` → `agent/context/` domain module: `wire.rs` (request window: `MAX_RESENT_TURNS=12`, `WIRE_BUDGET_TOKENS=48k`), `compact.rs` (stored-history stubs/drops: `TOOL_API_MAX=6144`, `COMPACT_KEEP_TOOLS=6`), `tokens.rs` (chars/4 estimate, `TokenStats`, breakdown/top/usage lines), `mod.rs` (re-exports + `Agent` context-policy methods + thresholds `AUTO_COMPACT_TOKENS=32k`, `HISTORY_HARD_CAP_TOKENS=64k` — lifted to module consts so `/context`'s `limits:` line can't drift).
- Thresholds live as named consts next to the policy that uses them; `/context` renders `limits:` from the same consts — never hardcode the numbers in display strings.
- `tests/support/mod.rs` — shared `TcpListener` mock (`serve_bodies`) captures raw request bodies; `tests/context_budget.rs` asserts the *wire* payload (12-user-turn cap, `"{}"` arg stubs, `[compacted tool result]` markers), `tests/approval_turn.rs` uses it for the mutex-free approval regression. Reuse it for any test needing real `LlmClient` traffic.
- Full policy spec: `docs/context.md` (linked from README + SPEC `/context` row).

## Session rewind + atomic save (2026-11-15)

- `/rewind n` → `Agent::rewind_turns`: truncate `api_history`/`ui_history` at the nth-from-last `User` index (clamps to first user when fewer turns exist). Truncating at a `User` boundary removes whole turns — same protocol-safety rule as `drop_oldest_turn` (never split tool_call pairs).
- Session `save` writes `<hash>.json.tmp` then `fs::rename` — same-dir rename is atomic; a crash mid-write can't leave truncated JSON. Orphaned `.tmp` after a crash is ignored by `load` (only reads `.json`).
- Rewind also clears `active_user_expansion` — a rewound turn must not carry stale `@`/`$` attach state.

## Shell audit log (2026-11-15)

- `src/audit.rs::log_shell` appends `{ts, tool, command, decision, result}` JSONL to `.lightrs/audit.log` — command redacted via `redact_secrets`, `result` is `result_head` (first nonblank line, 160 chars) not full output.
- Decision labels derive in `execute_one_tool_shared` from `needs_interactive_approval` + gate outcome: `auto` / `approved` / `approved_bypass` / `denied` / `denied_headless`.
- Best-effort semantics: `create_dir_all`/`OpenOptions` errors swallowed — audit failure must never break a tool call.

## Prompt-injection scan (2026-11-15)

- `tools/injection.rs::detect` — lowercase substring match against craftine's curated pattern list (low-FP by design; docs about injection will flag, that's fine — warning not block).
- The flag goes **into** `api_history` (prepended `[warning: …]` prefix in `execute_one_tool_shared`) so the model sees it inline, plus an ephemeral `[sys]` line. Scanning UI-only would leave the model blind.
- Scan points: `execute_one_tool_shared` api push (all tools) + heal-loop `[heal]` output (`heal_record`) — any path where external text enters API history needs it.

## Token transparency (2026-11-15)

- Three visibility layers: **header** (always-on `N msg · ~est · session · last in+out`), **`[sys]` lines** (`send · ~N`, per-step `model · in+out`, turn-close `turn · N calls · in+out` — all gated by `/quiet`), **`/context`** (per-role breakdown `sys/user/asst/tool-out/call-args` + top-3 heaviest messages `role #idx ~tok` + turn usage). `/status` shares the tokens line via `context_details`.
- `TokenStats` carries `last_*` (real API), `session_*` (cumulative real), `turn_*` (reset in `begin_user_turn`, accumulated in `apply_usage` — `turn_calls` counts every model call incl. no-usage ones). New fields need `#[serde(default)]` — TokenStats lives in session JSON.
- Estimate is `chars/4` and self-consistent; comparing `est` vs `last_prompt` (real) in `/context` shows drift without a tokenizer dep.

## Agent-mutex discipline / approval deadlock fix (2026-11-15)

- **Audit found**: `run_user_turn_shared` held the `tokio::Mutex<Agent>` guard across `handle_model_message().await`, which contained (a) the approval oneshot wait → **deterministic TUI deadlock** (draw needs `blocking_lock`, which blocks until the mutex frees — the prompt could never render, approval was never functional in the TUI), and (b) synchronous subprocess exec (`run_command`, heal's 120s `run_command`) → full UI freeze per command.
- **Fix**: split `handle_model_message` into sync record helpers (`record_tool_call_step`, `finish_text_turn`, `record_tool_denial`, `record_tool_outcome`, `heal_plan`/`heal_record`). `run_user_turn_shared` now: resolves `resolve_shell_approvals` **before** locking, then `execute_one_tool_shared` locks only for record sections and runs `execute_tool`/heal via `tokio::task::spawn_blocking` with the mutex free.
- **Rule**: never await UI-bound oneshots or run subprocesses while holding `tokio::Mutex<Agent>` — the TUI thread needs `blocking_lock` every frame. Regression test: `tests/approval_turn.rs` asserts `agent.try_lock().is_ok()` inside the approval responder (mock `TcpListener` serving SSE bodies).
- `ui_history` now capped at `MAX_UI_MESSAGES = 500` (`helpers::trim_ui_history` at turn end + session restore) — was unbounded, persisted in session JSON and rebuilt per frame.

## Single turn loop (2026-11-15)

- After the deadlock fix there were two parallel loops (`run_user_turn_shared` Arc<Mutex> + `drive_turn_loop` `&mut self`) duplicating the whole gate→exec→record→heal orchestration — a drift hazard (the deadlock itself came from them diverging). Ponytail pass deleted the `&mut self` path entirely: `run_user_turn`, `run_user_turn_observed`, `drive_turn_loop`, `handle_model_message`, sync `execute_one_tool_shared` (~160 lines) are gone.
- Single entry: `run_user_turn_shared(agent, text, on, cancel, ui_tx: Option<&Sender>)`; `run_user_turn_headless` wraps it for exec/ACP (no cancel, `ui_tx: None` → headless-denied approvals). ACP sessions store `Arc<Mutex<Agent>>` and clone the Arc under the sessions lock — the map lock is no longer held across the turn.
- Bonus fix found while building: `acp/mod.rs` had a `#[cfg(not(acp-v2))]` arm matching `AcpProtocol::V2` — a variant that doesn't exist without the feature, so `--features acp` alone never compiled. Dead arm deleted.

## Approval batching (2026-11-15)

- `resolve_shell_approvals` pre-scans a step's `tool_calls` **before** the exec loop: collects every `run_command` needing `needs_interactive_approval`, sends ONE `CommandApproval { commands: Vec<String>, reply }`. Verdict is all-or-nothing for the step — `Ok(None)` = nothing needed approval OR headless (denial wording picked by `ui_tx.is_none()` in the gate).
- `resolve_shell_gate` is **sync** — the only `.await` in the approval path is the batch oneshot in `run_user_turn_shared`. Per-call gate just maps `verdict: Option<bool>` → Run/Denied; `Some(true)` + allowlist-blocked → `bypass_allowlist`.
- Audit decisions (`auto`/`approved`/`approved_bypass`/`denied`/`denied_headless`) unchanged — batch approval still reads as `approved*` per call.
- Input-area banner: fixed 5-row block → cap display at 3 command lines + `… +n more`.

## Per-project config overlay (2026-11-15)

- `<workdir>/.lightrs/config.toml` reuses `FileConfig`; applied via `file::shell_from_files` + model/fallback precedence (CLI > project > global). Project `[shell]` replaces global wholesale — `allowlist` is `Vec` not `Option`, so presence-vs-empty can't be distinguished; whole-section replace avoids that ambiguity.
- **Security**: `provider`, `base_url`, `keys`, `workdir` are parsed but never applied — a hostile repo file must not redirect API calls or inject keys. AGENTS.md remains a bigger injection surface regardless; this keeps config from adding another.
- mtime watching: `App::watched_config_mtime` = max(global, project) — project file create/delete both trigger the auto `[shell]` reload before send; `/cd` + `/reload` re-sync `agent.heal` via `sync_heal` (latent gap: `/reload` previously never refreshed heal).

## Mouse reporting vs native copy (2026-11-15)

- Crossterm `EnableMouseCapture` = `?1000h;?1002h;?1006h` — `?1002` drag reporting is what kills terminal drag-select (the app eats every drag, copy impossible). lightrs never uses `Drag`/`Moved`/`Up` events, so it writes **`tui::{MOUSE_ON,MOUSE_OFF}`** (`?1000h;?1006h` only) directly instead — clicks + wheel still reported, drags fall back to native selection (Cmd+C copy). Do NOT "fix" back to `EnableMouseCapture`.
- Applied in `run/mod.rs` (enter/exit) and `editor.rs` (suspend/resume for `$EDITOR`); `MOUSE_OFF` also clears `?1002` in case a prior session leaked it.
- Paste path is separate and already worked: `EnableBracketedPaste` → `Event::Paste` → composer `insert_text`.

## TUI header/status signals (2026-11-15)

- `tui::info::git_branch_label(dir)` reads `.git/HEAD` directly — worktree-safe (`.git` file → `gitdir:` path, absolute or relative) and detached-HEAD → 7-char sha. Never spawn `git` per frame; cache in `App.git_branch`, refresh only on `/cd`.
- `config::model_context_window(model)` — static prefix table (longest match): glm-4.6/M2.1 → 204_800, kimi-k2-0905 → 262_144, glm-4.5/kimi-k2-0711 → 131_072, unknown → `None`. Fed to `context_usage_text` → `ctx est (N%)`; `None` omits the `%` rather than guessing.
- Status line when `app.scroll > 0`: `↑N scrolled · End latest` — the affordance appears only when needed.

## Mermaid ASCII layout (2026-11-15)

- `tui/highlight/mermaid.rs` renders real diagrams: Kahn longest-path ranks → layers, barycenter ordering, `Canvas` char grid. Key invariant: `put_line` never writes into `blocked` (box) cells — edges pass *behind* boxes; `junction()` merges overlapping line cells into `┬`/`├`/`┼` from a neighbor bitmask.
- **Gotcha**: `wrap_plain_lines` word-wraps via `split_whitespace` — it destroys ASCII-art indentation. `MermaidView.pre_wrapped` rows bypass it (emit `Line::styled` directly). Any preformatted block must not go through the word-wrap path.
- Panel styling: `Canvas.kind` grid (0=edge, 1=box border, 2=box label) survives layout → `segment_row` splits each row into styled spans over the `code_bg`/`inline_code_bg`/`muted`/`text` ladder, padded to a solid rectangle. Reuses existing palette colors — no new theme fields.
- Fallbacks → flat edge list: cycles (Kahn leaves nodes unprocessed), `BT`/`RL` directions, `width < 24` or canvas > width, >48 nodes. `flowchart`/`graph` default to TD when no direction keyword.

## Release footprint (2026-11-15)

- `[profile.release]` had **no** LTO/strip — adding `lto = true` + `codegen-units = 1` + `strip = "symbols"` cut the binary ~32%: default 11M→7.4M, `--no-default-features` 8.1M→5.6M, `--features full` →9.0M. `panic = "abort"` was skipped on purpose — the TUI installs a panic hook to restore the terminal, and abort kills unwinding before the hook path runs. `opt-level = "z"` also skipped — trades render/regex speed for a marginal size win.

## TUI terminal integration (2026-11-15)

- **OSC title**: `\x1b]2;…\x07` in `tui::title_escape` — compare with last emitted, write only on change (once per loop pass is enough; events that flip state always pass through the same loop). Clear with an empty title on exit so the shell takes over. Always strip control chars from user-influenced pieces (dir/branch names) — a `\x1b` or `\x07` in a branch name would break the escape or inject sequences.
- **Event-loop batching (verified, don't regress)**: `tui/run` draws once at loop top, drains `rx.try_recv()` in batches, and `redraw_immediately` continues the loop without polling input while events are queued — streamed deltas get one paint per wakeup, not per delta. If this pattern is ever refactored, keep the drain-then-paint ordering.
- **Composer history nav**: `Composer.draft` snapshots in-progress input on first Ctrl+P and `recall_newer` returns it at the newest end — don't regress to replacing input in place (opencode's "dirty composer" bug class).

## Session change review (2026-11-15, aider intel)

- `/diff` replays **stored `FileDiffUi`s** as tool rows instead of recomputing diffs — the write-time diff is already the source of truth, zero fs reads, and the existing tool-row renderer (collapse/preview/color) does all the display work. When a review-style command needs data that was already rendered once, prefer replaying stored UI data over recomputing from disk (backups are for *restore*, not display).
- New slash command checklist: `COMMANDS` entry → handler arm → `docs_sync::SLASH` → SPEC table + ARCH slash row + README enumeration. The suite fails on any miss.

## Coverage-driven testing (2026-11-16)

- **`cargo llvm-cov --summary-only`** = the real per-file number; `--lib` alone undercounts (integration tests add ~4%). Rank by *missed lines*, not %. First pass: 68.6%→73.9%.
- **Ratatui `TestBackend`** makes draw paths testable without a pty: `Terminal::new(TestBackend::new(w,h))` → `term.draw(|f| draw(f, &mut app, &agent))` → scan `buffer()[(x,y)].symbol()` into a string, assert substrings. One smoke test covers header+transcript+chrome at once.
- **Border-eats-a-row bug class**: `Paragraph::new(text).block(Block::borders(BOTTOM))` in a `Length(n)` area renders only n−1 text rows — the border row comes out of the same allocation. It silently clipped `header_lines[1]` for months; string-builder unit tests can't see it, only a rendered frame can. When a widget has borders, always `+1..+2` the constraint.
- Fixture pattern for TUI tests: `App::new(AppConfig{...})` + `Agent::new(LlmClient::new("http://127.0.0.1",...))` + `run_slash`/`draw` directly — no network ever contacted.
- Temp dirs: `std::env::temp_dir() + pid`, `create_dir_all` first, `remove_dir_all` at end (no tempfile dep, by convention).

## Latent bugs found by coverage tests (2026-11-16)

- **Cross-canonicalization**: any `canonicalize()` on one side of a `starts_with`/strip comparison while the other stays literal breaks on macOS (`/var`→`/private/var`) — grep scanned 0 files under `path=` when workdir was non-canonical. Rule: canonicalize once at the API boundary (`grep()`), same as `validate_work_path` does internally.
- **Token misalignment after peeling**: `shell_readonly` peeled `KEY=val` env prefixes into `s` but classified git/cargo/find on the original `segment` — `tokens[1]` was the env pair, so `FOO=bar git status` was rejected while `FOO=bar ls` passed. When a parser produces a normalized remainder, downstream classification must use *it*.
- **Cap-eviction ordering**: `skills::load` scanned ambient roots first, so a machine with ≥64 skills silently dropped `extra_paths`. Declared/configured sources should be scanned before ambient discovery — order *is* precedence here (`or_insert` keeps the first).
- Test-writing uncovered these precisely because the paths had never executed — good tests assert behavior, then investigate surprises rather than fitting expectations.

## Patterns to harvest next

- Tool-call JSON differences between GLM / Kimi / MiniMax
- Assistant messages with empty `content` but populated `tool_calls`
- Error body shapes (`ApiErrorBody` vs raw HTML)
- `agent-client-protocol-schema` 1.18 blocked on `agent-client-protocol` 5.2 pin to 1.17
## Mid-session model switch + cli e2e (2026-11-16)

- **`/model` swaps the whole client, not a field**: `Agent.client` is `Arc<dyn ModelClient>` — `set_client(new LlmClient)` replaces it (mirrors `set_fallback_client`); header + context % read `client.model()` at render so nothing else needs syncing. `AppConfig` in `App` is the source of base_url/api_key for the rebuild — the agent doesn't store them.
- **Provider switch re-resolves the key** via `AppConfig::resolve_runtime_api_key` (global file + env, no CLI flags — flags only applied at startup); fails fast naming the missing env var rather than half-switching. Same-provider switch keeps custom `--base-url` + session key.
- **Path-seam for HOME-bound code**: `session::load/save/remove` hardcode `session_path()`; splitting each into a pub wrapper + private `load_file(path, workdir)` / `save_to(agent, workdir, path)` / `remove_file(path)` made every reject branch (version, workdir mismatch, empty history, malformed) unit-testable in temp dirs — 6-line cost, 41%→90% coverage.
- **Spawned-binary tests cover what unit tests can't**: anything touching `config_path()`/`HOME` (opencode import orchestrator, migrate, exec) goes in `tests/cli.rs` with isolated `HOME` + cleared key env vars (`run_isolated`). `env!("CARGO_BIN_EXE_lightrs")` gives the built binary; support mock serves SSE.
- llvm-cov after pass 3: 79.4% total. Still thin: `tui/run/mod` (pty event loop, ~25%), `migrate/` (mostly spawnable paths), `acp/` (feature-gated, needs protocol frames).

## Coverage pass 4 findings (2026-11-16)

- **Substring `replace` without segment awareness**: `base.replace("/anthropic", "/api/paas/v4")` on `https://api.z.ai/api/anthropic` produced `…/api/api/paas/v4` — the anchor matched mid-path. Handle the longer segment first (`/api/anthropic` → `/api/paas/v4`) or use `strip_suffix`. Latent until `migrate/profile.rs` got tests.
- **`app.scroll` is distance-from-bottom**: `scroll_up` *adds*, `scroll_down` subtracts, `scroll_to_top` = `u16::MAX` (draw clamps), `scroll_to_bottom` = 0 + `pin_bottom`. Don't assume top=0 when writing tests.
- **Synthetic `crossterm::event::Event`s make the whole keymap testable** without a pty: `Event::Key(KeyEvent::new(code, mods))`, `Event::Paste`, `Event::Mouse(MouseEvent{kind,column,row,modifiers})` → `handle_key`/`handle_crossterm_event` + channel assertions (`oneshot` for approvals, `watch` for interrupt, `mpsc` for kicks).
- **`Agent::from_config(cfg, workdir)`** is the shared client bootstrap (primary + fallback + weak). TUI `run` and ACP `make_agent` both call it — don't re-add inline `LlmClient` wiring in a third caller.
- Env-dependent assertions (real API keys may exist on a dev box): assert the *contract*, not the branch — `Ok(key)` nonempty OR `Err` names the env var; never `env::remove_var` in parallel tests (races).
- llvm-cov note: `--summary-only` prints three `(total missed %)` triplets — lines coverage is the *third* ($8/$9/$10 when awk-split).

## Coverage pass 5 findings (2026-11-16)

- **`run_slash` rescans `.claude/commands/` on every unknown `/name`** (`other` arm reloads `app.commands` from disk) — injecting `app.commands` in a test is silently overwritten. Write a real `.claude/commands/<name>.md` into the fixture workdir instead. `expand` = body + "\n\n" + args; `SlashAction::Prompt` flows to `submit_line` → `kick_turn` (composer commits the *typed* line, model gets the *expanded* body).
- `tui/slash.rs` 61%→86% via the `fixture::build` pattern for every handler; remaining misses are `/editor` ($EDITOR-bound) and error paths needing real session files.
- `git diff` after big batches still the cheapest sanity: 333 tests, **84.99%** lines total.

## Coverage pass 6 findings (2026-11-16)

- **HOME-bound orchestrators are fully testable via spawned binary + fabricated fixtures**: `migrate::run_migrate`/`bootstrap_if_needed` scan `~/.claude`, `~/.config/opencode`, etc. — write the fake tool config under the isolated `HOME` in `tests/cli.rs` and run the real subcommand. `doctor --check` doubles as a bootstrap e2e: `AppConfig::load(auto_migrate)` → `bootstrap_if_needed` → writes config → doctor runs on the imported profile.
- **Verify flag names against clap, not memory**: `migrate`'s filter flag is `--from` (not `--only`); `doctor` is a `visible_alias` of `migrate` and prints `format_scan_report` at the end. Provider in doctor output is `{:?}` — "Glm", not "GLM".
- `pick_best` priority (opencode > claude-code > aider > pi > cursor, else any-with-profile) is pure — unit-test it directly; `migrate/mod.rs` 22.6%→86.4%, scan/mod.rs →100%.

## Coverage pass 7 findings (2026-11-16)

- **`$EDITOR`-bound code: extract the spawn from the terminal suspend**: `run_editor_on_tempfile(editor)` takes the command *string*, not `std::env::var("EDITOR")` — unit-testable with a real executable fixture script (temp `.sh` + `chmod 0o755`; lightrs appends the temp path as last arg so `$1`/`$2` indexing exercises arg forwarding). Terminal suspend/resume stays in the pty-bound caller. `tui/editor.rs` 0→73%.
- **`split_whitespace` can't parse quoted `EDITOR` args** (`EDITOR='sh -c "echo x"'` splits inside the quotes) — acceptable for `code --wait`/`vim`; use a path-only script in tests.
- Fake-tool detection fixtures: `~/.pi/agent/settings.json` (any file = installed), `~/.cursor/mcp.json` (parsed as mcp_json), `~/.aider.conf.yml` (`model:` line = profile) — one `migrate --dry-run` asserts all names in the report. `detect.rs` →86%.

## ACP coverage note (2026-11-16)

- `cargo llvm-cov --summary-only` uses *default* features — `acp/*` is invisible until `--features acp` (and `acp-v2` for v2). Measure feature-gated code separately or `--all-features` totals get diluted by untestable `Stdio` wiring.
- ACP testable seam = the `Mutex<HashMap<SessionId, _>>` state machine (`AcpV1State`, `V2State`) — `#[tokio::test]` + fixture `AppConfig`; `make_agent` needs no network. The `Agent.v1().on_receive_request…connect_to(Stdio)` table is protocol-crate wiring — skip it, same class as the pty `run` loop.

## No hosted CI (owner decision, 2026-11-16)

- lightrs deliberately runs **no GitHub Actions / hosted CI** — do not scaffold `.github/workflows`, CI badges, or status checks. Verification is local only: `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo build --release`. If binary-size regressions matter, check `stat -f%z target/release/lightrs` (macOS) locally.

## Leanness audit results (2026-11-16)

- **Dep tree clean**: `cargo tree --duplicates` shows only `syn` v2+v3 (proc-macro build deps from different derive crates — unavoidable, build-time only).
- **Zero dead code**: all 308 `pub`/`pub(crate) fn` have ≥2 references (grep single-mention scan). Clippy `-D warnings` already covers unused imports/vars.
- **`cargo audit` clean**: 0 vulnerabilities / 309 crates; one *unmaintained* warning — `bincode 1.3.3` via `syntect` (`dump-load` for `default-syntaxes`/`default-themes`). syntect 5.3 is latest and wires it unconditionally; the dumps are syntect's own baked assets, not untrusted input. No action possible without dropping syntax highlighting — **accepted risk, documented**.
- `cargo audit` is installed (`cargo-audit-audit 0.22.1`); rerun after any dep bump.

## run/ refactor (2026-11-16, commit d951927)

- The "pty-bound" gap resolved by extraction, not a pty harness: `tui/run/mod.rs` shrank to a thin shell (terminal setup + `pump_queued` + 3-arm `InputAction` dispatch); the event machinery moved to `events.rs` (apply/progress table, 80% cov) and `observe.rs` (stream→event mapping, 91%). `run/mod.rs` 26.6% is now genuinely irreducible — remaining lines need a real terminal.

## Attachment-strip precision (2026-11-16)

- `slim_stored_user_content` previously truncated at a bare `\n[attached]`/`\n[skills]` — a user typing that literal text lost the tail of their message on resend/session-save. Real blocks always emit `\n\n[marker]\n--- @file` / `\n--- $skill:` (mentions.rs / skills/mod.rs), so the marker now requires the `\n---` header. Rule for text-stripping markers: match the full block signature, not just the label — labels are user-typable, headers aren't.

## UTF-8 boundary bug class (2026-11-16)

- **`String::truncate(n)` panics mid-char** — `tools/grep.rs` capped rg output at 64KiB with a raw `s.truncate(65536)`; a multi-byte char straddling the boundary → panic in the tool path. Shared fix: `tools::truncate_bytes(&s, n) -> &str` (walks back to `is_char_boundary`), now deduped with skills' private copy. When auditing truncation: byte-index slices/truncates are suspect anywhere *external* text flows (tool output, file bodies); `chars().take()` or `from_utf8_lossy` paths are safe by construction.
- rg-path fixture trick: compute the exact prefix rg prints (`<abs>:1:`), then fill so a 3-byte char straddles the cap — deterministic boundary test through the real `grep()`.

- **Same bug, two more sites** (skills): `&rules[..RULES_MAX_CHARS]` in `system_appendix` and `&chunk[..room]` in `load_rules` — a `rules/*.md` file pushing the catalog over 4KiB mid-char would panic *on every send* (appendix rebuilds per request). Both now use `tools::truncate_bytes`. Sweep heuristic that found all three: `grep -rn "\[\.\.[A-Z_]*\]\|\.truncate(" src/ | grep -v lossy\|chars()` — every raw byte-index on a `&str`/`String` carrying external text is a candidate panic.
