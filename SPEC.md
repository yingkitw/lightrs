# Specification

## Goals

- Terminal UI coding assistant with low footprint and fast startup
- Pluggable backends: GLM, Kimi, MiniMax via OpenAI Chat Completions + tools
- Safe-by-scope: tools only affect configured working directory

## Modes

- **TUI** — default (`lightrs` / `lightrs tui`)
- **ACP v1** — `lightrs acp` (initialize, authenticate, session/new, session/prompt, session/cancel); requires `acp` feature (default)
- **ACP v2** — `lightrs acp --protocol v2` (baseline session lifecycle, async prompt + `session/update`)
- **Import** — `lightrs import opencode` merges OpenCode JSON into `~/.config/lightrs/`; provider `options.apiKey` values (all mapped providers, `{env:VAR}` resolved) plus `~/.local/share/opencode/auth.json` `type:"api"` keys are carried into `[keys]` — explicit config keys win over auth.json
- **Migrate** — `lightrs migrate` / `lightrs tui --auto-migrate` scans OpenCode, Claude Code, Codex, Aider

## Non-goals (v0.1)

- In-process MCP server — `lightrs import opencode` writes `~/.config/lightrs/mcp.json` only; no stdio MCP host in the agent loop yet
- LSP, full regex/ripgrep parity, multi-agent orchestration
- Sandboxed containers for shell execution
- Side-by-side or inline patched diff editor (unified diff in transcript only)

## User interface

### Transcript

- Roles: user, assistant, tool, system, error
- **`[sys]`**: ephemeral UI-only (`UiRole::System`); styled with `▏ sys` bars (`tui/highlight/sys.rs`); never sent to the API; **excluded** from `session.rs` save/restore
- **Markdown**: full pulldown pass on message bodies — headings, lists, **bold** / *italic* / ~~strike~~, links (styled), blockquotes (`▏`), pipe tables (`│`), rules; strikethrough/task lists enabled
- **Fenced code**: syntect highlight + wrap
- **Mermaid** (`mermaid` / `mmd`): laid-out ASCII diagram + source — `flowchart TD`/`graph` renders boxed nodes in topological layers with `▼`/`─`/`┬` connectors (labels on wide spans); `flowchart LR` renders column-major with `├─►`; `sequenceDiagram` renders per-message rows. Cycles, `BT`/`RL`, over-width or >48-node graphs fall back to a flat edge list. Laid-out diagrams render on a solid panel — `code_bg` layout area, `inline_code_bg` box fill, `muted` borders, `text` labels, `mermaid_edge` connectors (rows padded to a common block width)
- **Tool blocks**: `◌ name args … running` while the call executes, replaced in place by the result with an elapsed suffix (`· 1.2s`); header row keeps a one-line call summary (args or file +/−) in both states; collapsed shows truncated result, expanded shows full args/result — tool bodies render **preformatted** (literal lines, no markdown), and ANSI escape sequences are stripped at capture and render. Results beginning `error:` render in the error color; other results use the normal body color
- **Tokens**: muted `[sys]` lines per turn — `send · ~N ctx` after your message (estimate), `model · N in + M out` after each API step when the provider reports usage, and a `turn · N calls · X in + Y out · Ns` close line (real usage + wall-clock)
- **`write_file`** / **`patch_file`**: colorized unified diff (`+`/`-`/`@@`); `+N −M` in collapsed header; full diff when expanded; API tool body remains a one-line summary
- **Mouse**: wheel scroll; left-click toggles tool expand (with `o` when input empty); drag = native terminal selection (copy) — the app only requests `?1000` click/scroll reporting, never `?1002` drag tracking
- **Busy**: spinner + optional “waiting for model…” when streaming has not started
- **Terminal title**: OSC 2 set once per loop pass when it changes — `lightrs · dir[:branch]` with a `· ●` suffix while `Busy`; cleared on exit so the shell title takes over

### Chrome

- Header: version, provider, model, state, `workdir:branch` (git branch from `.git/HEAD`, worktree-safe), context estimate + `%` of the model window when known (`config::model_context_window`), API host
- Multi-line composer (not single-line only); placeholder hints when empty
- Status bar: slash hints, scroll/tool hints, or busy/error text; when scrolled up it shows `↑N scrolled · End latest`
- **Theme**: selectable palettes via `/theme` (`tui/theme/`; default `mocha`)

### Input

- `Enter` send; `Shift+Enter` / `Ctrl+J` newline
- `@relative/path` file mentions in the composer; `Tab` completes paths under **workdir** (status bar shows matches). On send, matching files attach to the **API** user message only (12 KiB per file, 36 KiB total); the transcript shows your composer text unchanged
- `$skill-id` invokes discovered skills (`Tab` on `$` prefix); skill bodies attach to the API message (see **Skills** below)
- `Ctrl+P` / `Ctrl+N` prompt history
- Scroll transcript when input empty (`↑`/`↓`, PgUp/PgDn, Home/End)
- Interrupt while busy: `Esc`, `Ctrl+C`, `Ctrl+Q`, or send a bare `stop` / `wait` / `halt` (does not quit)
- **Queued input**: typing while busy edits the composer; `Enter` queues the line (`queued N` in the status bar) and it sends automatically when the turn ends or is interrupted. Slash commands queued the same way run when idle

### Slash commands (not sent to model)

| Command | Action |
|---------|--------|
| `/help`, `/?` | List commands |
| `/clear` | Clear API + UI history, delete session file |
| `/cwd` | Print working directory |
| `/cd` [path] | Change working directory (`~`, `~/x`, absolute, or relative; keeps the conversation; session file follows the new workdir) |
| `/todo` | Show the task list written by `todo_write` (`.lightrs/todos.json`) |
| `/editor` | Compose the prompt in `$EDITOR` (e.g. `vim`, `code --wait`); buffer loads into the composer on save |
| `/status` | Session / model details + token usage |
| `/model` | Bare `/model` fetches `GET /models` for every provider with a resolvable key in the background and renders a numbered list (`←` marks current; unkeyed/failed providers show an error row). `/model <n>` applies row n; `/model <name>` switches model on the same provider (keeps base URL + key); `/model <provider> [name]` switches provider (default endpoint, provider default model when omitted; key re-resolved from config/env — fails fast when missing). Header + context % update |
| `/context` | Context budgets (`limits:`), wire vs stored turn counts, per-role breakdown, heaviest messages, current-turn in/out — full policy in `docs/context.md` |
| `/reload` | Re-read `config.toml` `[shell]`; rescan skills; refresh agent system prompt |
| `/quiet` | Toggle `[sys]` token lines (`send · ~N ctx`, per-step in/out, turn total); header + `/context` unchanged |
| `/skills` | List discovered skills (`$id` to invoke on send) |
| `/export` [path] | Write UI transcript to markdown under workdir (default `lightrs-transcript.md`; skips `[sys]`) |
| `/copy` [`code`] | Copy latest assistant reply to the OS clipboard; `code` = last fenced ``` block only |
| `/theme` [`id`] | List palettes or set `mocha` / `hc`; saved under `[tui]` in `config.toml` |
| `/compact` | Replace older API tool bodies + assistant `tool_calls` argument blobs with placeholders (keeps last 6 tool results); no LLM call |
| `/rewind` [`n`] | Drop the last `n` user turns (default 1) from API + UI history at user-turn boundaries, then resave the session |
| `/diff` | Replay this session's stored write diffs as expandable tool rows (`+N −M` totals per file; no filesystem reads) |
| `/undo` | Restore files edited since your last message from `.lightrs/backups/` |
| `/quit` | Exit (same as `q` when idle) |

`Tab` completes unique slash prefix.

**Project commands** (`src/commands.rs`): `.claude/commands/*.md` under the workdir are discovered as `/name` (filename stem); `description:` frontmatter shows in hints, and the body is sent to the model with any args appended. Built-ins win on name collisions. Rescanned on `/cd`, `/reload`, sends, and unknown-slash dispatch.

### Session resume

Per canonical **workdir**, `~/.config/lightrs/sessions/<hash>.json` stores API history, UI messages (including `tool_call`, `file_diff`), and token stats. **`[sys]` lines are ephemeral**: shown in the live transcript but omitted from session save/restore. `/clear` wipes disk + memory. Saves are atomic (temp file + same-dir rename — a crash leaves the old or new file, never partial JSON). The transcript keeps at most **500 messages** (`MAX_UI_MESSAGES`; oldest drop first) so session files and per-frame rendering stay bounded.

## Agent behavior

- System prompt: concise coding helper; must use tools for filesystem/shell facts
- Loop: user message → model → (optional) tool calls → tool results → model until text reply or step cap (**16**)
- **Lean API context**: persisted user rows are composer text only; `@` file bodies and `$skill` bodies attach for the **active turn** only. Prior turns omit attachments when resent. Tool results in API history are capped (**6 KiB**); full output may still appear in the TUI.
- **Streaming**: assistant text appended live when supported; tool calls assembled from stream deltas before execution
- Tools: `read_file`, `write_file`, `patch_file`, `multi_edit`, `list_dir`, `grep`, `glob_file_search`, `run_command`, `todo_write` (schemas in `tools/schemas.rs`)
- **`patch_file`**: exact match first; single-occurrence edits fall back to whitespace/indent-tolerant match (`tools/edit_match.rs`). **`replace_all`** requires exact `old_string` matches.
- **`run_command`**: stdout and stderr each capped at **64 KiB** with a truncation note in the tool result; ANSI escape sequences stripped.
- **`todo_write`**: replaces the whole task list (`items[].content`, `items[].status` ∈ `pending|in_progress|completed`, max 32) persisted to `.lightrs/todos.json` (`tools/todo.rs`); view with `/todo`
- **HTTP**: chat requests use 120s timeout, 30s connect timeout, up to **3** retries on 429/5xx and transient transport errors (`provider/http.rs`).
- **Fallback model**: `--fallback-model <m>` or `fallback_model` in `config.toml` retries a failed model fetch **once** on the secondary model (any non-interrupt error); a `[sys]` line notes the switch
- **Weak-model routing**: `--weak-model <m>` or `weak_model` runs each user turn on the cheaper model until a step calls a write/exec tool (`write_file`, `patch_file`, `multi_edit`, `run_command`), then escalates to `model` for the rest of that turn (one `[sys]` note per escalation). Next turn starts weak again.
- **Context pressure**: when estimated API context exceeds **~20k tokens**, the TUI shows a `[sys]` hint (respects `/quiet`).
- **Wire window**: the model never receives the full stored history — each request is trimmed to the **last 12 user turns**, then oldest whole turns drop until the wire payload fits **~48k est tokens** (never the in-flight turn, never split tool_call pairs). `api_history` keeps everything for `/rewind` and session save; a one-time `[sys]` note announces the first trim, and `/context` shows `wire: X of Y turns`.
- **Auto-compact**: before each request, if the *stored* history estimate exceeds **~32k tokens**, old tool results and assistant `tool_calls` argument blobs (>512 chars) are stubbed in `api_history`; if still above **~64k tokens**, the oldest whole user turns are dropped permanently. A `[sys]` line reports what was removed. The estimate is recomputed via `chars/4`, not provider tokenizers.
- **`grep`**: literal substring search under workdir; uses **`rg`** when on PATH else in-tree walk; skips `.git` / `target` / `node_modules`; capped matches/files/bytes (`tools/grep.rs`)
- **`glob_file_search`**: glob under workdir (max **200** paths); skips same dirs as grep (`tools/glob_search.rs`)
- **Backups**: before overwriting an existing file, prior content is saved under **`.lightrs/backups/`** in the workdir (`tools/backup.rs`)
- **CLI `exec`**: `lightrs exec -p "…"` one-shot agent turn to stdout; `--json` for scripting; `--approve-shell` skips TUI approval (allowlist still applies)
- **CLI `doctor --check`**: health report (config, keys, workdir, `rg`, rustc, local agent configs) without importing
- **Shell deny list** (`tools/shell_deny.rs`): hard-block catastrophic commands (e.g. `rm -rf /`, fork bomb, `curl | sh`); not bypassed by TUI approval
- **Secret redaction**: tool results scrubbed for common key/token shapes before API history (`tools/redact.rs`, via `truncate_tool_api`)
- **Audit log**: every `run_command` decision appends a JSON line to `.lightrs/audit.log` — `{ts, tool, command, decision, result}` where decision is `auto` / `approved` / `approved_bypass` / `denied` / `denied_headless`. Command text is redacted; `result` is a short status line. Best-effort (failures are ignored).
- **Prompt-injection scan** (`tools/injection.rs`): tool results matching instruction-override patterns ("ignore previous instructions", "system prompt:", …) get a `[warning: …]` prefix prepended to the **API** history copy plus a `[sys]` transcript notice. Also applies to `[heal]` test output. Warning only — the result is still sent.
- **Workdir context**: `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `AIDER.md`, `.cursorrules`, `.windsurfrules`, `.github/copilot-instructions.md`, `.lightrs/AGENTS.md`, plus `*.md`/`*.mdc` under `.cursor/rules/` and `.windsurf/rules/` (frontmatter stripped), merged into system prompt (6 KiB cap each, ~12 KiB total)
- **Skills**: `skills/mod.rs` scans `SKILL.md` directories (`[skills].paths` first — declared roots win id collisions and the 64-cap — then OpenCode import tree, Cursor, Claude Code, pi, Codex). Catalog appended to system prompt; `$skill-id` in the user message attaches skill body to the API payload (UI unchanged). `@path` file attach unchanged (`mentions.rs`). Rules from `~/.config/lightrs/rules/*.md` merge into system prompt. A `trigger:` frontmatter keyword shows a non-blocking `[sys]` hint when the user input contains it (and `$id` wasn't already used).
- **Heal loop** (`agent/turn.rs::maybe_run_heal`): when `[shell].test_command` is set, it runs after each successful `write_file`/`patch_file`/`multi_edit` (max `heal_steps` per user turn, default 3; bypasses allowlist, deny list still applies). A pass shows `heal ok` in the transcript; a failure appends the output to the API history so the model can fix it

## Configuration

Priority: **CLI/env > project overlay > config file > defaults** (`config/file.rs`, `config.example.toml`). API-key env vars feed the CLI tier via clap `env=` args.

**Per-project overlay**: `<workdir>/.lightrs/config.toml` uses the same schema and layers over the global file. Only `model`, `fallback_model`, `weak_model`, and `[shell]` are applied — a project `[shell]` section replaces the global one entirely. `provider`, `base_url`, `keys`, and `workdir` are parsed but ignored, so a repo file cannot redirect API calls or inject credentials.

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