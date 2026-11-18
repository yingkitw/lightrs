//! Agent skills: discover `SKILL.md` trees from lightrs + Cursor / Claude / OpenCode / pi / Codex layouts.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config;
use crate::mentions;
use crate::paths::config_dir;
use crate::tools::truncate_bytes;

pub(crate) const SKILL_FILE: &str = "SKILL.md";
const MAX_SKILLS: usize = 64;
const CATALOG_MAX_CHARS: usize = 3_500;
const RULES_MAX_CHARS: usize = 4_096;
const ATTACH_PER_SKILL: usize = 12 * 1024;
const ATTACH_TOTAL_MAX: usize = 48 * 1024;

#[derive(Clone, Debug)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub body: String,
    /// Optional frontmatter keyword (lowercased); matched against user input for a hint.
    pub trigger: Option<String>,
}

#[derive(Clone, Default, Debug)]
pub struct SkillCatalog {
    pub skills: Vec<SkillEntry>,
    pub rules: String,
}

#[derive(Clone, Debug)]
pub struct SkillsSettings {
    pub enabled: bool,
    pub extra_paths: Vec<PathBuf>,
}

impl Default for SkillsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            extra_paths: Vec::new(),
        }
    }
}

pub fn load_settings() -> SkillsSettings {
    let (enabled, extra_paths) = config::skills_settings();
    SkillsSettings {
        enabled,
        extra_paths,
    }
}

pub fn load(settings: &SkillsSettings) -> SkillCatalog {
    if !settings.enabled {
        return SkillCatalog::default();
    }
    // Declared extra_paths come first: they win id collisions and can't be
    // evicted by the MAX_SKILLS ambient cap on machines with many skills.
    let mut roots = settings.extra_paths.clone();
    roots.extend(default_skill_roots());
    let mut by_id: HashMap<String, SkillEntry> = HashMap::new();
    for root in roots {
        if by_id.len() >= MAX_SKILLS {
            break;
        }
        collect_skills_under(&root, &mut by_id);
    }
    let mut skills: Vec<SkillEntry> = by_id.into_values().collect();
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    let rules = load_rules(&config_dir("lightrs").join("rules"));
    SkillCatalog { skills, rules }
}

pub fn default_skill_roots() -> Vec<PathBuf> {
    let home = home_dir();
    let lightrs = config_dir("lightrs");
    vec![
        lightrs.join("skills"),
        home.join(".cursor/skills"),
        home.join(".cursor/skills-cursor"),
        home.join(".claude/skills"),
        home.join(".agents/skills"),
        home.join(".pi/agent/skills"),
        home.join(".codex/skills"),
    ]
}

pub fn system_appendix(catalog: &SkillCatalog) -> String {
    let mut out = String::new();
    if !catalog.skills.is_empty() {
        out.push_str("\n\nSkills — invoke with `$id` (body attaches only on that message):\n");
        let mut used = out.len();
        for s in &catalog.skills {
            let line = format!("- {} — {}\n", s.id, trim_line(&s.description, 160));
            if used + line.len() > CATALOG_MAX_CHARS {
                out.push_str("- … (catalog truncated)\n");
                break;
            }
            out.push_str(&line);
            used += line.len();
        }
    }
    if !catalog.rules.trim().is_empty() {
        out.push_str("\n\nProject rules (summary; use read_file on `rules/` if needed):\n");
        let rules = if catalog.rules.len() > RULES_MAX_CHARS {
            format!("{}\n[rules truncated]", truncate_bytes(&catalog.rules, RULES_MAX_CHARS))
        } else {
            catalog.rules.clone()
        };
        out.push_str(&rules);
    }
    out
}

/// API user text: composer + `@` file attachments + `$skill` bodies (UI transcript unchanged).
pub fn api_user_content(
    workdir: &Path,
    composer: &str,
    catalog: &SkillCatalog,
    read_roots: &[std::path::PathBuf],
) -> String {
    let base = mentions::api_user_content(workdir, composer, read_roots);
    attach_skills(&base, catalog)
}

pub fn collect_skill_invocations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            i += 1;
            let start = i;
            while i < chars.len() {
                let c = chars[i];
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            if i > start {
                let id: String = chars[start..i].iter().collect();
                if seen.insert(id.clone()) {
                    out.push(id);
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

fn attach_skills(composer: &str, catalog: &SkillCatalog) -> String {
    if catalog.skills.is_empty() {
        return composer.to_string();
    }
    let ids = collect_skill_invocations(composer);
    if ids.is_empty() {
        return composer.to_string();
    }
    let map: HashMap<_, _> = catalog.skills.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut extra = String::new();
    let mut used = 0usize;
    for id in ids {
        let Some(skill) = map.get(id.as_str()) else {
            continue;
        };
        if used >= ATTACH_TOTAL_MAX {
            extra.push_str("\n[skill attachments truncated: budget]");
            break;
        }
        let cap = ATTACH_PER_SKILL.min(ATTACH_TOTAL_MAX - used);
        let body = truncate_bytes(&skill.body, cap);
        extra.push_str(&format!("\n--- $skill:{} ({}) ---\n{}", skill.id, skill.name, body));
        used += body.len();
    }
    if extra.is_empty() {
        composer.to_string()
    } else {
        format!("{composer}\n\n[skills]{extra}")
    }
}

fn collect_skills_under(root: &Path, out: &mut HashMap<String, SkillEntry>) {
    if !root.is_dir() {
        return;
    }
    let skill_md = root.join(SKILL_FILE);
    if skill_md.is_file() {
        try_insert_skill(root, &skill_md, out);
        return;
    }
    let Ok(read) = fs::read_dir(root) else {
        return;
    };
    for ent in read.flatten() {
        if out.len() >= MAX_SKILLS {
            break;
        }
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let md = path.join(SKILL_FILE);
        if md.is_file() {
            try_insert_skill(&path, &md, out);
        }
    }
}

fn try_insert_skill(dir: &Path, skill_md: &Path, out: &mut HashMap<String, SkillEntry>) {
    let Some(text) = crate::tools::read_to_string_capped(skill_md, 256 * 1024) else {
        return;
    };
    let (name, description, trigger, body) = parse_skill_md(&text);
    let fallback = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "skill".into());
    let name = if name.is_empty() { fallback.clone() } else { name };
    let id = slug_id(&name);
    if id.is_empty() {
        return;
    }
    let description = if description.is_empty() {
        first_nonempty_line(&body).unwrap_or_else(|| "agent skill".into())
    } else {
        description
    };
    out.entry(id.clone()).or_insert(SkillEntry {
        id,
        name,
        description,
        path: dir.to_path_buf(),
        body,
        trigger,
    });
}

/// `(name, description, trigger, body)` parsed from SKILL.md frontmatter.
pub fn parse_skill_md(text: &str) -> (String, String, Option<String>, String) {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return (
            String::new(),
            String::new(),
            None,
            text.trim().to_string(),
        );
    }
    let rest = trimmed.trim_start_matches("---").trim_start();
    let Some(end) = rest.find("\n---") else {
        return (String::new(), String::new(), None, text.trim().to_string());
    };
    let front = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('-').trim_start();
    let mut name = String::new();
    let mut description = String::new();
    let mut trigger = None;
    for line in front.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("trigger:") {
            let t = v.trim().trim_matches('"').to_ascii_lowercase();
            if !t.is_empty() {
                trigger = Some(t);
            }
        }
    }
    (name, description, trigger, body.to_string())
}

fn load_rules(rules_dir: &Path) -> String {
    if !rules_dir.is_dir() {
        return String::new();
    }
    let Ok(read) = fs::read_dir(rules_dir) else {
        return String::new();
    };
    let mut files: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().is_some_and(|e| e == "md")
                && p.file_name().is_some_and(|n| n != "INDEX.md")
        })
        .collect();
    files.sort();
    let mut out = String::new();
    for path in files {
        if out.len() >= RULES_MAX_CHARS {
            break;
        }
        let Some(text) = crate::tools::read_to_string_capped(&path, 256 * 1024) else {
            continue;
        };
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let chunk = format!("\n### {name}\n{text}\n");
        let room = RULES_MAX_CHARS.saturating_sub(out.len());
        if chunk.len() > room {
            out.push_str(truncate_bytes(&chunk, room));
            out.push_str("\n[rules truncated]");
            break;
        }
        out.push_str(&chunk);
    }
    out.trim().to_string()
}

pub(crate) fn slug_id(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if (c.is_whitespace() || c == '/')
            && !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.to_ascii_lowercase()
}

fn trim_line(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        format!(
            "{}…",
            one_line.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

fn first_nonempty_line(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(ToString::to_string)
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Active `$skill-id` token at end of input (no spaces).
pub fn active_skill_token(input: &str) -> Option<(&str, usize)> {
    let dollar = input.rfind('$')?;
    let tail = &input[dollar + 1..];
    if tail.contains(char::is_whitespace) {
        return None;
    }
    Some((tail, dollar))
}

pub fn skill_hint(input: &str, catalog: &SkillCatalog) -> Option<String> {
    let (prefix, _) = active_skill_token(input)?;
    let mut matches: Vec<&SkillEntry> = catalog
        .skills
        .iter()
        .filter(|s| prefix.is_empty() || s.id.starts_with(prefix))
        .collect();
    if matches.is_empty() {
        return Some("$skill — no match (try /skills)".into());
    }
    matches.sort_by_key(|s| &s.id);
    let show = matches.len().min(3);
    let parts: Vec<String> = matches
        .iter()
        .take(show)
        .map(|s| format!("${}", s.id))
        .collect();
    if matches.len() <= show {
        Some(parts.join(" · "))
    } else {
        Some(format!("{} · …", parts.join(" · ")))
    }
}

/// Non-blocking hint when the user's input matches a skill's `trigger:` keyword
/// (and the skill wasn't already invoked via `$id`).
pub fn trigger_hint(input: &str, catalog: &SkillCatalog) -> Option<String> {
    let lower = input.to_lowercase();
    let invoked = collect_skill_invocations(input);
    catalog
        .skills
        .iter()
        .filter(|s| {
            s.trigger
                .as_deref()
                .is_some_and(|t| lower.contains(t))
                && !invoked.iter().any(|i| i == &s.id)
        })
        .map(|s| format!("skill `${}` may apply — resend with `${}` to attach it", s.id, s.id))
        .next()
}

pub fn tab_complete_skill(input: &str, catalog: &SkillCatalog) -> Option<String> {
    let (prefix, at) = active_skill_token(input)?;
    let mut matches: Vec<&str> = catalog
        .skills
        .iter()
        .filter(|s| prefix.is_empty() || s.id.starts_with(prefix))
        .map(|s| s.id.as_str())
        .collect();
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    if matches.len() == 1 {
        let head = &input[..at];
        return Some(format!("{head}${}", matches[0]));
    }
    if !prefix.is_empty() {
        let mut common = matches[0].to_string();
        for name in matches.iter().skip(1) {
            common = common_prefix(&common, name);
        }
        if common.len() > prefix.len() {
            let head = &input[..at];
            return Some(format!("{head}${common}"));
        }
    }
    None
}

fn common_prefix(a: &str, b: &str) -> String {
    a.chars()
        .zip(b.chars())
        .take_while(|(x, y)| x == y)
        .map(|(c, _)| c)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_skill_frontmatter() {
        let md = "---\nname: lean-implementation\ndescription: stay small\ntrigger: simplify\n---\n\n# Body\n";
        let (name, desc, trigger, body) = parse_skill_md(md);
        assert_eq!(name, "lean-implementation");
        assert_eq!(desc, "stay small");
        assert_eq!(trigger.as_deref(), Some("simplify"));
        assert!(body.contains("# Body"));
    }

    #[test]
    fn trigger_hint_matches_keyword_not_invoked() {
        let catalog = SkillCatalog {
            skills: vec![SkillEntry {
                id: "lean".into(),
                name: "Lean".into(),
                description: String::new(),
                path: PathBuf::from("."),
                body: String::new(),
                trigger: Some("simplify".into()),
            }],
            rules: String::new(),
        };
        assert!(trigger_hint("please simplify this", &catalog).is_some());
        assert!(trigger_hint("use $lean please", &catalog).is_none());
        assert!(trigger_hint("unrelated input", &catalog).is_none());
    }

    #[test]
    fn discovers_skill_in_temp_dir() {
        let root = std::env::temp_dir().join(format!("lightrs-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let skill_dir = root.join("my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: test skill\n---\n\ndo thing\n",
        )
        .unwrap();
        let mut map = HashMap::new();
        collect_skills_under(&root, &mut map);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("my-skill"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn attaches_dollar_skill() {
        let catalog = SkillCatalog {
            skills: vec![SkillEntry {
                id: "foo".into(),
                name: "Foo".into(),
                description: String::new(),
                path: PathBuf::from("."),
                body: "skill body".into(),
                trigger: None,
            }],
            rules: String::new(),
        };
        let api = api_user_content(Path::new("."), "run $foo", &catalog, &[]);
        assert!(api.contains("skill body"));
        assert!(api.contains("$skill:foo"));
    }

    #[test]
    fn collects_skill_tokens() {
        assert_eq!(
            collect_skill_invocations("a $foo and $bar-baz"),
            vec!["foo".to_string(), "bar-baz".to_string()]
        );
        assert_eq!(collect_skill_invocations("$a $a"), vec!["a".to_string()]);
        assert!(collect_skill_invocations("ends with $").is_empty());
        assert_eq!(collect_skill_invocations("$$a"), vec!["a".to_string()]);
    }

    fn entry(id: &str) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            name: id.to_uppercase(),
            description: format!("{id} desc"),
            path: PathBuf::from("."),
            body: format!("{id} body"),
            trigger: None,
        }
    }

    fn catalog(ids: &[&str]) -> SkillCatalog {
        SkillCatalog {
            skills: ids.iter().map(|i| entry(i)).collect(),
            rules: String::new(),
        }
    }

    #[test]
    fn load_disabled_returns_empty_catalog() {
        let cat = load(&SkillsSettings {
            enabled: false,
            extra_paths: Vec::new(),
        });
        assert!(cat.skills.is_empty());
        assert!(cat.rules.is_empty());
    }

    #[test]
    fn load_picks_up_extra_paths() {
        let root = std::env::temp_dir().join(format!("lightrs-skill-xp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let skill_dir = root.join("xtra");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: xtra\ndescription: from extra\n---\n\nbody\n",
        )
        .unwrap();
        let cat = load(&SkillsSettings {
            enabled: true,
            extra_paths: vec![root.clone()],
        });
        let _ = fs::remove_dir_all(&root);
        assert!(cat.skills.iter().any(|s| s.id == "xtra"));
    }

    #[test]
    fn system_appendix_lists_skills_and_rules() {
        let mut cat = catalog(&["alpha"]);
        cat.rules = "do the thing".into();
        let out = system_appendix(&cat);
        assert!(out.contains("$id"));
        assert!(out.contains("- alpha — alpha desc"));
        assert!(out.contains("Project rules"));
        assert!(out.contains("do the thing"));
    }

    #[test]
    fn system_appendix_truncates_oversized_rules() {
        let mut cat = catalog(&[]);
        cat.rules = "x".repeat(RULES_MAX_CHARS + 100);
        assert!(system_appendix(&cat).contains("[rules truncated]"));
    }

    #[test]
    fn skill_hint_lists_matches_then_ellipsis() {
        let cat = catalog(&["aa", "ab", "ac", "ad"]);
        let hint = skill_hint("use $a", &cat).unwrap();
        assert!(hint.contains("$aa"), "hint = {hint}");
        assert!(hint.contains('…'), "hint = {hint}");
        let exact = skill_hint("$aa", &cat).unwrap();
        assert_eq!(exact, "$aa");
    }

    #[test]
    fn skill_hint_no_match_is_named() {
        let cat = catalog(&["aa"]);
        let hint = skill_hint("$zz", &cat).unwrap();
        assert!(hint.contains("no match"), "hint = {hint}");
        assert!(skill_hint("no dollar", &cat).is_none());
    }

    #[test]
    fn active_skill_token_only_at_tail() {
        assert_eq!(active_skill_token("use $fo"), Some(("fo", 4)));
        assert_eq!(active_skill_token("$fo and $"), Some(("", 8)));
        assert!(active_skill_token("a $x y").is_none());
        assert!(active_skill_token("plain").is_none());
    }

    #[test]
    fn tab_complete_skill_unique_and_common() {
        let cat = catalog(&["alpha", "alpine", "beta"]);
        assert_eq!(
            tab_complete_skill("run $bet", &cat).as_deref(),
            Some("run $beta")
        );
        // $alp already equals the common prefix of alpha/alpine — nothing to add.
        assert!(tab_complete_skill("run $alp", &cat).is_none());
        assert!(tab_complete_skill("$z", &cat).is_none());
        assert!(tab_complete_skill("plain", &cat).is_none());
    }

    #[test]
    fn attach_unknown_skill_leaves_composer_unchanged() {
        let cat = catalog(&["foo"]);
        assert_eq!(api_user_content(Path::new("."), "use $zzz", &cat, &[]), "use $zzz");
    }

    #[test]
    fn slug_id_normalizes_names() {
        assert_eq!(slug_id("My Cool/Skill"), "my-cool-skill");
        // Trailing dashes trimmed; leading dashes pass through literally.
        assert_eq!(slug_id("  --x--  "), "--x");
        assert_eq!(slug_id("keep_underscore-ok"), "keep_underscore-ok");
        assert!(slug_id("!!!").is_empty());
    }

    #[test]
    fn truncate_bytes_stops_at_char_boundary() {
        assert_eq!(truncate_bytes("héllo", 2), "h");
        assert_eq!(truncate_bytes("héllo", 3), "hé");
        assert_eq!(truncate_bytes("abc", 9), "abc");
    }

    #[test]
    fn appendix_rules_cap_stops_at_char_boundary() {
        // 'é' straddles byte RULES_MAX_CHARS — a raw byte slice would panic.
        let catalog = SkillCatalog {
            skills: Vec::new(),
            rules: format!("{}éé", "x".repeat(RULES_MAX_CHARS - 1)),
        };
        let out = system_appendix(&catalog);
        assert!(out.contains("[rules truncated]"), "out: {out}");
    }

    #[test]
    fn load_rules_chunk_cap_stops_at_char_boundary() {
        let dir = std::env::temp_dir().join(format!("lightrs-rules-cap-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // chunk = "\n### r.md\n" + text + "\n": pad so 'é' straddles the cap.
        let fill = RULES_MAX_CHARS - "\n### r.md\n".len() - 1;
        fs::write(dir.join("r.md"), format!("{}éé", "x".repeat(fill))).unwrap();
        let out = load_rules(&dir);
        let _ = fs::remove_dir_all(&dir);
        assert!(out.contains("[rules truncated]"), "out tail: {}", &out[out.len().saturating_sub(60)..]);
    }

    #[test]
    fn load_rules_sorts_and_skips_index() {
        let dir = std::env::temp_dir().join(format!("lightrs-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.md"), "bee").unwrap();
        fs::write(dir.join("a.md"), "aye").unwrap();
        fs::write(dir.join("INDEX.md"), "skip me").unwrap();
        fs::write(dir.join("note.txt"), "skip me too").unwrap();
        let out = load_rules(&dir);
        let _ = fs::remove_dir_all(&dir);
        let a = out.find("### a.md").unwrap();
        let b = out.find("### b.md").unwrap();
        assert!(a < b, "sorted order: {out}");
        assert!(!out.contains("skip me"), "out = {out}");
        assert!(load_rules(&dir.join("absent")).is_empty());
    }

    #[test]
    fn parse_skill_md_without_frontmatter_returns_body() {
        let (_n, _d, _t, body) = parse_skill_md("just text\nmore");
        assert_eq!(body, "just text\nmore");
        // Unterminated frontmatter — whole text is body, no metadata.
        let (n, d, t, body) = parse_skill_md("---\nname: x\nno end marker");
        assert!(n.is_empty() && d.is_empty() && t.is_none());
        assert_eq!(body, "---\nname: x\nno end marker");
    }

    #[test]
    fn first_nonempty_line_skips_blanks_and_headings() {
        assert_eq!(
            first_nonempty_line("\n# heading\n  real line \n").as_deref(),
            Some("real line")
        );
        assert!(first_nonempty_line("\n\n# only\n").is_none());
    }
}