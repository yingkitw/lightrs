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
    #[arg(long, value_enum, default_value = "kimi")]
    pub provider: Provider,

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