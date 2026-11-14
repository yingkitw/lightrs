use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::chat::{ApiMessage, ChatRequest, UiMessage, UiRole};
use crate::provider::LlmClient;
use crate::tools::{execute_tool, tool_schemas};

const MAX_TOOL_STEPS: usize = 16;

fn system_prompt(workdir: &Path) -> String {
    format!(
        r#"You are lightrs, a minimal terminal coding agent.
Workdir: {}.

Use tools to read or change the codebase and to run commands — do not invent file contents.
Prefer small, focused edits. Explain briefly after completing the task.
If a tool returns an error, fix the issue or ask the user for missing info."#,
        workdir.display()
    )
}

pub struct Agent {
    client: LlmClient,
    workdir: Arc<Path>,
    api_history: Vec<ApiMessage>,
    ui_history: Vec<UiMessage>,
}

impl Agent {
    pub fn new(client: LlmClient, workdir: Arc<Path>) -> Self {
        let mut api_history = Vec::new();
        api_history.push(ApiMessage::system(system_prompt(&workdir)));
        Self {
            client,
            workdir,
            api_history,
            ui_history: Vec::new(),
        }
    }

    pub fn ui_messages(&self) -> &[UiMessage] {
        &self.ui_history
    }

    pub fn push_system(&mut self, content: impl Into<String>) {
        self.ui_history.push(UiMessage {
            role: UiRole::System,
            content: content.into(),
        });
    }

    pub fn push_error(&mut self, content: impl Into<String>) {
        self.ui_history.push(UiMessage {
            role: UiRole::Error,
            content: content.into(),
        });
    }

    pub fn clear_conversation(&mut self) {
        self.api_history.clear();
        self.api_history
            .push(ApiMessage::system(system_prompt(&self.workdir)));
        self.ui_history.clear();
        self.ui_history.push(UiMessage {
            role: UiRole::System,
            content: "Conversation cleared.".into(),
        });
    }

    pub async fn run_user_turn(&mut self, user_text: String) -> Result<()> {
        self.ui_history.push(UiMessage {
            role: UiRole::User,
            content: user_text.clone(),
        });
        self.api_history.push(ApiMessage::user(user_text));

        for _step in 0..MAX_TOOL_STEPS {
            let request = ChatRequest {
                model: self.client.model().to_string(),
                messages: self.api_history.clone(),
                tools: Some(tool_schemas()),
                tool_choice: Some(serde_json::json!("auto")),
            };

            let response = self.client.chat(request).await?;
            let choice = response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("empty choices"))?;
            let message = choice.message;

            if let Some(calls) = &message.tool_calls {
                if !calls.is_empty() {
                    self.api_history.push(message.clone());
                    for call in calls {
                        let name = call.function.name.clone();
                        let args = call.function.arguments.clone();
                        self.ui_history.push(UiMessage {
                            role: UiRole::Tool,
                            content: format!("→ {name}({})", truncate(&args, 120)),
                        });
                        let result = execute_tool(&self.workdir, &name, &args);
                        self.ui_history.push(UiMessage {
                            role: UiRole::Tool,
                            content: truncate(&result, 8000),
                        });
                        self.api_history.push(ApiMessage::tool_result(
                            call.id.clone(),
                            name,
                            result,
                        ));
                    }
                    continue;
                }
            }

            let text = message
                .text_content()
                .unwrap_or_else(|| "(empty response)".to_string());
            self.api_history.push(message);
            self.ui_history.push(UiMessage {
                role: UiRole::Assistant,
                content: text,
            });
            return Ok(());
        }

        self.ui_history.push(UiMessage {
            role: UiRole::Error,
            content: format!("Stopped after {MAX_TOOL_STEPS} tool steps."),
        });
        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end: String = s.chars().take(max).collect();
    format!("{end}…")
}