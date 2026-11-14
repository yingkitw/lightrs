//! Multi-line composer helpers (Codex-style: history recall, draft lines).

const MAX_HISTORY: usize = 100;

#[derive(Debug, Default)]
pub struct Composer {
    history: Vec<String>,
    /// Index when browsing history with Ctrl+P/N.
    browse: Option<usize>,
    draft: String,
}

impl Composer {
    pub fn commit(&mut self, line: String) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.history.last().is_some_and(|s| s == trimmed) {
            self.browse = None;
            self.draft.clear();
            return;
        }
        self.history.push(trimmed.to_string());
        if self.history.len() > MAX_HISTORY {
            self.history.remove(0);
        }
        self.browse = None;
        self.draft.clear();
    }

    /// Ctrl+P — older submitted line.
    pub fn recall_older(&mut self, current: &str) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        if self.browse.is_none() {
            self.draft = current.to_string();
        }
        let idx = self.browse.unwrap_or(self.history.len()) - 1;
        self.browse = Some(idx);
        Some(self.history[idx].clone())
    }

    /// Ctrl+N — newer submitted line (toward draft).
    pub fn recall_newer(&mut self) -> Option<String> {
        let idx = self.browse?;
        if idx + 1 >= self.history.len() {
            self.browse = None;
            let draft = std::mem::take(&mut self.draft);
            return if draft.is_empty() { None } else { Some(draft) };
        }
        let next = idx + 1;
        self.browse = Some(next);
        Some(self.history[next].clone())
    }

    pub fn cancel_browse(&mut self) {
        self.browse = None;
        self.draft.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_recall_round_trip() {
        let mut c = Composer::default();
        c.commit("first".into());
        c.commit("second".into());
        assert_eq!(c.recall_older(""), Some("second".to_string()));
        assert_eq!(c.recall_older("second"), Some("first".to_string()));
        assert_eq!(c.recall_newer(), Some("second".to_string()));
        assert_eq!(c.recall_newer(), None);
    }
}