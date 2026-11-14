mod acp;
mod agent;
mod chat;
mod cli;
mod config;
mod opencode;
mod provider;
mod tools;
mod tui;

use anyhow::Result;

use crate::cli::{Cli, Command, ImportSource};
use crate::config::AppConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = <Cli as clap::Parser>::parse();
    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => tui::run(AppConfig::load(&cli.model)?).await,
        Command::Acp { protocol } => {
            let cfg = AppConfig::load(&cli.model)?;
            acp::serve(cfg, protocol).await.map_err(|e| anyhow::anyhow!("{e}"))
        }
        Command::Import { source } => match source {
            ImportSource::Opencode { config, project } => opencode::import(config, project),
        },
    }
}