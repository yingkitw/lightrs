#[cfg(feature = "acp")]
pub mod acp;
pub mod agent;
pub mod audit;
pub mod chat;
pub mod cli;
pub mod commands;
pub mod config;
pub mod doctor;
pub mod exec;
pub mod lsp;
pub mod mcp;
pub mod mentions;
pub mod migrate;
pub mod opencode;
pub mod paths;
pub mod provider;
pub mod session;
pub mod skills;
pub mod tools;
pub mod tui;

use anyhow::Result;

use crate::cli::{Cli, Command, ImportSource};
use crate::config::AppConfig;

pub fn run() -> Result<()> {
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
        Command::Migrate {
            dry_run,
            from,
            check,
        } => {
            if check {
                let cfg = AppConfig::load(&cli.model, cli.auto_migrate)?;
                doctor::run_check(&cfg)
            } else {
                migrate::run_migrate(dry_run, from.as_deref())
            }
        }
        Command::Exec {
            prompt,
            json,
            approve_shell,
            output_schema,
        } => {
            let cfg = AppConfig::load(&cli.model, cli.auto_migrate)?;
            exec::run_prompt_blocking(cfg, prompt, json, approve_shell, output_schema)
        }
    }
}