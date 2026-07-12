use std::collections::HashMap;
use std::sync::Arc;

use agent_client_protocol::schema as acp;
use anyhow::Result;
use gpui::{App, SharedString, Task};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::credential_store::CredentialStore;
use crate::{AgentTool, ToolCallEventStream, ToolInput};

/// Requests credentials from the user and stores them persistently.
///
/// The agent calls this tool when it needs secrets (API keys, OAuth tokens,
/// etc.) to proceed. If the credentials are already stored, they are returned
/// immediately. Otherwise, the user is prompted to fill in the fields.
///
/// After the user submits, the values are saved to an encrypted store so the
/// user never has to re-enter them.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RequestCredentialToolInput {
    /// A short identifier for the service (e.g. "google_oauth", "openai_api").
    /// Used as the lookup key in the credential store.
    pub service: String,
    /// A human-readable title shown in the prompt (e.g. "Google OAuth Credentials").
    pub title: Option<String>,
    /// Step-by-step instructions telling the user where to find these credentials.
    pub instructions: String,
    /// The fields to collect. Each field has a `key` (env var name), `label`
    /// (human-readable), `description` (where to find it), and `secret`
    /// (whether to mask the input).
    pub fields: Vec<CredentialField>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CredentialField {
    /// The environment variable or config key (e.g. "GOOGLE_CLIENT_ID").
    pub key: String,
    /// Human-readable label shown in the form (e.g. "Client ID").
    pub label: String,
    /// Instructions for where to find this value.
    pub description: String,
    /// If true, the input is masked (for secrets/passwords).
    #[serde(default)]
    pub secret: bool,
    /// Example value showing the user what to paste.
    #[serde(default)]
    pub example: Option<String>,
}

impl From<CredentialField> for acp_thread::FormField {
    fn from(f: CredentialField) -> Self {
        acp_thread::FormField {
            key: f.key,
            label: f.label,
            description: f.description,
            secret: f.secret,
            example: f.example,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RequestCredentialToolOutput {
    /// Whether the credentials were already stored (`cached`) or just entered
    /// by the user (`fresh`).
    pub source: String,
    /// The credential values keyed by field key.
    pub values: HashMap<String, String>,
    /// Human-readable summary for the model.
    pub summary: String,
}

impl From<RequestCredentialToolOutput> for language_model::LanguageModelToolResultContent {
    fn from(output: RequestCredentialToolOutput) -> Self {
        language_model::LanguageModelToolResultContent::Text(
            format!(
                "Credentials for service `{}` (source: {})\n\n\
                 Values were securely stored and returned to the agent. \
                 The agent should use the returned values directly without \
                 logging them in chat or any output.\n\n{}",
                output.source,
                output.source,
                output.summary,
            )
            .into(),
        )
    }
}

pub struct RequestCredentialTool {
    store: CredentialStore,
}

impl RequestCredentialTool {
    pub fn new(store: CredentialStore) -> Self {
        Self { store }
    }
}

impl AgentTool for RequestCredentialTool {
    type Input = RequestCredentialToolInput;
    type Output = RequestCredentialToolOutput;

    const NAME: &'static str = "request_credential";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) => format!("Request credential: {}", input.service).into(),
            Err(_) => "Request credential".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = match input.recv().await {
                Ok(input) => input,
                Err(e) => {
                    return Err(RequestCredentialToolOutput {
                        source: "error".into(),
                        values: HashMap::new(),
                        summary: format!("Failed to receive tool input: {e}"),
                    })
                }
            };

            let service = input.service.clone();

            // 1. Check persistent store first
            match self.store.get(&service).await {
                Ok(Some(values)) => {
                    return Ok(RequestCredentialToolOutput {
                        source: "cached".into(),
                        values,
                        summary: "Credentials were loaded from the persistent store.".into(),
                    })
                }
                Ok(None) => { /* not stored, need to prompt */ }
                Err(e) => {
                    log::error!("Failed to read credential store: {e}");
                    // fall through to prompt user
                }
            }

            // 2. Build the form prompt
            let title = input
                .title
                .clone()
                .unwrap_or_else(|| format!("Credentials needed: {}", input.service));

            let message = format!(
                "{}\n\nPlease enter the following credentials. They will be saved securely for future use.",
                input.instructions
            );

            let form_fields: Vec<acp_thread::FormField> =
                input.fields.into_iter().map(|f| f.into()).collect();

            // 3. Request form data from the user
            let form_fut = cx.update(|cx| {
                event_stream.request_form_data(Some(title), Some(message), form_fields, cx)
            });
            let form_values = match form_fut.await {
                Ok(values) => values,
                Err(e) => {
                    return Err(RequestCredentialToolOutput {
                        source: "cancelled".into(),
                        values: HashMap::new(),
                        summary: format!("Credential request was cancelled: {e}"),
                    })
                }
            };

            // 4. Save to persistent store if values were actually entered
            if !form_values.is_empty() {
                if let Err(e) = self.store.set(&service, form_values.clone()).await {
                    log::error!("Failed to save credentials: {e}");
                }
                Ok(RequestCredentialToolOutput {
                    source: "fresh".into(),
                    values: form_values,
                    summary: "Credentials were provided by the user and saved for future use."
                        .into(),
                })
            } else {
                // Empty values means user needs to paste in chat
                Ok(RequestCredentialToolOutput {
                    source: "chat_paste".into(),
                    values: HashMap::new(),
                    summary: "The user needs to paste the credentials in the chat. Ask them to paste each value clearly.".into(),
                })
            }
        })
    }
}
