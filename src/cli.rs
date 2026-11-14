use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::Provider;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum AcpProtocol {
    #[default]
    V1,
    #[cfg(feature = "acp-v2")]
    V2,
}

#[derive(Debug, Parser)]
#[command(
    name = "lightrs",
    about = "Lightweight TUI / ACP coding agent (GLM, Kimi, MiniMax)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub model: ModelFlags,

    /// On startup, import from OpenCode / Claude Code / Aider if config.toml is missing
    #[arg(long, default_value_t = true)]
    pub auto_migrate: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Interactive terminal UI (default)
    #[command(name = "tui", visible_alias = "run")]
    Tui,
    /// Agent Client Protocol server on stdio (for Zed, JetBrains, etc.)
    #[command(name = "acp")]
    Acp {
        #[arg(long, value_enum, default_value = "v1")]
        protocol: AcpProtocol,
    },
    /// Copy settings from OpenCode into the lightrs config directory
    #[command(name = "import")]
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },
    /// Scan this machine for agent configs and import the best match
    #[command(name = "migrate", visible_alias = "doctor")]
    Migrate {
        #[arg(long)]
        dry_run: bool,
        /// Print setup health (config, keys, workdir, rg) without importing
        #[arg(long, conflicts_with_all = ["dry_run", "from"])]
        check: bool,
        #[arg(long, help = "Only migrate from this tool (opencode, claude-code, aider, pi, cursor)")]
        from: Option<String>,
    },
    /// Run one prompt without the TUI (stdout or JSON)
    Exec {
        /// Prompt text (non-interactive agent turn)
        #[arg(short)]
        prompt: String,
        /// Print `{"text":...}` instead of plain text
        #[arg(long)]
        json: bool,
        /// Auto-approve shell commands (no TUI); allowlist still applies
        #[arg(long)]
        approve_shell: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ImportSource {
    /// Model keys, MCP, skills paths, instructions/rules, hooks
    Opencode {
        /// Global opencode config (default `~/.config/opencode/opencode.json`)
        #[arg(long)]
        config: Option<PathBuf>,
        /// Project config to merge (walk cwd for `opencode.json` / `.opencode/opencode.json`)
        #[arg(long)]
        project: Option<PathBuf>,
    },
}

#[derive(Debug, Parser, Clone)]
pub struct ModelFlags {
    #[arg(long, value_enum)]
    pub provider: Option<Provider>,

    #[arg(long)]
    pub model: Option<String>,

    #[arg(long)]
    pub base_url: Option<String>,

    #[arg(long, default_value = ".")]
    pub workdir: PathBuf,

    #[arg(long, env = "ZAI_API_KEY")]
    pub glm_api_key: Option<String>,

    #[arg(long, env = "MOONSHOT_API_KEY")]
    pub kimi_api_key: Option<String>,

    #[arg(long, env = "MINIMAX_API_KEY")]
    pub minimax_api_key: Option<String>,
}