//! `/model` — provider/model switching: arg resolution, background `/models`
//! fetch, numbered choice list, and the switch decision.

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

use crate::config::{AppConfig, Provider};
use crate::provider::LlmClient;
use crate::tui::App;
use crate::tui::app::AgentEvent;
use crate::tui::info::provider_name;

pub(super) struct ModelSwitch {
    pub(super) provider: Provider,
    pub(super) model: String,
    pub(super) base_url: String,
    pub(super) api_key: String,
}

/// `/model` args → `Some(switch)` or `None` when called bare (list fetched).
/// Forms: `/model <n>` (pick from the last fetched list),
/// `/model <name>` (same provider, keeps custom `--base-url` and key), or
/// `/model <provider> [name]` (provider default model when name omitted).
pub(super) fn resolve_model_args(args: &[&str], app: &App) -> Result<Option<ModelSwitch>> {
    let cfg = &app.config;
    let Some(first) = args.first() else {
        return Ok(None);
    };
    match (Provider::from_str(first, true).ok(), args.get(1)) {
        (Some(p), name) => model_switch(p, name.copied().unwrap_or_else(|| p.default_model()), cfg)
            .map(Some),
        (None, None) => {
            if let Ok(n) = first.parse::<usize>() {
                return match n.checked_sub(1).and_then(|i| app.model_choices.get(i)) {
                    Some((p, m)) => model_switch(*p, m, cfg).map(Some),
                    None => bail!("no model #{n} — run /model first to fetch the list"),
                };
            }
            Ok(Some(ModelSwitch {
                provider: cfg.provider,
                model: (*first).into(),
                base_url: cfg.base_url.clone(),
                api_key: cfg.api_key.clone(),
            }))
        }
        (None, Some(extra)) => {
            bail!("unexpected argument `{extra}` — usage: /model [n|name|provider name]")
        }
    }
}

/// Spawn the background `/models` fetch for every provider with a resolvable
/// key (current provider keeps its custom base_url). Returns the "fetching…"
/// note, or `None` when the event plumbing isn't wired (tests).
pub(super) fn start_model_fetch(app: &App) -> Option<String> {
    let (rt, tx) = (app.rt.clone()?, app.ev_tx.clone()?);
    let mut targets = Vec::new();
    for p in [Provider::Glm, Provider::Kimi, Provider::Minimax] {
        let (base, key) = if p == app.config.provider {
            (app.config.base_url.clone(), Ok(app.config.api_key.clone()))
        } else {
            (
                app.config.endpoint_for(p),
                AppConfig::resolve_runtime_api_key(p)
                    .map_err(|_| format!("no key — set {}", p.env_var())),
            )
        };
        targets.push((p, base, key));
    }
    rt.spawn(async move {
        let mut results = Vec::new();
        for (p, base, key) in targets {
            let res = match key {
                Err(e) => Err(e),
                Ok(key) => LlmClient::new(base, key, String::new())
                    .list_models()
                    .await
                    .map_err(|e| format!("{e:#}")),
            };
            results.push((p, res));
        }
        let _ = tx.send(AgentEvent::ModelsListed(results));
    });
    Some(format!(
        "model: {} · {} · {}\nfetching model lists…",
        provider_name(app.config.provider),
        app.config.model,
        app.config.base_url
    ))
}

/// Flatten fetch results into numbered `/model <n>` choices (display order).
pub(crate) fn model_choices_from(
    entries: &[(Provider, Result<Vec<String>, String>)],
) -> Vec<(Provider, String)> {
    entries
        .iter()
        .filter_map(|(p, r)| r.as_ref().ok().map(|ms| (p, ms)))
        .flat_map(|(p, ms)| ms.iter().map(move |m| (*p, m.clone())))
        .collect()
}

/// Numbered model list for the transcript: `1. glm-4.6 · glm  ← current`.
/// Per-provider failures render as `provider: <error>` lines.
pub(crate) fn format_model_list(
    entries: &[(Provider, Result<Vec<String>, String>)],
    cfg: &AppConfig,
) -> String {
    let mut out = String::from("available models:");
    let mut n = 0;
    for (p, res) in entries {
        match res {
            Ok(models) if models.is_empty() => {
                out.push_str(&format!("\n  {}: endpoint returned no models", provider_name(*p)));
            }
            Ok(models) => {
                for m in models {
                    n += 1;
                    let cur = if *p == cfg.provider && m == &cfg.model {
                        "  ← current"
                    } else {
                        ""
                    };
                    out.push_str(&format!("\n  {n}. {m} · {}{cur}", provider_name(*p)));
                }
            }
            Err(e) => out.push_str(&format!("\n  {}: {}", provider_name(*p), e)),
        }
    }
    if n > 0 {
        out.push_str("\n/model <n> to switch · /model <name> for any id");
    }
    out
}

/// Same-provider switch keeps the session's base_url + key; a provider switch
/// resets to the provider's default endpoint and re-resolves its key from
/// global config/env (fails fast when the key is missing).
fn model_switch(provider: Provider, model: &str, cfg: &AppConfig) -> Result<ModelSwitch> {
    if provider == cfg.provider {
        return Ok(ModelSwitch {
            provider,
            model: model.into(),
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
        });
    }
    let api_key = AppConfig::resolve_runtime_api_key(provider)
        .with_context(|| format!("cannot switch to {}", provider_name(provider)))?;
    Ok(ModelSwitch {
        provider,
        model: model.into(),
        base_url: cfg.endpoint_for(provider),
        api_key,
    })
}
