use collections::IndexMap;
use language_model::{
    LanguageModelRequestMessage, LanguageModelToolResult, LanguageModelToolResultContent,
    LanguageModelToolUse, LanguageModelToolUseId, Role,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::fmt::Write;
use util::markdown::MarkdownCodeBlock;
impl AgentMessage {
    pub fn to_markdown(&self) -> String {
        let mut markdown = String::new();

        for content in &self.content {
            match content {
                AgentMessageContent::Text(text) => {
                    markdown.push_str(text);
                    markdown.push('\n');
                }
                AgentMessageContent::Thinking { text, .. } => {
                    markdown.push_str("<think>");
                    markdown.push_str(text);
                    markdown.push_str("</think>\n");
                }
                AgentMessageContent::RedactedThinking(_) => {
                    markdown.push_str("<redacted_thinking />\n")
                }
                AgentMessageContent::ToolUse(tool_use) => {
                    markdown.push_str(&format!(
                        "**Tool Use**: {} (ID: {})\n",
                        tool_use.name, tool_use.id
                    ));
                    markdown.push_str(&format!(
                        "{}\n",
                        MarkdownCodeBlock {
                            tag: "json",
                            text: &format!("{:#}", tool_use.input)
                        }
                    ));
                }
            }
        }

        for tool_result in self.tool_results.values() {
            markdown.push_str(&format!(
                "**Tool Result**: {} (ID: {})\n\n",
                tool_result.tool_name, tool_result.tool_use_id
            ));
            if tool_result.is_error {
                markdown.push_str("**ERROR:**\n");
            }

            for part in &tool_result.content {
                match part {
                    LanguageModelToolResultContent::Text(text) => {
                        writeln!(markdown, "{text}\n").ok();
                    }
                    LanguageModelToolResultContent::Image(_) => {
                        writeln!(markdown, "<image />\n").ok();
                    }
                }
            }

            if let Some(output) = tool_result.output.as_ref() {
                writeln!(
                    markdown,
                    "**Debug Output**:\n\n```json\n{}\n```\n",
                    serde_json::to_string_pretty(output).unwrap()
                )
                .unwrap();
            }
        }

        markdown
    }

    pub fn to_request(&self) -> Vec<LanguageModelRequestMessage> {
        let mut assistant_message = LanguageModelRequestMessage {
            role: Role::Assistant,
            content: Vec::with_capacity(self.content.len()),
            cache: false,
            reasoning_details: self.reasoning_details.clone(),
        };
        for chunk in &self.content {
            match chunk {
                AgentMessageContent::Text(text) => {
                    assistant_message
                        .content
                        .push(language_model::MessageContent::Text(text.clone()));
                }
                AgentMessageContent::Thinking { text, signature } => {
                    assistant_message
                        .content
                        .push(language_model::MessageContent::Thinking {
                            text: text.clone(),
                            signature: signature.clone(),
                        });
                }
                AgentMessageContent::RedactedThinking(value) => {
                    assistant_message.content.push(
                        language_model::MessageContent::RedactedThinking(value.clone()),
                    );
                }
                AgentMessageContent::ToolUse(tool_use) => {
                    if self.tool_results.contains_key(&tool_use.id) {
                        assistant_message
                            .content
                            .push(language_model::MessageContent::ToolUse(tool_use.clone()));
                    }
                }
            };
        }

        let mut user_message = LanguageModelRequestMessage {
            role: Role::User,
            content: Vec::new(),
            cache: false,
            reasoning_details: None,
        };

        for tool_result in self.tool_results.values() {
            let mut tool_result = tool_result.clone();
            // Surprisingly, the API fails if we return an empty string here.
            // It thinks we are sending a tool use without a tool result.
            if tool_result.is_content_empty() {
                tool_result.content = vec!["<Tool returned an empty string>".into()];
            }
            user_message
                .content
                .push(language_model::MessageContent::ToolResult(tool_result));
        }

        let mut messages = Vec::new();
        if !assistant_message.content.is_empty() {
            messages.push(assistant_message);
        }
        if !user_message.content.is_empty() {
            messages.push(user_message);
        }
        messages
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub(crate) content: Vec<AgentMessageContent>,
    pub(crate) tool_results: IndexMap<LanguageModelToolUseId, LanguageModelToolResult>,
    pub(crate) reasoning_details: Option<Arc<serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentMessageContent {
    Text(String),
    Thinking {
        text: String,
        signature: Option<String>,
    },
    RedactedThinking(String),
    ToolUse(LanguageModelToolUse),
}

