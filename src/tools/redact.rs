//! Redact likely secrets in tool output before it is sent to the model.

const REDACTED: &str = "[REDACTED]";

/// Scrub common secret shapes from tool result text (API history path).
pub fn scrub_tool_output(text: &str) -> String {
    redact_secrets(text)
}

pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&redact_line(line));
    }
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn redact_line(line: &str) -> String {
    let mut s = line.to_string();
    s = redact_tokens(&s, "sk-", 8);
    s = redact_tokens(&s, "sk_live_", 8);
    s = redact_tokens(&s, "sk_test_", 8);
    s = redact_tokens(&s, "xoxb-", 8);
    s = redact_tokens(&s, "ghp_", 8);
    s = redact_tokens(&s, "gho_", 8);
    s = redact_tokens(&s, "github_pat_", 8);
    if let Some(i) = s.to_ascii_lowercase().find("bearer ") {
        if let Some(rest) = s.get(i + 7..) {
            let token = rest.split_whitespace().next().unwrap_or("");
            if token.len() >= 12 {
                s.replace_range(i..i + 7 + token.len(), &format!("Bearer {REDACTED}"));
            }
        }
    }
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY-----") {
        return REDACTED.into();
    }
    if looks_like_env_secret(line) {
        if let Some((k, _v)) = line.split_once('=') {
            return format!("{k}={REDACTED}");
        }
    }
    s
}

fn redact_tokens(s: &str, prefix: &str, min_body: usize) -> String {
    let mut out = s.to_string();
    let mut search_from = 0;
    while let Some(i) = out[search_from..].find(prefix) {
        let start = search_from + i;
        let rest = &out[start + prefix.len()..];
        let len = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if len >= min_body {
            let end = start + prefix.len() + len;
            out.replace_range(start..end, REDACTED);
            search_from = start + REDACTED.len();
        } else {
            search_from = start + prefix.len();
        }
    }
    out
}

fn looks_like_env_secret(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('=') {
        return false;
    }
    let key = trimmed.split('=').next().unwrap_or("").to_ascii_uppercase();
    key.contains("API_KEY")
        || key.contains("SECRET")
        || key.contains("TOKEN")
        || key.contains("PASSWORD")
        || key.contains("PRIVATE_KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_sk_key() {
        let line = "key=sk-abcdefghijklmnopqrstuvwxyz123456";
        assert!(redact_line(line).contains(REDACTED));
        assert!(!redact_line(line).contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn leaves_normal_lines() {
        assert_eq!(redact_line("hello world"), "hello world");
    }
}