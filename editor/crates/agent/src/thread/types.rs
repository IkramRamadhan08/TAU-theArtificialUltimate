use super::agent_message::AgentMessage;
use super::user_message::UserMessage;
use agent_client_protocol::schema as acp;
use gpui::SharedString;
use language_model::{LanguageModelProviderId, LanguageModelRequestMessage, Role};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;
#[derive(Debug)]
pub struct NoModelConfiguredError;

impl std::fmt::Display for NoModelConfiguredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no language model configured")
    }
}

impl std::error::Error for NoModelConfiguredError {}

/// Context passed to a subagent thread for lifecycle management
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentContext {
    /// ID of the parent thread
    pub parent_thread_id: acp::SessionId,

    /// Current depth level (0 = root agent, 1 = first-level subagent, etc.)
    pub depth: u8,
}

/// The ID of the user prompt that initiated a request.
///
/// This equates to the user physically submitting a message to the model (e.g., by pressing the Enter key).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize)]
pub struct PromptId(Arc<str>);

impl PromptId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string().into())
    }
}

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub(crate) const MAX_RETRY_ATTEMPTS: u8 = 4;
pub(crate) const BASE_RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(crate) enum RetryStrategy {
    ExponentialBackoff {
        initial_delay: Duration,
        max_attempts: u8,
    },
    Fixed {
        delay: Duration,
        max_attempts: u8,
    },
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum Message {
    User(UserMessage),
    Agent(AgentMessage),
    Resume,
    Compaction(CompactionInfo),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum CompactionInfo {
    Summary(SharedString),
    ProviderNative {
        provider: LanguageModelProviderId,
        items: Vec<serde_json::Value>,
    },
}

impl CompactionInfo {
    fn to_request(&self) -> Vec<LanguageModelRequestMessage> {
        match self {
            Self::Summary(summary) => vec![LanguageModelRequestMessage {
                role: Role::User,
                content: vec![format!(
                    "The previous conversation was compacted. Use this summary as context:\n\n{}",
                    summary
                )
                .into()],
                cache: false,
                reasoning_details: None,
            }],
            Self::ProviderNative { .. } => Vec::new(),
        }
    }
}

impl Message {
    pub fn as_agent_message(&self) -> Option<&AgentMessage> {
        match self {
            Message::Agent(agent_message) => Some(agent_message),
            _ => None,
        }
    }

    pub fn to_request(&self) -> Vec<LanguageModelRequestMessage> {
        match self {
            Message::User(message) => {
                if message.content.is_empty() {
                    vec![]
                } else {
                    vec![message.to_request()]
                }
            }
            Message::Agent(message) => message.to_request(),
            Message::Compaction(info) => info.to_request(),
            Message::Resume => vec![LanguageModelRequestMessage {
                role: Role::User,
                content: vec!["Continue where you left off".into()],
                cache: false,
                reasoning_details: None,
            }],
        }
    }

    pub fn to_markdown(&self) -> String {
        match self {
            Message::User(message) => message.to_markdown(),
            Message::Agent(message) => message.to_markdown(),
            Message::Resume => "[resume]\n".into(),
            Message::Compaction(_) => "--- Context Compacted ---\n".into(),
        }
    }

    pub fn role(&self) -> Role {
        match self {
            Message::User(_) | Message::Resume | Message::Compaction(_) => Role::User,
            Message::Agent(_) => Role::Assistant,
        }
    }
}
