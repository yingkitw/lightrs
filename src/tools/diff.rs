use similar::{ChangeTag, TextDiff};

use crate::chat::FileDiffUi;

pub fn build_file_diff(old: Option<&str>, new: &str, path: &str) -> FileDiffUi {
    let prior = old.unwrap_or("");
    let diff = TextDiff::from_lines(prior, new);
    let mut lines_added = 0usize;
    let mut lines_removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => lines_added += 1,
            ChangeTag::Delete => lines_removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let is_new_file = old.is_none();
    let unified = if is_new_file {
        let body: String = new
            .lines()
            .map(|line| format!("+{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("--- /dev/null\n+++ b/{path}\n{body}")
    } else {
        diff.unified_diff()
            .context_radius(3)
            .header(&format!("--- a/{path}"), &format!("+++ b/{path}"))
            .to_string()
    };
    FileDiffUi {
        path: path.to_string(),
        unified,
        lines_added,
        lines_removed,
        is_new_file,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_add_and_remove() {
        let d = build_file_diff(Some("a\nb\n"), "a\nc\n", "x.rs");
        assert_eq!(d.lines_removed, 1);
        assert_eq!(d.lines_added, 1);
        assert!(d.unified.contains("@@"));
    }

    #[test]
    fn new_file_is_all_additions() {
        let d = build_file_diff(None, "hello\n", "new.rs");
        assert!(d.is_new_file);
        assert_eq!(d.lines_added, 1);
        assert!(d.unified.contains("+++ b/new.rs"));
    }
}