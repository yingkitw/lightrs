# Context Management

How lightrs bounds what it sends to the model, what it stores, and how to see it.

## Why not send full history?

Every model call pays for its input tokens — resending a whole conversation each
step is the single biggest latency and cost driver in a coding agent. lightrs
therefore keeps two distinct views of the conversation:

| Layer | What | Mutable? | Code |
|---|---|---|---|
| **Stored history** | `agent.api_history` — everything; backs `/rewind`, `/context` counts, session save | Yes, by compaction only | `agent/context/compact.rs` |
| **Wire history** | The bounded copy actually sent per request | Rebuilt per request | `agent/context/wire.rs` |

The wire layer is **non-destructive**: `messages_for_api` returns a trimmed copy;
`api_history` keeps the full conversation. Nothing the model can't see is lost.

## Wire layer (per request)

`wire::messages_for_api` builds the request payload:

1. **System prompt** — always first, never dropped.
2. **Active-turn expansion** — `@file` contents and `$skill` bodies are appended
   to the *current* user message only. Older user messages keep their composer
   text but have legacy attachment blocks stripped (`strip_legacy_user_expansions`)
   — attachments don't ride along forever.
3. **Turn window** — at most the last `MAX_RESENT_TURNS = 12` user turns.
   Whole turns only: an assistant `tool_calls` message is never split from its
   `tool` results, and the in-flight turn is never dropped.
4. **Token budget** — `WIRE_BUDGET_TOKENS = 48_000` (estimated); oldest whole
   turns drop until the copy fits.

## Stored layer (persistent)

`auto_compact_context` runs inside `build_chat_request` on the **raw**
`api_history` estimate — not the wire estimate, which is already bounded:

- `≥ AUTO_COMPACT_TOKENS (32k est)` — stub the bodies of all but the most recent
  `COMPACT_KEEP_TOOLS = 6` tool results (`[compacted tool result — use read_file
  if needed]`) and replace tool-call arguments `> 512 B` outside that window
  with `"{}"`.
- `> HISTORY_HARD_CAP_TOKENS (64k est)` — additionally drop oldest whole turns
  permanently, still never splitting tool pairs or eating the in-flight turn.
- Compaction emits one `[sys]` line summarizing what was stubbed/dropped.

`/compact` applies the same stubbing policy on demand; `/clear` resets history
entirely. `MAX_UI_MESSAGES = 500` caps the rendered transcript (dropped at turn
end and session restore).

## Per-message caps

- **Tool results**: truncated to `TOOL_API_MAX = 6_144` chars before they enter
  `api_history` (`truncate_tool_api` — appended at tool-result record time).
- **Tool-call args**: full-fidelity while recent; stubbed to `"{}"` once old
  enough — a `write_file` of a 20 KB file doesn't cost 20 KB on every later
  request.

## Estimates are estimates

Token math is `chars / 4` (`tokens::estimate_context_tokens`) — deliberately
dependency-free, not a provider tokenizer. Drift is visible and intended: the
header shows `~Nk ctx est` next to real usage (`last Xk+Yk`), and `/context`
shows both so you can eyeball calibration.

## Transparency surface

| Where | What it shows |
|---|---|
| Header | `N msg · ~Xk ctx est · Yk session · last A in + B out` |
| `/context` | `limits:` all budgets · `wire: M of N turns · K msgs` · `breakdown:` by role (sys/user/asst/tool-out/call-args) · `top:` heaviest messages · `turn:` real usage |
| `/status` | Session + the same `tokens:` line |
| Per step | `model · X in + Y out` real usage lines |
| Turn end | `turn · N call(s) · X in + Y out` |
| First trim | `[sys] context window: sending last M of N turns …` (once per session) |
| `/quiet` | Suppresses the routine sys lines only — header and `/context` unaffected |

## Invariants

- Never split an assistant `tool_calls` message from its `tool` results.
- Never drop the in-flight user turn (wire window or compaction).
- Wire trimming never mutates `api_history`.
- Stored compaction is the only destructive path, and it always preserves the
  newest turn.

## Tests

- `agent/context/wire.rs` — window, budget, identity-under-limits, pair safety.
- `agent/context/compact.rs` — stubbing, arg caps, whole-turn drops.
- `agent/context/tokens.rs` — estimate, breakdown, ranking, usage accumulation.
- `tests/context_budget.rs` — e2e via a local HTTP mock: captured request bodies
  prove the wire caps user turns at 12 and that compaction stubs reach the wire.
