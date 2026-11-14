//! Minimal glob matching for `glob_file_search` (`*`, `?`, `**`; no brace expansion).

/// Returns `true` when `pattern` matches the slash-separated path `text` (forward slashes).
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    let pat: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = text.split('/').filter(|s| !s.is_empty()).collect();
    glob_segments(&pat, &path)
}

fn glob_segments(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }
    let head = pattern[0];
    let rest = &pattern[1..];
    if head == "**" {
        if rest.is_empty() {
            return true;
        }
        for i in 0..=path.len() {
            if glob_segments(rest, &path[i..]) {
                return true;
            }
        }
        return false;
    }
    if path.is_empty() {
        return false;
    }
    if !glob_component(head, path[0]) {
        return false;
    }
    glob_segments(rest, &path[1..])
}

fn glob_component(pattern: &str, text: &str) -> bool {
    let bytes: Vec<u8> = pattern.bytes().collect();
    match_component(&bytes, text.as_bytes())
}

fn match_component(pattern: &[u8], text: &[u8]) -> bool {
    match_component_at(pattern, 0, text, 0)
}

fn match_component_at(pattern: &[u8], pi: usize, text: &[u8], ti: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }
    match pattern[pi] {
        b'*' => {
            for tj in ti..=text.len() {
                if match_component_at(pattern, pi + 1, text, tj) {
                    return true;
                }
            }
            false
        }
        b'?' => {
            if ti >= text.len() {
                false
            } else {
                match_component_at(pattern, pi + 1, text, ti + 1)
            }
        }
        c => {
            if ti >= text.len() || text[ti] != c {
                false
            } else {
                match_component_at(pattern, pi + 1, text, ti + 1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_single_segment() {
        assert!(glob_matches("*.rs", "lib.rs"));
        assert!(!glob_matches("*.rs", "src/lib.rs"));
    }

    #[test]
    fn double_star() {
        assert!(glob_matches("**/*.rs", "src/lib.rs"));
        assert!(glob_matches("**/*.rs", "lib.rs"));
        assert!(!glob_matches("**/*.rs", "README.md"));
    }
}