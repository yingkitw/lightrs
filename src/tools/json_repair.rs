//! Best-effort repair for malformed tool-call argument JSON from models.

/// Parse JSON after light repair (fences, substring, trailing commas).
pub fn parse_tool_json<T: for<'de> serde::Deserialize<'de>>(arguments: &str) -> anyhow::Result<T> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return serde_json::from_str("{}").map_err(Into::into);
    }
    if let Ok(v) = serde_json::from_str::<T>(trimmed) {
        return Ok(v);
    }
    let repaired = repair_json_text(trimmed);
    serde_json::from_str(&repaired).map_err(Into::into)
}

fn repair_json_text(s: &str) -> String {
    let mut t = strip_code_fence(s).trim().to_string();
    if let Some(slice) = extract_json_object(&t) {
        t = slice.to_string();
    }
    remove_trailing_commas(&mut t);
    t
}

fn strip_code_fence(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with("```") {
        return t.to_string();
    }
    let rest = t.strip_prefix("```").unwrap_or(t).trim_start();
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest);
    let rest = rest.trim_start();
    if let Some(i) = rest.rfind("```") {
        rest[..i].trim().to_string()
    } else {
        rest.to_string()
    }
}

fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

fn remove_trailing_commas(s: &mut String) {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    *s = out;
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Mini {
        path: String,
    }

    use super::*;

    #[test]
    fn accepts_trailing_comma() {
        let v: Mini = parse_tool_json(r#"{"path":"a.rs",}"#).unwrap();
        assert_eq!(v.path, "a.rs");
    }

    #[test]
    fn accepts_fenced_json() {
        let v: Mini = parse_tool_json("```json\n{\"path\":\"b.rs\"}\n```").unwrap();
        assert_eq!(v.path, "b.rs");
    }

    #[test]
    fn extracts_embedded_object() {
        let v: Mini = parse_tool_json("here is json {\"path\":\"c.rs\"} thanks").unwrap();
        assert_eq!(v.path, "c.rs");
    }
}