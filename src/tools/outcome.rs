use crate::chat::FileDiffUi;

#[derive(Clone, Debug)]
pub struct ToolOutcome {
    pub body: String,
    pub file_diff: Option<FileDiffUi>,
}

impl ToolOutcome {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            file_diff: None,
        }
    }

    pub fn with_diff(body: impl Into<String>, file_diff: FileDiffUi) -> Self {
        Self {
            body: body.into(),
            file_diff: Some(file_diff),
        }
    }
}