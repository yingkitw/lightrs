use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::agent::Agent;
use crate::config::AppConfig;
use crate::provider::LlmClient;
use crate::tools::ShellPolicy;

pub async fn run_prompt(cfg: AppConfig, prompt: String, json: bool, approve_shell: bool) -> Result<String> {
    let shell_policy = if approve_shell {
        ShellPolicy::new(
            cfg.shell_policy.allowlist().to_vec(),
            false,
        )
    } else {
        cfg.shell_policy.clone()
    };
    let client = LlmClient::new(cfg.base_url.clone(), cfg.api_key.clone(), cfg.model.clone());
    let workdir: Arc<Path> = Arc::from(cfg.workdir.as_path());
    let mut agent = Agent::new(client, Arc::clone(&workdir), shell_policy);
    let catalog = crate::skills::load(&crate::skills::load_settings());
    agent.set_skill_catalog(catalog);
    let text = agent.run_user_turn(prompt).await?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "text": text,
                "workdir": cfg.workdir.display().to_string(),
                "model": cfg.model,
            })
        );
    } else {
        println!("{text}");
    }
    Ok(text)
}

pub fn run_prompt_blocking(
    cfg: AppConfig,
    prompt: String,
    json: bool,
    approve_shell: bool,
) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_prompt(cfg, prompt, json, approve_shell))?;
    Ok(())
}