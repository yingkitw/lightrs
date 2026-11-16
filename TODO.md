# TODO

## Done

- [x] Per-model response dialects: parsers now accept the union of OpenAI-compatible shapes unconditionally — `content` string or `[{type,text}]` parts, `reasoning_content`/`reasoning` delta fields, index-less `tool_calls` deltas (was hard-required, killed streams on omit). `provider/dialect.rs` `ModelDialect::for_model` holds model policy — `inline_think` splits `<think>…</think>` for deepseek/r1/minimax-m1 names. Reasoning → `StreamApply::ReasoningDelta` → muted `[think]` transcript row (live, or post-hoc for non-stream/inline); `ApiMessage.reasoning_content` is receive-only (`skip_serializing`) — never echoed to providers or persisted in api_history
- [x] Terminal-escape injection + secrets-file perms: 3 render paths passed untrusted strings raw — `render_message_body` (model text: OSC 52/title injection), `render_unified_diff` (file bytes), `truncate_inline`/`fd.path` headers (model args; escapes mid-token survive `split_whitespace`) — all now strip via `strip_ansi` (CSI+OSC+two-char). Plus `write_private_file` (0o600 create + normalize existing) for `config.toml`/`mcp.json`/`opencode-import.toml` — keys no longer umask-0o644 world-readable
- [x] Index-stability sweep: `/rewind` truncated `ui_history` but left `expanded_tools` holding dead indices — now retains `i < ui_len` on rewind (prefix indices stay valid since rewind only drops the tail); pending-on-restore (`mod.rs`) and `/clear`→`clear_tool_expansion` already handled. `rewind_prunes_expanded_tool_indices` regression
- [x] Transcript render cache: `transcript_fingerprint` (DefaultHasher over width + expanded set + every rendered field per message, full content bytes) gates the per-frame `transcript_lines` rebuild — a cache hit skips all markdown/syntect work, only the viewport slice runs each frame; prior behavior rebuilt the entire history every 16ms busy tick
- [x] Secrets-to-disk sweep: `audit.log` redacted the command but wrote `result` raw — `env`/`cat .env` output landed on disk unredacted; `result` now passes `redact_secrets`. Plus `ensure_lightrs_dir` drops `.lightrs/.gitignore` (`audit.log*`, `backups/`, `todos.json`) once — machine state can't be `git add -A`'d; `config.toml` stays committable, user's own `.gitignore` never clobbered
- [x] Bounded reads on user-supplied files: `tools::read_to_string_capped` (open + `take(cap)` — bound at the source, `from_utf8_lossy` on the cut) applied to workdir instruction files (`AGENTS.md`/`CLAUDE.md`/rules), `SKILL.md`, and `rules/*.md` — a 1 GB instruction file no longer loads fully before the 6/4 KiB content caps (256 KiB read bound); own-dir reads (`~/.config` session/config) exempt
- [x] Interrupt reaches running shell commands: `watch::Receiver` plumbed turn → `execute_tool` → `run_command` poll loop → `kill_process_tree` — Esc/`stop` now kills a `sleep 600` in ~50ms instead of only acting between steps (heal command covered too); `cancel_stops_running_command` regression
- [x] Full-pipe deadlock fix: `run_command` read stdout/stderr only **after** `try_wait` — any command emitting >64 KiB OS pipe buffer blocked on write and ate the whole timeout (seq 100000: was 120s kill, now ~0.2s). `drain_stream` reader threads consume continuously, retaining ≤64 KiB; same pattern applied to `grep`'s `rg` spawn (was `output()` = no timeout + unbounded buffering → 60s deadline + bounded drain + group kill)
- [x] Cleanup/bounds sweep: `/editor` tempfile leaked when the `$EDITOR` binary failed to spawn (`?` before `remove_file` — restructured so cleanup always runs); SSE `line_buf` unbounded on newlineless streams — extracted `take_completed_lines` (1 MiB line cap + `drain()` instead of per-line remainder copy)
- [x] `run_command` timeout lifecycle: `kill()` alone left un-reaped zombies **and** only signaled the `sh` shell — subprocesses (sleep/build/server) survived orphaned. Now `process_group(0)` spawn + `kill -KILL -- -pgid` + `wait()` — whole tree dies, no zombie; regression test proves the `sleep` grandchild never writes its marker
- [x] Bounded `.lightrs/` growth: `audit.log` rotates to `audit.log.1` past 256 KiB (was append-forever); `.lightrs/backups/` prunes to newest 256 `.bak` snapshots per workdir (was unbounded — `/undo` only reads newest-per-path, tail is preserved)
- [x] Tool `result:` syntax highlight: `read_file` results render through syntect (ANSI-stripped first — file bytes are untrusted), lang derived from `arguments.path` extension; `error:` bodies and shell/grep/ls output stay literal pre (never markdown-parsed)
- [x] Whitespace-preserving render: new `wrap_code_lines` (verbatim chars, char-boundary hard-wrap) replaces the `split_whitespace` word-wrap in code fences, tool/pre output, unified diffs, and mermaid diagram/source rows — fixes lost tab/leading indentation; `wrap_plain_lines` kept for prose only
- [x] `[endpoints]` per-provider base-url overrides: `AppConfig::endpoint_for` (active `base_url` → override → default) used by `/model` switch + list fetch; OpenCode import fills it from `options.baseURL` + coding-plan auth ids (`kimi-code-plan-global`→api.kimi.ai, `minimax-cn-*`→api.minimaxi.com); project overlays ignore it (redirect-attack surface)

- [x] `/model` live picker: bare `/model` fetches `GET /models` per keyed provider on the background runtime (`App.rt`/`ev_tx` wiring, `AgentEvent::ModelsListed`), renders numbered rows (`← current`, error rows for missing keys/401s); `/model <n>` applies the pick through the existing `model_switch` path

- [x] OpenCode key carry is complete: all providers' `options.apiKey` (with `{env:VAR}` resolution), `options.baseURL`, and `opencode auth login`'s `~/.local/share/opencode/auth.json` `type:"api"` entries merged into `[keys]` (config apiKey wins; oauth/unmapped skipped) — covers both `migrate` scan and `import opencode`

- [x] Workdir context reads rule conventions: `.cursorrules`, `.windsurfrules`, `.github/copilot-instructions.md`, and `*.md`/`*.mdc` under `.cursor/rules/` + `.windsurf/rules/` (`.mdc` frontmatter stripped, sorted by name, shared ~12 KiB total cap bounds all sources)

- [x] Migrate detects Codex (`~/.codex/config.toml` → `model_provider`/`model`/`[model_providers.*] base_url`+`env_key`); `provider_from_base_url` host mapper (id substrings ≠ URL hosts: `api.z.ai` ≠ `zai`); priority slot between claude-code and aider; e2e via `env_key = "HOME"`

- [x] Workdir context reads `CLAUDE.md` + `GEMINI.md` (competitor-convention intel: codex→AGENTS, aider→AIDER already covered); SPEC cap corrected 4→6 KiB drift

- [x] Panic fix ×3: `grep` rg-path 64KiB output cap + `skills` rules caps (`&rules[..RULES_MAX_CHARS]`, `&chunk[..room]`) all used raw byte slices — mid-UTF-8 panic; shared `tools::truncate_bytes` boundary walk, boundary-straddling fixtures per site used `String::truncate` — panics on mid-UTF-8 cuts; shared `tools::truncate_bytes` boundary walk (deduped skills copy), regression test with a computed '€'-straddling fixture

- [x] Context precision fix: `slim_stored_user_content` now requires the `\n---` block header — a user-typed literal `[attached]`/`[skills]` no longer truncates their message on resend/session-save; regression test + `docs/context.md` note

- [x] Leanness audit: dep tree clean (only `syn` v2/v3 build dup), 0 dead `pub fn` of 308, `cargo audit` 0 vulns (bincode-via-syntect unmaintained warning accepted — no fix without losing syntax highlighting); CI skipped by owner decision

- [x] ACP state machines: `AcpV1State` (insert/alloc/remove, distinct ids, remove-miss noop) + `V2State::create` tokio tests — `acp/v1.rs` 0→43% lines; remainder is `Stdio` wiring only

- [x] `run_editor_on_tempfile` seam: `/editor` spawn + read-back unit-testable via fake executable scripts (0→73%); `detect.rs` pi/cursor/aider covered via scan-all fixture test

- [x] Migrate orchestrator coverage: `pick_best` unit tests + spawned-binary e2e (`migrate` real run from fake `~/.claude/settings.json`, `--from` miss error, `doctor` auto-bootstrap → config.toml → GLM doctor pass); `migrate/mod.rs` 22.6→86.4%

- [x] `/model` mid-session switch (`/model`, `/model <name>`, `/model <provider> [name]`); `Agent::set_client` swap + `resolve_runtime_api_key`; coverage passes: `tui/slash.rs` 61→86%, `tui/run/input.rs` →94% (synthetic key events + project-command kick), total 84.99%

- [x] Mermaid renderer: layered box layout for `flowchart TD`/`LR` (Kahn ranks, barycenter ordering, `Canvas` junction merging, edge labels on wide spans); flat edge list for cycles / `BT`/`RL` / over-width; sequence rows unchanged; panel bg (`code_bg` + `inline_code_bg` node fill) via `Canvas.kind` → `segment_row`

- [x] OpenCode-style UX signals: `path:branch` in header (`.git/HEAD`, worktree-safe, refreshed on `/cd`), context `%` of model window (`model_context_window` table; glm-4.6/M2.1 204.8k, kimi-k2 256k), `↑N scrolled · End latest` hint

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
- [x] Context module extraction — `agent/context/` domain module (`wire`/`compact`/`tokens` + policy `mod.rs`); `/context` gains a `limits:` line rendered from the same consts; `tests/support` mock server captures request bodies; `tests/context_budget.rs` e2e asserts the wire cap (12 turns) + stub markers on real requests; full policy in `docs/context.md`
- [x] Wire context window — `messages_for_api` trims the request payload to last **12** user turns / ≤**48k est** (non-destructive; `/rewind`+session keep full history); stored-history auto-compact unchanged (stub 32k / drop 64k raw); one-shot `[sys]` note + `/context` `wire: X of Y turns`
- [x] Tool output renders clean — ANSI escapes stripped at `run_command` capture (`tools/shell::strip_ansi`: CSI/OSC/two-char) and defensively at render; tool bodies + args render preformatted via `highlight::render_pre_body` (no markdown mangling of `#`/`|`/`---`)
- [x] Live tool rows + elapsed — `UiMessage.pending` row (`◌ name … running`) pushed at ToolStart and replaced in place by the result (`agent.pending_tool`); `elapsed_ms` renders `· 1.2s`; turn-close line gains wall-clock (`turn · N calls · X in + Y out · Ns`); stale `◌` cleared on turn end + session restore (`chat::fmt_elapsed`)
- [x] `ModelClient` trait seam — `provider::ModelClient` (object-safe, hand-rolled `BoxFuture`, no `async-trait` dep); `Agent` holds `Arc<dyn ModelClient>` via generic `new`/`set_fallback_client`; stream→non-stream fallback moved inside `LlmClient::send`; `MockModel` serves canned replies + records requests → in-memory turn tests (fallback retry, interrupt propagation, weak-model pick); interrupt sentinel moved to `provider::INTERRUPTED`
- [x] Weak-model routing — `weak_model`/`--weak-model` serves read-only steps on a cheaper model; first write/exec tool call (`write_file`/`patch_file`/`multi_edit`/`run_command`) escalates to `model` for the rest of the turn (`agent/turn.rs::escalate_from_weak`); e2e `tests/weak_model.rs` asserts the `model` field on captured requests
- [x] Ponytail audit — single turn loop: deleted `&mut self` path (`drive_turn_loop`, `handle_model_message`, sync `execute_one_tool`); exec/ACP use `run_user_turn_headless` over the shared loop; ACP sessions hold `Arc<Mutex<Agent>>` (sessions lock no longer spans turns); fixed dead `not(acp-v2)` match arm that broke `--features acp` build
- [x] Coverage pass 2 (73.9%→76.1%): skills 67→95% (load/hints/attach/slug/rules/parse edges), shell_readonly 70→98% (env-prefix, chains, git/cargo/find tables), grep 63→80% (caps/scope/ci/no-match). Fixed 3 latent bugs: `extra_paths` scanned after ambient roots + `MAX_SKILLS` cap eviction (now first = precedence); `FOO=bar git status` misrejected (classify on env-stripped remainder); grep `path=` silently scanned 0 files under non-canonical workdir (canonicalize at entry)
- [x] Coverage pass (llvm-cov 68.6%→73.9%): backup.rs 33→92% (snapshot/restore/escape-guard tests), slash `run_slash` 27→55% (quit/unknown/diff/rewind/undo/cd handlers), TestBackend draw tests → transcript 19→79%, chrome 0→58%, ui/mod 100%; **found+fixed**: 2-line header's `path:`/ctx row clipped by the bottom border (`Length(header_rows)` → `header_rows + 1`) — `path:branch` was invisible at h≥22
- [x] `/diff` — session change review (aider-style): `session::collect_file_diffs` replays stored `FileDiffUi`s as collapsed `diff` tool rows (expand with `o`/click); `[sys]` summary totals unique files + `+N −M` across writes — zero filesystem reads
- [x] Terminal title — OSC 2 `lightrs · dir[:branch]` + `· ●` while Busy, set on change each loop pass, cleared on exit (`tui::title_escape`, control chars stripped)
- [x] Native copy without losing scroll — mouse reporting narrowed to `?1000h`+`?1006h` (`tui::MOUSE_ON/OFF`); `?1002` drag tracking off → terminal drag-select + Cmd/Ctrl+C work while wheel scroll and tool-row clicks are unaffected
- [x] `/model` — mid-session switch: `/model <name>` (same provider, keeps base URL + key), `/model <provider> [name]` (provider default endpoint + model; key re-resolved via `AppConfig::resolve_runtime_api_key`, fails fast); `Agent::set_client` swaps `Arc<dyn ModelClient>` — header + context % recompute from `client.model()`
- [x] Coverage pass 3 (76.8%→79.4%): `session/mod` 41→90% (`load_file`/`save_to`/`remove_file` path-seam refactor + all reject branches), `opencode/assets` 0→90% (mcp/skills/rules/hooks import + tilde), `tui/info` 66→98% (header/session builders, host label, row-count threshold); `tests/cli.rs` + import e2e → `opencode/mod` orchestrator covered
- [x] Coverage pass 4 (81.5%→84.4%): `run/input` 63→94% (synthetic crossterm keymap: approval y/n/Esc, ctrl-q/q quit-vs-interrupt, history recall, scroll keys, paste/mouse), `agent/context/mod` 50→97% (report/quiet/pressure/compact policy), `config/file` 69→83% (resolve_api_key layers, mtime, skills/tui parse), `tui/mentions` 57→91% (skip-dirs, common prefix, hints), `migrate/` write 98% + profile 90% — **found+fixed**: `anthropic_base_to_profile` doubled `/api/api/paas/v4` on `/api/anthropic` bases. Extracted `Agent::from_config` (shared TUI/ACP client bootstrap dedup)

## Backlog

- [x] MCP import documented (stored `mcp.json`; no in-process server in v0.1)

## Scorecard (2026-11-15, rev 2)

| Dimension | Score | Note |
|-----------|-------|------|
| Correctness & coverage | 5 | 346 tests, 85.73% lines (llvm-cov); `run/` event logic extracted to `events.rs` (80%) + `observe.rs` (91%) — remainder is the pty shell; spawned-binary e2e + request-capture on the shared mock |
| Agent reliability | 5 | Step cap, interrupt, heal loop, fallback + weak-model routing, single turn loop |
| UX (TUI) | 5 | Queued busy input, `/editor`, `/cd`, `/todo`, native copy + scroll; OpenCode signals (ctx %, `path:branch`, jump-to-latest); mermaid ASCII layout panel |
| Maintainability | 5 | `agent/context/` domain module; duplicated turn loop deleted (net -108) |
| Docs alignment | 5 | `docs_sync` + `docs/context.md` + root MDs audited to code |
| Footprint | 5 | `lto`+`codegen-units=1`+`strip` in release → 7.4M default / 5.6M minimal / 9.0M full (−32%); `--no-default-features` drops ACP + syntect |
| Token efficiency | 5 | Wire window (12 turns/48k) + stored compact (32k/64k) + weak-model routing; `chars/4` estimate (documented) |

**Weakest next:** MCP wiring (out of v0.1 scope) or optional live SSE capture into `tests/fixtures/`.

## Brainstorm (unprioritized)

- ~~Release binary size badge in CI~~ — **deliberately skipped**: project runs no GitHub Actions / hosted CI (owner decision 2026-11-16). Verification stays local: `cargo test`, `clippy`, `cargo build --release` per AGENTS.md Step 3.

## Competitive intel — craftine + vendored minicode (2026-11-15)

Sourced from `../craftine` (sibling Rust TUI agent, ~39k lines) and its `craftine/minicode/` vendored reference. Filtered for the lightweight charter — MCP, tree-sitter, vim mode, semantic index, sub-agents, worktrees, pricing tables, browser tools, modes, SDD, hooks system, OS sandbox, keyring deliberately skipped.

### Tier A — cheap, high value

Shipped 2026-11-15 (see Done): queued input while `Busy`, interrupt keywords, `/cd`, fallback model, `todo_write` + `/todo`, heal loop (`[shell].test_command` — config only, no auto-detect), `/editor`, skill `trigger` hints, `.claude/commands/*.md` project slash commands. **Tier A complete.**

### Tier B — moderate weight

- ~~**Approval batching**~~ — shipped: `resolve_shell_approvals` pre-scans the step, one prompt, all-or-nothing verdict.
- ~~**Audit log**~~ — shipped: `.lightrs/audit.log` JSONL per `run_command` decision.
- ~~**Weak model routing**~~ — shipped: `weak_model` config/flag; per-turn escalation on write/exec tools.
- ~~**Per-project config overlay**~~ — shipped: safe-subset `.lightrs/config.toml` (model/fallback/weak/`[shell]`).
- ~~**Prompt-injection warning**~~ — shipped: `tools/injection.rs` scans tool results; flagged prefix goes into API history + `[sys]` notice.
- ~~**Atomic session save**~~ — shipped: temp file + same-dir rename.

### Patterns worth borrowing (not features)

- ~~**Trait seam for tests**~~ — shipped as `provider::ModelClient` + `MockModel` (see Done); `ToolExecutor` seam skipped — tools run against temp dirs cheaply already.
- ~~**Event loop batching**~~ — verified already correct: `tui/run` drains `rx.try_recv()` batches then paints once per wakeup; `redraw_immediately` skips the input poll while events queue; 16ms busy cadence caps delta draws. Opencode's "don't navigate prompt history if dirty" also already covered — `Composer.draft` preserves in-progress input across Ctrl+P/N recall and restores it at the newest end.