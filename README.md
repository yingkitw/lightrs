# lightrs

A minimal terminal coding agent inspired by pi / opencode. Single Rust binary, OpenAI-compatible chat + tools against **GLM (Z.AI)**, **Kimi (Moonshot)**, and **MiniMax**.

## Install

```bash
cargo install --path .
# or
cargo build --release
```

## API keys

Set one key for your provider (or use flags / config file):

| Provider | Environment variable |
|----------|----------------------|
| GLM      | `ZAI_API_KEY` or `GLM_API_KEY` |
| Kimi     | `MOONSHOT_API_KEY` |
| MiniMax  | `MINIMAX_API_KEY` |

Optional config: `~/.config/lightrs/config.toml` (see `config.example.toml`).

## Usage

```bash
# Kimi (default model kimi-k2-0905-preview)
export MOONSHOT_API_KEY=...
lightrs --provider kimi

# GLM
export ZAI_API_KEY=...
lightrs --provider glm --model glm-4.6

# MiniMax
export MINIMAX_API_KEY=...
lightrs --provider minimax --model MiniMax-M2.1

# Custom base URL (proxy / China endpoint)
lightrs --provider kimi --base-url https://api.moonshot.cn/v1
```

### Keys in TUI

- `Enter` — send message
- `Esc` — clear input
- `Ctrl+C` / `q` — quit
- `/clear` — reset conversation
- `/cwd` — print working directory

The agent can **read/write files**, **list directories**, and **run shell commands** in the current working directory (same scope as you launching the binary).

## Design

- One OpenAI-compatible HTTP client for all backends
- Small fixed tool set; no MCP, no LSP, no embeddings
- Async LLM calls on a Tokio runtime; sync TUI on the main thread

See `ARCHITECTURE.md` and `SPEC.md` for details.