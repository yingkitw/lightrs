use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::agent::Agent;
use crate::config::AppConfig;
use crate::tools::ShellPolicy;

pub async fn run_prompt(
    mut cfg: AppConfig,
    prompt: String,
    json: bool,
    approve_shell: bool,
    output_schema: Option<String>,
) -> Result<String> {
    // `--approve-shell` widens policy by mutating cfg before from_config —
    // every agent-built-from-config feature (mcp/lsp/read_only/weak/…)
    // reaches exec for free, and the mutation stays exec-local.
    if approve_shell {
        cfg.shell_policy = ShellPolicy::new(cfg.shell_policy.allowlist().to_vec(), false);
    }
    let workdir: Arc<Path> = Arc::from(cfg.workdir.as_path());
    let mut agent = Agent::from_config(&cfg, Arc::clone(&workdir));
    // `--output-schema` accepts a literal JSON schema or a path to a file.
    let output_schema = output_schema
        .map(|s| {
            let p = std::path::Path::new(&s);
            if p.is_file() {
                std::fs::read_to_string(p).unwrap_or(s)
            } else {
                s
            }
        });
    if cfg.untrusted_project {
        eprintln!("note: .lightrs/config.toml skipped — dir untrusted (use --trust or /trust)");
    }
    if let Some(cmd) = &cfg.test_command {
        agent.set_heal(Some(crate::agent::HealSetting {
            command: cmd.clone(),
            max_steps: cfg.heal_steps,
        }));
    }
    let catalog = crate::skills::load(&crate::skills::load_settings());
    agent.set_skill_catalog(catalog);
    let agent = Arc::new(tokio::sync::Mutex::new(agent));
    let prompt = match &output_schema {
        Some(schema) => format!(
            "{prompt}\n\nRespond with only a JSON value matching this JSON schema — no prose, no code fences:\n{schema}"
        ),
        None => prompt,
    };
    let text = Agent::run_user_turn_headless(agent, prompt, |_| {}).await?;
    if output_schema.is_some() {
        validate_json_reply(&text)?;
    }
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

/// Strip a lone ```json fence then require the reply to parse as JSON —
/// `--output-schema` validates syntactically, not against the schema itself
/// (a full JSON-schema validator would be a heavy dep for this charter).
fn validate_json_reply(text: &str) -> Result<()> {
    let trimmed = text.trim();
    let candidate = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    if serde_json::from_str::<serde_json::Value>(candidate).is_err() {
        anyhow::bail!("reply did not parse as JSON (--output-schema)");
    }
    Ok(())
}

pub fn run_prompt_blocking(
    cfg: AppConfig,
    prompt: String,
    json: bool,
    approve_shell: bool,
    output_schema: Option<String>,
) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_prompt(cfg, prompt, json, approve_shell, output_schema))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_json_reply_accepts_plain_and_fenced() {
        validate_json_reply("{\"a\":1}").unwrap();
        validate_json_reply("```json\n{\"a\":1}\n```").unwrap();
        validate_json_reply("[1,2]").unwrap();
    }

    #[test]
    fn validate_json_reply_rejects_prose() {
        assert!(validate_json_reply("sure, here is the answer").is_err());
        assert!(validate_json_reply("```\nnot json\n```").is_err());
    }
}
