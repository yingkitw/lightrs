//! Flexible search/replace for `patch_file` (single occurrence).
//!
//! Tries exact match first, then progressively more tolerant strategies when
//! whitespace or indentation drifts (Aider-style).

/// Apply `search` → `replace` once in `content`, trying flexible strategies.
pub fn replace_once(content: &str, search: &str, replace: &str) -> Option<String> {
    if search.is_empty() {
        return None;
    }

    if let Some(start) = find_exact(content, search) {
        return Some(patch(content, start, search.len(), replace));
    }
    if let Some(start) = find_line_trimmed(content, search) {
        let len = matched_line_trimmed_len(content, search, start);
        return Some(patch(content, start, len, replace));
    }
    if let Some(start) = find_strip_blank_edges(content, search) {
        let trimmed = trim_outer_newlines(search);
        return Some(patch(content, start, trimmed.len(), replace));
    }

    replace_indent_flexible(content, search, replace)
}

fn patch(content: &str, start: usize, len: usize, replace: &str) -> String {
    let mut out = String::with_capacity(content.len() + replace.len());
    out.push_str(&content[..start]);
    out.push_str(replace);
    out.push_str(&content[start + len..]);
    out
}

fn replace_indent_flexible(content: &str, search: &str, replace: &str) -> Option<String> {
    let search_lines: Vec<&str> = search.lines().collect();
    if search_lines.is_empty() {
        return None;
    }
    let content_lines: Vec<&str> = content.lines().collect();
    if content_lines.len() < search_lines.len() {
        return None;
    }

    let search_trimmed: Vec<&str> = search_lines.iter().map(|l| l.trim()).collect();
    let search_indent = leading_spaces(search_lines[0]);

    for start in 0..=content_lines.len() - search_lines.len() {
        let window = &content_lines[start..start + search_lines.len()];
        if !window
            .iter()
            .zip(search_trimmed.iter())
            .all(|(a, b)| a.trim() == *b)
        {
            continue;
        }
        let content_indent = leading_spaces(content_lines[start]);
        let replace_lines: Vec<&str> = replace.lines().collect();
        let mut out: Vec<String> = content_lines[..start]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        for (i, rline) in replace_lines.iter().enumerate() {
            let s_indent = search_lines
                .get(i)
                .map(|line| leading_spaces(line))
                .unwrap_or(search_indent);
            let delta = s_indent as i32 - search_indent as i32;
            let indent = (content_indent as i32 + delta).max(0) as usize;
            out.push(format!("{}{}", " ".repeat(indent), rline.trim_start()));
        }
        out.extend(
            content_lines[start + search_lines.len()..]
                .iter()
                .map(|s| (*s).to_string()),
        );
        let mut result = out.join("\n");
        if content.ends_with('\n') {
            result.push('\n');
        }
        return Some(result);
    }
    None
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn matched_line_trimmed_len(content: &str, search: &str, start: usize) -> usize {
    let search_lines: Vec<&str> = search.lines().filter(|l| !l.is_empty()).collect();
    let content_tail = &content[start..];
    let mut consumed = 0usize;
    for (i, sline) in search_lines.iter().enumerate() {
        if i > 0
            && let Some(p) = content_tail[consumed..].find('\n')
        {
            consumed += p + 1;
        }
        let cline = content_tail[consumed..].lines().next().unwrap_or("");
        if cline.trim_end() != sline.trim_end() {
            return search.len();
        }
        consumed += cline.len();
        if i < search_lines.len() - 1 {
            consumed += 1;
        }
    }
    consumed
}

fn find_exact(haystack: &str, needle: &str) -> Option<usize> {
    haystack.find(needle)
}

fn trim_outer_newlines(s: &str) -> &str {
    let s = s.strip_prefix('\n').unwrap_or(s);
    s.strip_suffix('\n').unwrap_or(s)
}

fn find_strip_blank_edges(haystack: &str, needle: &str) -> Option<usize> {
    if !needle.starts_with('\n') && !needle.ends_with('\n') {
        return None;
    }
    let trimmed = trim_outer_newlines(needle);
    if trimmed.is_empty() || trimmed == needle {
        return None;
    }
    haystack.find(trimmed)
}

fn find_line_trimmed(haystack: &str, needle: &str) -> Option<usize> {
    let search_lines: Vec<&str> = needle.lines().filter(|l| !l.is_empty()).collect();
    if search_lines.len() <= 1 {
        return None;
    }

    let hay_lines: Vec<(usize, &str)> = haystack
        .lines()
        .scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .collect();

    'outer: for window in 0..=hay_lines.len().saturating_sub(search_lines.len()) {
        for (i, sline) in search_lines.iter().enumerate() {
            let (_, hline) = hay_lines[window + i];
            if hline.trim_end() != sline.trim_end() {
                continue 'outer;
            }
        }
        return Some(hay_lines[window].0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_unchanged() {
        let content = "line1\nline2\nline3\n";
        let got = replace_once(content, "line2\n", "LINE2\n").unwrap();
        assert_eq!(got, "line1\nLINE2\nline3\n");
    }

    #[test]
    fn line_trimmed_ignores_trailing_spaces() {
        let content = "fn foo() {\n    bar();  \n}\n";
        let search = "    bar();\n";
        let replace = "    baz();\n";
        let got = replace_once(content, search, replace).unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("bar();"));
    }

    #[test]
    fn relative_indent_tolerates_different_base_indent() {
        let content = "    fn foo() {\n        bar();\n    }\n";
        let search = "fn foo() {\n    bar();\n}\n";
        let replace = "fn foo() {\n    baz();\n}\n";
        let got = replace_once(content, search, replace).unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("bar();"));
        assert!(got.starts_with("    fn foo()"));
    }
}