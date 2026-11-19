# Specification

## Goals

- Terminal UI coding assistant with low footprint and fast startup
- Pluggable backends: GLM, Kimi, MiniMax via OpenAI Chat Completions + tools
- Safe-by-scope: tools only affect configured working directory

## Modes

- **TUI** — default (`lightrs` / `lightrs tui`)
- **ACP v1** — `lightrs acp` (initialize, authenticate, session/new, session/prompt, session/cancel); requires `acp` feature (default). Capability exposure: `AgentMessageChunk` streams assistant text, `AgentThoughtChunk` carries reasoning (`ObserveEvent::ThinkingDelta` — never mixed into message text), `ToolCall`/`ToolCallUpdate` reports `InProgress`→`Completed`/`Failed`, `AvailableCommandsUpdate` advertises project + user `.claude/commands` (`/name args` prompts expand through the same command table as the TUI), `session/cancel` trips a `watch` token armed per turn — the in-flight request/tool dies, the prompt responds `StopReason::Cancelled`, and the session survives for the next prompt. **Modes**: `session/new` advertises `build` + `plan` (`read_only` gate, same as TUI `/plan`); `session/set_mode` flips it and notifies `current_mode_update` (the response body stays empty per spec); unadvertised ids error. Turn failures answer the request with a JSON-RPC error (v1 has no `error` stop reason)
- **ACP v2** — `lightrs acp --protocol v2` — same exposure over the v2 update vocabulary (`AgentMessage`/`AgentThought` chunks keyed by message id, `ToolCallUpdate` status, `AvailableCommandsUpdate`, `CancelSessionNotification`); the turn tail reports `Idle` with `EndTurn` / `Cancelled` / `Error(ErrorStopReason)` — errors also emit a message chunk with the failure text. Reply deltas are the single delivery channel: the non-stream fallback emits one post-hoc `ContentDelta` in `send()`'s `finish`, so observers never need a "final reply" side-channel (prevents the double-send duplication)
- **Import** — `lightrs import opencode` merges OpenCode JSON into `~/.config/lightrs/`; provider `options.apiKey` values (all mapped providers, `{env:VAR}` resolved) plus `~/.local/share/opencode/auth.json` `type:"api"` keys are carried into `[keys]` — explicit config keys win over auth.json. A **project** `opencode.json` merges too (skills paths, instructions, mcp, endpoints land in *global* state) — run import only in repos you trust. Emitted `config.toml` values are debug-quoted so foreign strings can't inject TOML sections
- **Migrate** — `lightrs migrate` / `lightrs tui --auto-migrate` scans OpenCode, Claude Code, Codex, Aider

## Non-goals (v0.1)

- Full regex/ripgrep parity, multi-agent orchestration beyond depth-1 `spawn_agent`
- Side-by-side or inline patched diff editor (unified diff in transcript only)
- Windows sandbox backend, container/VM isolation, MCP transports beyond stdio, LSP features beyond navigation (no workspace edits, diagnostics pull, or rename)

## User interface

### Transcript

- Roles: user, assistant, tool, system, error
- **`[sys]`**: ephemeral UI-only (`UiRole::System`); styled with `▏ sys` bars (`tui/highlight/sys.rs`); never sent to the API; **excluded** from `session.rs` save/restore
- **Markdown**: full pulldown pass on message bodies — headings, lists, **bold** / *italic* / ~~strike~~, links (styled), blockquotes (`▏`), pipe tables (`│`), rules; strikethrough/task lists enabled
- **Fenced code**: syntect highlight + wrap
- **Mermaid** (`mermaid` / `mmd`): laid-out ASCII diagram + source — `flowchart TD`/`graph` renders boxed nodes in topological layers with `▼`/`─`/`┬` connectors (labels on wide spans); `flowchart LR` renders column-major with `├─►`; `sequenceDiagram` renders per-message rows. Cycles, `BT`/`RL`, over-width or >48-node graphs fall back to a flat edge list. Laid-out diagrams render on a solid panel — `code_bg` layout area, `inline_code_bg` box fill, `muted` borders, `text` labels, `mermaid_edge` connectors (rows padded to a common block width)
- **Tool blocks**: `◌ name args … running` while the call executes, replaced in place by the result with an elapsed suffix (`· 1.2s`); header row keeps a one-line call summary (args or file +/−) in both states; collapsed shows truncated result, expanded shows full args/result — tool bodies render **preformatted** (literal lines, no markdown), and ANSI escape sequences are stripped at capture and render. Results beginning `error:` render in the error color; other results use the normal body color. Terminal escapes are stripped from **all** untrusted render inputs — model message text, tool args/path header lines, and diff bodies — not just tool output. Single-line chrome surfaces (shell-approval command list, status bar incl. `model asks:` text, busy label, Δ changed-files, slash/`@`/`$` hints) render through `escape_controls` — control bytes show as visible `\xNN` escapes so nothing reaches the terminal or forges rows
- **Tokens**: muted `[sys]` lines per turn — `send · ~N ctx` after your message (estimate), `model · N in + M out` after each API step when the provider reports usage, and a `turn · N calls · X in + Y out · Ns` close line (real usage + wall-clock)
- **`write_file`** / **`patch_file`**: colorized unified diff (`+`/`-`/`@@`); `+N −M` in collapsed header; full diff when expanded; API tool body remains a one-line summary
- **Mouse**: wheel scroll; left-click toggles tool expand (with `o` when input empty); drag = native terminal selection (copy) — the app only requests `?1000` click/scroll reporting, never `?1002` drag tracking
- **Busy**: spinner + optional “waiting for model…” when streaming has not started
- **Terminal title**: OSC 2 set once per loop pass when it changes — `lightrs · dir[:branch]` with a `· ●` suffix while `Busy`; cleared on exit so the shell title takes over

### Chrome

- Header: version, provider, model, state, `workdir:branch` (git branch from `.git/HEAD`, worktree-safe), context estimate + `%` of the model window when known (`config::model_context_window`), API host
- Multi-line composer (not single-line only); placeholder hints when empty
- Status bar: always leads with session indicators — `~N ctx (P%)` estimate, `Δ file1, file2 +n` for files written by write/edit tools this session (rebuilt from persisted diff rows on resume; `/diff` shows the patches), `$skill` ids attached to the latest turn, `· plan` in read-only mode — then slash hints, scroll/tool hints, or busy/error text; when scrolled up it shows `↑N scrolled · End latest`. A `[sys]` `skills attached: …` note also lands in the transcript when a `$id` resolves. A Ready-state `app.status` (failed turn, slash error) renders its text in the error color instead of the hint chain — previously style-only and invisible
- **Theme**: selectable palettes via `/theme` (`tui/theme/`; default `mocha`)

### Input

- `Enter` send; `Shift+Enter` / `Ctrl+J` newline
- `@relative/path` file mentions in the composer; `Tab` completes paths under **workdir** (status bar shows matches). On send, matching files attach to the **API** user message only (12 KiB per file, 36 KiB total); the transcript shows your composer text unchanged
- `$skill-id` invokes discovered skills (`Tab` on `$` prefix); skill bodies attach to the API message (see **Skills** below)
- `Ctrl+P` / `Ctrl+N` prompt history
- Scroll transcript when input empty (`↑`/`↓`, PgUp/PgDn, Home/End)
- Interrupt while busy: `Esc`, `Ctrl+C`, `Ctrl+Q`, or send a bare `stop` / `wait` / `halt` (does not quit). Interrupt reaches a running `run_command` within ~50ms and kills its process group — no waiting out `timeout_secs`. A turn cut mid-tool-batch leaves assistant `tool_calls` without results — `close_orphaned_tool_calls` inserts `[interrupted — no result recorded]` stubs before the next request (a dangling tool_call is a provider 400)
- **Queued input**: typing while busy edits the composer; `Enter` queues the line (`queued N` in the status bar) and it sends automatically when the turn ends. Interrupting the run instead **restores queued lines to the composer** — a stopped turn never auto-fires what was typed behind it. Slash commands queued the same way run when idle
- **Steering**: queued non-slash lines are also drained at each **tool-step boundary** mid-run (pi-style steering) — the turn loop injects them as user messages between tool steps, so a long tool loop can be course-corrected without aborting. Slash/`!` lines stay queued for local dispatch

### Slash commands (not sent to model)

| Command | Action |
|---------|--------|
| `/help`, `/?` | List commands |
| `/clear` | Clear API + UI history, delete session file |
| `/cwd` | Print working directory |
| `/cd` [path] | Change working directory (`~`, `~/x`, absolute, or relative; keeps the conversation; session file follows the new workdir) |
| `/todo` | Show the task list written by `todo_write` (`.lightrs/todos.json`) |
| `/editor` | Compose the prompt in `$EDITOR` (e.g. `vim`, `code --wait`); buffer loads into the composer on save |
| `/status` | Session / model details + token usage + files changed this session |
| `/model` | Bare `/model` fetches `GET /models` for every provider with a resolvable key in the background and renders a numbered list (`←` marks current; unkeyed/failed providers show an error row). Pick by typing a bare number as the next input (one-shot pending state), or `/model <n>`; `/model <name>` switches model on the same provider (keeps base URL + key); `/model <provider> [name]` switches provider (default endpoint, provider default model when omitted; key re-resolved from config/env — fails fast when missing). Every successful switch persists `provider`/`model`/`base_url` to `config.toml` (the API key is never persisted — re-resolved from env/`[keys]` next launch) so the selection survives restarts. Header + context % update |
| `/context` | Context budgets (`limits:`), wire vs stored turn counts, per-role breakdown, heaviest messages, current-turn in/out — full policy in `docs/context.md` |
| `/reload` | Re-read `config.toml` `[shell]`; rescan skills; refresh agent system prompt |
| `/trust` | Persist the current workdir in global `[trust].dirs` — its `.lightrs/config.toml` applies on next launch and to `/reload` now |
| `/quiet` | Toggle `[sys]` token lines (`send · ~N ctx`, per-step in/out, turn total); header + `/context` unchanged |
| `/skills` | List discovered skills (`$id` to invoke on send) |
| `/export` [path] | Write UI transcript to markdown under workdir (default `lightrs-transcript.md`; skips `[sys]`) |
| `/copy` [`code`] | Copy latest assistant reply to the OS clipboard; `code` = last fenced ``` block only |
| `/theme` [`id`] | List palettes or set `mocha` / `hc`; saved under `[tui]` in `config.toml` |
| `/compact` | Replace older API tool bodies + assistant `tool_calls` argument blobs with placeholders (keeps last 6 tool results); no LLM call |
| `/rewind` [`n`] | Move the last `n` user turns (default 1) out of API + UI history at user-turn boundaries — the detached tail is kept as a restorable **branch**, then the session resaves |
| `/branch` [`n`] | List rewind branches (turn count + prompt preview) or swap branch `n` back in; the replaced tail becomes a branch itself, so switching is reversible |
| `/diff` | Replay this session's stored write diffs as expandable tool rows (`+N −M` totals per file; no filesystem reads) |
| `/undo` | Restore files edited since your last message from `.lightrs/backups/` |
| `/plan` | Toggle plan/read-only mode — write/exec tools denied; system prompt + status bar reflect it |
| `/fork` [`name`] | Checkpoint the session to `<hash>-<name>.json` (untouched by later auto-saves) |
| `/resume` [`n`\|`name`] | List all session files newest-first, or restore one by number/name — cross-workdir resumes also `/cd` |
| `/doctor` | Run local environment checks (config, tools, sandbox backend) into the transcript |
| `/init` | Send a canned prompt — model explores the workdir and writes `AGENTS.md` |
| `/quit` | Exit (same as `q` when idle) |

`Tab` completes unique slash prefix. **`!cmd`** runs a shell command locally on a blocking thread — no model turn, no Busy state; output lands as a `[sys]` row (`$ cmd` echo + `exit=`/stdout/stderr). The user typed the command, so the shell **allowlist is bypassed**; the **deny list** (`rm -rf ~`, `curl | sh`, …) and `[sandbox]` wrap still apply.

**Project commands** (`src/commands.rs`): `.claude/commands/*.md` under the workdir **and `~/.claude/commands/*.md` (user-global, Claude Code layout)** are discovered as `/name` (filename stem); `description:` frontmatter shows in hints, and the body is sent to the model with any args appended. Built-ins win on name collisions, and project commands shadow user-global ones. Rescanned on `/cd`, `/reload`, sends, and unknown-slash dispatch.

### Session resume

Per canonical **workdir**, `~/.config/lightrs/sessions/<hash>.json` stores API history, UI messages (including `tool_call`, `file_diff`), and token stats. **`[sys]` lines are ephemeral**: shown in the live transcript but omitted from session save/restore. `/clear` wipes disk + memory. Saves are atomic (temp file + same-dir rename — a crash leaves the old or new file, never partial JSON). The transcript keeps at most **500 messages** (`MAX_UI_MESSAGES`; oldest drop first) so session files and per-frame rendering stay bounded.

## Agent behavior

- System prompt: concise coding helper; must use tools for filesystem/shell facts
- Loop: user message → model → (optional) tool calls → tool results → model until text reply or step cap (**16**)
- **Lean API context**: persisted user rows are composer text only; `@` file bodies and `$skill` bodies attach for the **active turn** only. Prior turns omit attachments when resent. Tool results in API history are capped (**6 KiB**); full output may still appear in the TUI.
- **Streaming**: assistant text appended live when supported; tool calls assembled from stream deltas before execution
- **Response dialects** (`provider/dialect.rs`): response parsing is lenient — `content` accepts a string or parts array (`[{"type":"text","text":…}]`), `reasoning_content`/`reasoning` fields, index-less `tool_calls` deltas, tool calls missing `id`/`type` (backfilled `call_{i}`/`function` via `normalize_tool_calls`), `function.arguments` as a JSON object (serialized), and `input_tokens`/`output_tokens` usage aliases are all accepted. Error responses read a **4 KiB-bounded** body (`error_body_capped` — HTML dumps would otherwise land in the transcript/history) and extract a message from `{"error":{"message"/"msg"/"detail"}}`, `{"error":"…"}`, or `{"message":"…"}` (`api_error_message`), falling back to the capped raw body. `ModelDialect::for_model` adds model-specific policy — today only `inline_think` (DeepSeek-R1/MiniMax-M1-style models emit `<think>…</think>` inside `content`; it's split into reasoning — live via a `ThinkSplitter` state machine that holds back tag fragments across deltas). Reasoning text renders as a muted `[think]` transcript row (streamed live via `ReasoningDelta`, or appended post-hoc for non-stream/field replies); it is **never** echoed back on the wire or persisted in API history (`ApiMessage.reasoning_content` is receive-only). Unknown models get the plain OpenAI dialect. Streamed requests send `stream_options: {"include_usage": true}` — providers omit the terminal `usage` chunk unless asked, so token stats would silently zero on every streamed turn (the field is cleared on the non-stream fallback). When SSE fails, the non-stream `chat()` retry is **cancelable** — Esc aborts the in-flight request (the select drops the future) instead of waiting out the HTTP timeout
- Tools: `read_file`, `write_file`, `patch_file`, `multi_edit`, `list_dir`, `grep`, `glob_file_search`, `run_command`, `todo_write`, `web_fetch`, `ask_user`, `spawn_agent`, `mcp_call`, `lsp` (schemas in `tools/schemas.rs`)
- **`read_file`**: `max_bytes` (default 64 KiB, hard max 512 KiB) bounds the **read at the source** (`File::take` + `metadata` size for the truncation note) — an oversized file can never load whole.
- **`patch_file`**: exact match first; single-occurrence edits fall back to whitespace/indent-tolerant match (`tools/edit_match.rs`). **`replace_all`** requires exact `old_string` matches. Prior content is read-bounded at **1 MiB** — larger files refuse (`patch` must match against real content); `write_file` over one proceeds without backup or diff and says so in the summary.
- **`grep`** (non-`rg` fallback): reads at most `MAX_READ_BYTES`+1 per file (256 KiB) — binaries are skipped by NUL check on that bounded slice, not by loading the file.
- **`run_command`**: stdout and stderr drain on reader threads (excess discarded past **64 KiB** each, truncation note in the result) so high-output commands return promptly; ANSI escape sequences stripped. `timeout_secs` (default 120s, max 600s) kills the command's **whole process group** — subprocesses (sleeps, builds, servers) die with the shell — and reaps the child (no zombies). **Session cwd**: the final `$PWD` of each call is captured (via a temp file in `.lightrs/`, not an stdout marker — the stream cap keeps first bytes) and becomes the next call's start, so `cd` persists across calls and for `!cmd`. Fmt/heal hooks always start at workdir (a hook `cd` must not move the session); `/cd` and `spawn_agent` children get a fresh cwd.
- **`todo_write`**: replaces the whole task list (`items[].content`, `items[].status` ∈ `pending|in_progress|completed`, max 32) persisted to `.lightrs/todos.json` (`tools/todo.rs`); view with `/todo`
- **`web_fetch`** (`tools/web.rs`): GET an `http(s)` URL; HTML stripped to text (script/style dropped, entities decoded, blanks collapsed); body bounded at **64 KiB** read live off the stream; http/https schemes only; cancel-aware. Dispatched async from the turn loop (no `reqwest::blocking` dep). **SSRF guard**: instance-metadata endpoints are denied (`169.254.169.254`, `fd00:ec2::254`, `metadata.google.internal`, `100.100.100.200`) on the initial URL **and** every redirect hop — a model GET bypasses the shell allowlist, worst in plan mode. Residual: DNS-rebind (hostname resolving to the link-local IP) is a documented limit
- **`ask_user`** (`agent/approval.rs`): model-initiated clarifying question — routes through `ui_tx` oneshot like shell approval; next Enter submits the answer, Esc declines with `(user declined to answer)`; headless/ACP fail closed (`needs an interactive TUI`)
- **`spawn_agent`** (`agent/approval.rs`): delegates a self-contained prompt to a child `Agent` sharing the parent's client — own context, same tools, **depth-capped** (children cannot spawn), headless policy (shell approvals fail closed), cancel propagates through the shared watch token. `read_only: true` forces the child into plan mode — inherited automatically when the parent is in plan mode
- **`mcp_call`** (`mcp/`): calls a tool on a `[mcp.<name>]` stdio server — newline-delimited JSON-RPC (`initialize` → `tools/list` cached → `tools/call`), spawned lazily on first use, 30s per-RPC deadline, dead servers respawn on next call. Omit `tool` to list the server's tools. Configured server names are advertised in the system prompt. Denied in plan mode (unknown side effects)
- **`lsp`** (`lsp/`): navigates via a `[lsp.<lang>]` stdio language server — `Content-Length` framed JSON-RPC, `initialize` per language, full-text `didOpen`/`didChange` sync (refused past 512 KiB — truncated docs would analyze wrong), 15s deadline, failed requests drop the server for respawn. `definition`/`references`/`hover`/`symbols`; language resolved from the file extension so the model never names a server
- **Plan mode** (`/plan`, `--read-only`): `write_file`/`patch_file`/`multi_edit`/`run_command`/`mcp_call` are denied at the tool gate before approval/execution (`READ_ONLY_DENY`); read/search/`todo_write`/`web_fetch`/`ask_user`/`lsp` still run, `spawn_agent` runs with an inherited read-only child; the system prompt carries the mode so the model plans instead of discovering per-call; status bar shows `· plan`
- **HTTP**: chat requests use 120s timeout, 30s connect timeout, up to **3** retries on 429/5xx and transient transport errors (`provider/http.rs`). Retries honor `Retry-After: <secs>` (seconds only, capped at **30s** — `max(hint, exponential backoff)`) and the retry sleep is **cancelable**; discarded retry bodies are read capped (4 KiB), never in full — and scanned for `insufficient_quota`, which short-circuits: billing failures are terminal, so neither the backoff retries nor the stream→non-stream fallback spend another request. Cancel covers **every await phase**: the `send()` response-head wait races `cancel_wakeup`, so a hung endpoint aborts on Esc/`session/cancel` instead of parking for the full 120s (previously only retry backoff, the SSE chunk loop, and the non-stream fallback were cancelable).
- **Fallback model**: `--fallback-model <m>` or `fallback_model` in `config.toml` retries a failed model fetch **once** on the secondary model (any non-interrupt error); a `[sys]` line notes the switch
- **Weak-model routing**: `--weak-model <m>` or `weak_model` runs each user turn on the cheaper model until a step calls a write/exec tool (`write_file`, `patch_file`, `multi_edit`, `run_command`), then escalates to `model` for the rest of that turn (one `[sys]` note per escalation). Next turn starts weak again.
- **Thinking knob**: `thinking = "high"` in `config.toml` (global or trusted project overlay) is sent verbatim as `reasoning_effort` on every chat request. No validation — providers without support may ignore or reject the field; opt-in only.
- **`lightrs add <src>`**: install resources from a git URL (shallow clone, `GIT_TERMINAL_PROMPT=0`) or local dir — dirs containing `SKILL.md` copy to `~/.config/lightrs/skills/<slug>/` (same-id replaced; `.git`/oversized files skipped), `*.md` under a `commands/` dir copy to `~/.claude/commands/`. Scan/copy are bounded (8k entries, 5 depth, 200 files, 256 KiB/file).
- **Context pressure**: when estimated API context exceeds **~20k tokens**, the TUI shows a `[sys]` hint (respects `/quiet`).
- **Wire window**: the model never receives the full stored history — each request is trimmed to the **last 12 user turns**, then oldest whole turns drop until the wire payload fits **~48k est tokens** (never the in-flight turn, never split tool_call pairs). `api_history` keeps everything for `/rewind` and session save; a one-time `[sys]` note announces the first trim, and `/context` shows `wire: X of Y turns`.
- **Auto-compact**: before each request, if the *stored* history estimate exceeds **~32k tokens**, old tool results and assistant `tool_calls` argument blobs (>512 chars) are stubbed in `api_history`; if still above **~64k tokens**, the oldest whole user turns are dropped permanently. A `[sys]` line reports what was removed. The estimate is recomputed via `chars/4`, not provider tokenizers.
- **`grep`**: literal substring search under workdir; uses **`rg`** when on PATH else in-tree walk; skips `.git` / `target` / `node_modules`; capped matches/files/bytes (`tools/grep.rs`)
- **`glob_file_search`**: glob under workdir (max **200** paths); skips same dirs as grep (`tools/glob_search.rs`)
- **Backups**: before overwriting an existing file, prior content is saved under **`.lightrs/backups/`** in the workdir (`tools/backup.rs`); pruned to the newest 256 snapshots per workdir
- **CLI `exec`**: `lightrs exec -p "…"` one-shot agent turn to stdout; `--json` for scripting; `--approve-shell` skips TUI approval (allowlist still applies); `--read-only` runs the turn in plan mode; `--output-schema <file-or-literal>` appends a JSON-schema instruction and requires the reply to parse as JSON
- **CLI `doctor --check`**: health report (config, keys, workdir, `rg`, rustc, local agent configs) without importing. When `[sandbox].on`, the backend check is a **live probe** (spawns `true` inside seatbelt/bwrap) — presence alone misleads: `bwrap` on PATH still fails when the kernel denies unprivileged userns (Debian default)
- **Shell deny list** (`tools/shell_deny.rs`): hard-block catastrophic commands (e.g. `rm -rf /`, fork bomb, `curl | sh`); not bypassed by TUI approval
- **Secret redaction**: tool results scrubbed for common key/token shapes before API history (`tools/redact.rs`, via `truncate_tool_api`)
- **Audit log**: every `run_command` decision appends a JSON line to `.lightrs/audit.log` — `{ts, tool, command, decision, result}` where decision is `auto` / `approved` / `approved_bypass` / `denied` / `denied_headless`. Command text **and** the short `result` line are secret-redacted. Rotates to `audit.log.1` past 256 KiB. Best-effort (failures are ignored). `.lightrs/` gets a `.gitignore` for machine state (`audit.log*`, `backups/`, `todos.json`) — `config.toml` stays committable.
- **Prompt-injection scan** (`tools/injection.rs`): tool results matching instruction-override patterns ("ignore previous instructions", "system prompt:", …) get a `[warning: …]` prefix prepended to the **API** history copy plus a `[sys]` transcript notice. Also applies to `[heal]` test output. Warning only — the result is still sent.
- **Workdir context**: `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `AIDER.md`, `.cursorrules`, `.windsurfrules`, `.github/copilot-instructions.md`, `.lightrs/AGENTS.md`, plus `*.md`/`*.mdc` under `.cursor/rules/` and `.windsurf/rules/` (frontmatter stripped), merged into system prompt (6 KiB cap each, ~12 KiB total). **User-global memory** appends after project sections: `~/.claude/CLAUDE.md` (Claude Code) and `~/.config/lightrs/AGENTS.md` — project files win the shared budget
- **Skills**: `skills/mod.rs` scans `SKILL.md` directories (`[skills].paths` first — declared roots win id collisions and the 64-cap — then OpenCode import tree, Cursor, Claude Code, pi, Codex). Catalog appended to system prompt; `$skill-id` in the user message attaches skill body to the API payload (UI unchanged). `@path` file attach unchanged (`mentions.rs`). Rules from `~/.config/lightrs/rules/*.md` merge into system prompt. A `trigger:` frontmatter keyword shows a non-blocking `[sys]` hint when the user input contains it (and `$id` wasn't already used).
- **Heal loop** (`agent/turn.rs::heal_plan`): when `[shell].test_command` is set, it runs after each successful `write_file`/`patch_file`/`multi_edit` (max `heal_steps` per user turn, default 3; bypasses allowlist, deny list still applies). A pass shows `heal ok` in the transcript; a failure appends the output to the API history so the model can fix it
- **Formatter** (`[shell].format_command`, `format_plan`): runs after each successful write/edit, before heal — `{path}` substitutes the diff path (else appended shell-quoted); same bypass/120s spawn as heal; failures are reported to the model without the fix-it framing

## Configuration

Priority: **CLI/env > project overlay > config file > defaults** (`config/file.rs`, `config.example.toml`). API-key env vars feed the CLI tier via clap `env=` args.

**Per-project overlay** (trust-gated): `<workdir>/.lightrs/config.toml` uses the same schema and layers over the global file, but only when the workdir is **trusted** — its canonical path is listed in global `[trust].dirs` (written by `/trust`) or the session started with `--trust` (one run). An untrusted project overlay is skipped with a startup `[sys]` notice (`/trust` to persist, `--trust` for one run). Only `model`, `fallback_model`, `weak_model`, `thinking`, `[shell]`, `[mcp.*]`, and `[lsp.*]` are applied — a project `[shell]` section replaces the global one entirely; `[mcp]`/`[lsp]` entries merge (same-name project wins). `provider`, `base_url`, `keys`, `endpoints`, `workdir`, and `extra_read_dirs` are parsed but ignored, so a repo file cannot redirect API calls, inject credentials, or widen its own read scope.

**`extra_read_dirs` / `--add-dir <path>`** — extra directories the agent may **read** (never write) beyond the workdir. `read_file`/`list_dir`/`grep`/`glob_file_search`/`lsp`/`@`-mentions resolve via `validate_read_path` (workdir → then each root, canonicalized). Global file + CLI flag only; writes stay confined to the workdir (use `/cd` to move write scope).

**Secret-bearing files** are written owner-only: `config.toml` (`save_tui_theme`, `migrate`, `import opencode` incl. the `opencode-import.toml` alt) and OpenCode `mcp.json` go through `write_private_file` — `mode(0o600)` at create plus `set_permissions` to normalize a pre-existing permissive file.

**`[endpoints]`** — per-provider base-url overrides (`kimi = "https://api.kimi.ai/coding/v1"`), used when a key targets a non-default endpoint (coding-plan/region variants). `AppConfig::endpoint_for(p)`: active provider's `base_url` wins → `[endpoints]` override → provider default. Migration writes it from OpenCode `options.baseURL` and known coding-plan auth ids.

### Shell allowlist

```toml
[shell]
allowlist = ["cargo ", "git ", "ls"]
```

When non-empty, `run_command` only runs if the command string equals or starts with an entry (prefix match). Blocked commands return an error string to the model; system prompt notes when allowlist is active.

### Shell approval (TUI)

```toml
[shell]
require_approval = true
```

When `require_approval` is true, the TUI prompts **y/n** before `run_command`, except **likely read-only** commands (`ls`, `git status`, `cargo check`, …) auto-run (`tools/shell_readonly.rs`; deny list still wins; `KEY=val` env prefixes are peeled before classifying). When the allowlist blocks a command, the same prompt appears; **y** runs it once (bypass allowlist for that invocation only). **n** or **Esc** denies and the tool returns `error: user denied shell command` to the model. While prompting, the input area shows the command(s) and other keys are ignored.

**Batched**: all `run_command` calls in one model step that need approval are collected into a **single** prompt (up to 3 command lines shown + `… +n more`); **y** approves every command in the step, **n** denies all. Read-only calls in the same step still auto-run. **ACP / headless** auto-runs the same read-only set; other commands fail closed without a TUI.

### OS sandbox for spawned commands

```toml
[sandbox]
on = true                                  # default false
writable = ["~/.cargo", "~/.rustup", "~/.npm"]  # extra writable roots (toolchain caches)
network = true                             # default true; false closes the exfil path
```

When `on`, every spawned child — `run_command`, `format`/`test` hooks, `[mcp.*]`/`[lsp.*]` servers — runs confined: **workspace-write** semantics (writes allowed only under the workdir + tmp + `writable` roots; read/exec everywhere; network per the flag). Backends: `sandbox-exec` (seatbelt) on macOS, `bwrap` on Linux; **fail-closed** when enabled with no backend. File tools are *not* sandboxed — they're already confined in-process by `validate_work_path` (the agent itself must write sessions/config outside workdir, so process-wide jails don't apply). Honest limit: network-on confinement stops destructive writes but not exfiltration of readable files — set `network = false` for that.

**Overlay rule**: `on`/`network` are GLOBAL-only — a project `[sandbox]` may add `writable` roots but can never weaken confinement (same redirect-attack class as `endpoints`). When a confined command exits non-zero with a permission-denial signature (`Operation not permitted` / `Permission denied` / `deny(`), the result appends a confinement hint naming `writable`/`on` as the escape — a raw EPERM otherwise sends the model hunting in the wrong place.

### MCP servers

```toml
[mcp.docs]
command = "npx -y @modelcontextprotocol/server-filesystem /tmp"
```

Each `[mcp.<name>]` registers a stdio MCP server reachable through `mcp_call`. Servers spawn lazily on first call and are killed at drop (whole process group — a launcher's children can't outlive it); framing is newline-delimited JSON-RPC 2.0 (the common stdio-server shape — not Content-Length). Response lines cap at 4 MiB (newlineless streams can't grow unbounded during the 30s deadline) and result text at 64 KiB. Project overlays may add/override entries (same risk profile as `test_command`).

### Language servers

```toml
[lsp.rust]
command = "rust-analyzer"
extensions = ["rs"]          # file extensions routed to this server

[lsp.ts]
command = "typescript-language-server --stdio"
extensions = ["ts", "tsx"]
```

Each `[lsp.<lang>]` registers a stdio language server for the `lsp` tool. `extensions` is required — it maps a file path to a server so the tool schema stays language-free. `command` runs via `sh -c` in the workdir (rootUri). Frames cap at 8 MiB, header lines at 8 KiB; drop kills the whole process group.

### Heal / post-edit check

```toml
[shell]
test_command = "cargo test"   # or "go test ./...", "npm test", "pytest"
heal_steps = 3                # max runs per user turn
```

### Model fallback

```toml
fallback_model = "kimi-k2-0905-preview"   # retried once on provider error
weak_model = "glm-4.5-air"                # read-only steps; escalates on write/exec
```

### Skills

```toml
[skills]
enabled = true
paths = []   # extra roots with skill subdirs or SKILL.md
```

Disabled when `enabled = false`. Catalog and rules refresh on each send and on `/reload` / `/skills`.

### Auto config refresh

Before each user message, if `config.toml` **or** `.lightrs/config.toml` mtime changed, `[shell]` is reloaded (same as `/reload` shell portion) without restarting the binary; `agent.heal` is re-synced too. `/cd` also re-reads the overlay for the new workdir.