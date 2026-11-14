#[cfg(feature = "acp")]
mod acp;
mod agent;
mod chat;
mod cli;
mod config;
mod migrate;
mod opencode;
mod provider;
mod paths;
mod session;
mod tools;
mod tui;

use anyhow::Result;

use crate::cli::{Cli, Command, ImportSource};
use crate::config::AppConfig;

fn main() -> Result<()> {
    let cli = <Cli as clap::Parser>::parse();
    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => tui::run(AppConfig::load(&cli.model, cli.auto_migrate)?),
        Command::Acp { protocol } => {
            #[cfg(feature = "acp")]
            {
                let cfg = AppConfig::load(&cli.model, cli.auto_migrate)?;
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(acp::serve(cfg, protocol))
                    .map_err(|e| anyhow::anyhow!("{e}"))
            }
            #[cfg(not(feature = "acp"))]
            {
                let _ = protocol;
                anyhow::bail!("ACP support disabled; rebuild with `--features acp`")
            }
        }
        Command::Import { source } => match source {
            ImportSource::Opencode { config, project } => opencode::import(config, project),
        },
        Command::Migrate { dry_run, from } => migrate::run_migrate(dry_run, from.as_deref()),
    }
}