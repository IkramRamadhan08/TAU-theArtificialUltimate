use std::sync::Arc;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;
use gpui::{App, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::SharedString;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TodoItem {
    /// Brief description of the task
    pub content: String,
    /// Current status of the task
    pub status: TodoStatus,
    /// Present continuous form shown while in progress (e.g. "Fixing bug")
    #[serde(rename = "activeForm")]
    #[schemars(rename = "activeForm")]
    pub active_form: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TodoWriteToolInput {
    /// The updated todo list for the current session
    pub todos: Vec<TodoItem>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TodoWriteToolOutput {
    Success { todos: Vec<TodoItem> },
    Error { error: String },
}

impl From<TodoWriteToolOutput> for LanguageModelToolResultContent {
    fn from(value: TodoWriteToolOutput) -> Self {
        match value {
            TodoWriteToolOutput::Success { todos } => {
                let mut out = String::from("Updated todo list:\n\n");
                for (i, t) in todos.iter().enumerate() {
                    let mark = match t.status {
                        TodoStatus::Completed => "[x]",
                        TodoStatus::InProgress => "[-]",
                        TodoStatus::Pending => "[ ]",
                    };
                    out.push_str(&format!("{}. {} {}\n", i + 1, mark, t.content));
                    if matches!(t.status, TodoStatus::InProgress) {
                        if let Some(f) = &t.active_form {
                            out.push_str(&format!("   -> {}\n", f));
                        }
                    }
                }
                if todos.is_empty() {
                    out.clear();
                    out.push_str("Todo list cleared.");
                }
                out.into()
            }
            TodoWriteToolOutput::Error { error } => error.into(),
        }
    }
}

pub struct TodoWriteTool;

impl TodoWriteTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TodoWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTool for TodoWriteTool {
    type Input = TodoWriteToolInput;
    type Output = TodoWriteToolOutput;

    const NAME: &'static str = "todo_write";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Thought
    }

    fn initial_title(
        &self,
        _input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        "Planning tasks".into()
    }

    fn description(&self) -> SharedString {
        "Create and manage a structured task list for the current session. Use for multi-step tasks to track progress. Update as you go, mark items in_progress when starting and completed immediately when done.".into()
    }

    fn supports_provider(_provider: &language_model::LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| TodoWriteToolOutput::Error {
                error: e.to_string(),
            })?;
            Ok(TodoWriteToolOutput::Success { todos: input.todos })
        })
    }
}
