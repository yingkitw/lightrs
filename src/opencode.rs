use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::config::{Provider, config_path};
use crate::migrate::{self, ModelProfile, config_lines, write_config};

pub fn import(global: Option<PathBuf>, project: Option<PathBuf>) -> Result<()> {
    let mut docs = Vec::new();
    if let Some(path) = global {
        docs.push(load_jsonc(&path)?);
    } else if let Some(path) = default_opencode_global_config() {
        if path.exists() {
            docs.push(load_jsonc(&path)?);
        }
    }

    if let Some(path) = project {
        docs.push(load_jsonc(&path)?);
    } else if let Some(path) = discover_project_opencode()? {
        docs.push(load_jsonc(&path)?);
    }

    if docs.is_empty() {
        bail!("no OpenCode config found; pass --config or run from a project with opencode.json");
    }

    let merged = merge_json_docs(&docs);
    let lightrs_dir = config_path()
        .parent()
        .map(Path::to_path_buf)
        .context("lightrs config directory")?;

    fs::create_dir_all(&lightrs_dir)?;

    import_model_settings(&merged, &lightrs_dir)?;
    import_mcp(&merged, &lightrs_dir)?;
    import_skills(&merged, &lightrs_dir)?;
    import_rules(&merged, &lightrs_dir)?;
    import_hooks(&merged, &lightrs_dir)?;

    println!("Imported OpenCode settings into {}", lightrs_dir.display());
    Ok(())
}

pub fn default_opencode_global_config() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(d) = directories::ProjectDirs::from("", "", "opencode") {
        candidates.push(d.config_dir().join("opencode.json"));
        candidates.push(d.config_dir().join("opencode.jsonc"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".config/opencode/opencode.json"));
        candidates.push(home.join(".config/opencode/opencode.jsonc"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn discover_project_opencode() -> Result<Option<PathBuf>> {
    let mut dir = std::env::current_dir().context("current directory")?;
    loop {
        for candidate in [
            dir.join("opencode.json"),
            dir.join("opencode.jsonc"),
            dir.join(".opencode/opencode.json"),
            dir.join(".opencode/opencode.jsonc"),
        ] {
            if candidate.is_file() {
                return Ok(Some(candidate));
            }
        }
        if !dir.pop() {
            break;
        }
    }
    Ok(None)
}

fn merge_json_docs(docs: &[Value]) -> Value {
    let mut out = Value::Object(Default::default());
    for doc in docs {
        merge_into(&mut out, doc);
    }
    out
}

fn merge_into(base: &mut Value, overlay: &Value) {
    match overlay {
        Value::Object(b) if base.is_object() => {
            let a = base.as_object_mut().expect("object");
            for (k, v) in b {
                if let Some(existing) = a.get_mut(k) {
                    merge_into(existing, v);
                } else {
                    a.insert(k.clone(), v.clone());
                }
            }
        }
        _ => *base = overlay.clone(),
    }
}

pub fn extract_model_profile(doc: &Value) -> ModelProfile {
    let mut profile = ModelProfile::default();

    if let Some(model_ref) = doc.get("model").and_then(Value::as_str) {
        if let Some((p, m)) = model_ref.split_once('/') {
            profile.provider = migrate::map_opencode_provider_id(p);
            profile.model = Some(m.to_string());
        }
    }

    let provider_map = doc
        .get("providers")
        .or_else(|| doc.get("provider"))
        .and_then(Value::as_object);

    if let Some(map) = provider_map {
        for (id, cfg) in map {
            if profile.provider.is_none() {
                profile.provider = migrate::map_opencode_provider_id(id);
            }
            if let Some(options) = cfg.get("options").and_then(Value::as_object) {
                if let Some(key) = options
                    .get("apiKey")
                    .or_else(|| options.get("api_key"))
                    .and_then(Value::as_str)
                {
                    if let Some(p) = migrate::map_opencode_provider_id(id) {
                        match p {
                            Provider::Glm => profile.keys.glm = Some(key.to_string()),
                            Provider::Kimi => profile.keys.kimi = Some(key.to_string()),
                            Provider::Minimax => profile.keys.minimax = Some(key.to_string()),
                        }
                    }
                }
            }
            if let Some(settings) = cfg.get("settings").and_then(Value::as_object) {
                if let Some(url) = settings
                    .get("baseURL")
                    .or_else(|| settings.get("base_url"))
                    .and_then(Value::as_str)
                {
                    profile.base_url = Some(url.trim_end_matches('/').to_string());
                }
            }
        }
    }

    profile
}

fn import_model_settings(doc: &Value, dest: &Path) -> Result<()> {
    let profile = extract_model_profile(doc);
    let path = dest.join("config.toml");
    if path.exists() {
        let alt = dest.join("opencode-import.toml");
        eprintln!("note: leaving existing {}; wrote {}", path.display(), alt.display());
        fs::write(
            alt,
            config_lines(&profile, "# Generated by `lightrs import opencode`").join("\n"),
        )?;
    } else {
        write_config(&profile, dest)?;
    }
    Ok(())
}

fn import_mcp(doc: &Value, dest: &Path) -> Result<()> {
    if let Some(mcp) = doc.get("mcp") {
        let path = dest.join("mcp.json");
        fs::write(&path, serde_json::to_string_pretty(mcp)?).with_context(|| path.display().to_string())?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn import_skills(doc: &Value, dest: &Path) -> Result<()> {
    let skills_root = dest.join("skills");
    fs::create_dir_all(&skills_root)?;
    let mut linked = 0usize;

    let paths = doc
        .get("skills")
        .and_then(|s| s.get("paths"))
        .and_then(Value::as_array);

    if let Some(paths) = paths {
        for entry in paths {
            let Some(raw) = entry.as_str() else { continue };
            let expanded = shellexpand_tilde(raw);
            let src = PathBuf::from(expanded);
            if !src.exists() {
                eprintln!("skip missing skill path: {}", src.display());
                continue;
            }
            let name = src
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "skill".into());
            let link = skills_root.join(name);
            symlink_or_copy_dir(&src, &link)?;
            linked += 1;
        }
    }

    let default_agents = PathBuf::from(format!(
        "{}/.agents/skills",
        std::env::var("HOME").unwrap_or_else(|_| ".".into())
    ));
    if default_agents.is_dir() && linked == 0 {
        symlink_or_copy_dir(&default_agents, &skills_root.join("opencode-agents"))?;
        linked += 1;
    }

    if linked > 0 {
        println!("linked {linked} skill source(s) under {}", skills_root.display());
    }
    Ok(())
}

fn import_rules(doc: &Value, dest: &Path) -> Result<()> {
    let rules_dir = dest.join("rules");
    fs::create_dir_all(&rules_dir)?;

    if let Some(instructions) = doc.get("instructions").and_then(Value::as_array) {
        let mut out = String::from("# Imported from OpenCode instructions\n\n");
        for item in instructions {
            if let Some(path) = item.as_str() {
                let expanded = shellexpand_tilde(path);
                let p = PathBuf::from(expanded);
                if p.is_file() {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "rule.md".into());
                    fs::copy(&p, rules_dir.join(&name))?;
                    out.push_str(&format!("- `{path}` → rules/{name}\n"));
                } else {
                    out.push_str(&format!("- (missing) `{path}`\n"));
                }
            }
        }
        fs::write(rules_dir.join("INDEX.md"), out)?;
        println!("imported rules into {}", rules_dir.display());
    }
    Ok(())
}

fn import_hooks(doc: &Value, dest: &Path) -> Result<()> {
    let hooks_dir = dest.join("hooks");
    fs::create_dir_all(&hooks_dir)?;

    let mut found = false;
    if let Some(plugin) = doc.get("plugin") {
        fs::write(hooks_dir.join("plugins.json"), serde_json::to_string_pretty(plugin)?)?;
        found = true;
    }

    for candidate in [
        default_opencode_global_config()
            .map(|p| p.with_file_name("hooks.json"))
            .unwrap_or_default(),
        PathBuf::from(".opencode/hooks.json"),
    ] {
        if candidate.is_file() {
            fs::copy(&candidate, hooks_dir.join(candidate.file_name().unwrap_or_default()))?;
            found = true;
        }
    }

    if found {
        println!("copied hooks/plugins metadata into {}", hooks_dir.display());
    }
    Ok(())
}

pub fn load_jsonc(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let stripped = strip_jsonc_comments(&text);
    serde_json::from_str(&stripped).with_context(|| format!("parse {}", path.display()))
}

fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }
        if ch == '/' && matches!(chars.peek(), Some('/')) {
            while matches!(chars.next(), Some(c) if c != '\n') {}
            continue;
        }
        out.push(ch);
    }
    out
}

fn shellexpand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

#[cfg(unix)]
fn symlink_or_copy_dir(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    std::os::unix::fs::symlink(src, dest).with_context(|| {
        format!("symlink {} -> {}", dest.display(), src.display())
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn symlink_or_copy_dir(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    copy_dir_recursive(src, dest)?;
    Ok(())
}

#[cfg(not(unix))]
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}