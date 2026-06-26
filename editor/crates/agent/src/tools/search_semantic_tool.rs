use std::sync::Arc;

use anyhow::Result;
use gpui::{App, Entity, SharedString, Task};
use http_client::HttpClientWithUrl;
use language_model::{
    LanguageModelProviderId, LanguageModelToolResultContent,
};
use project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use util::markdown::MarkdownInlineCode;

use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;

/// Search the codebase semantically using RAG (Retrieval-Augmented Generation).
///
/// This tool searches across the entire project using both keyword matching
/// and semantic similarity to find relevant code. Results are ranked by relevance.
///
/// Unlike `grep` (which searches for exact patterns), this tool understands
/// the meaning behind your query. For example, searching "user authentication"
/// will find code related to login, passwords, sessions, and auth middleware
/// even if those exact words don't appear together.
///
/// Use this when:
/// - You need to find code related to a concept rather than an exact string
/// - You want to discover how a feature is implemented across the codebase
/// - Grep returns too many or too few results
/// - You're new to the codebase and exploring
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchSemanticToolInput {
    /// What you're looking for — a natural language description of the code or concept.
    /// Be specific: "database migration setup" works better than "database".
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 50).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Optional file path filter (e.g., "src/auth/" or "*.rs").
    /// Only results matching this pattern will be returned.
    pub file_filter: Option<String>,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchSemanticToolOutput {
    Success(String),
    Error { error: String },
}

impl From<SearchSemanticToolOutput> for LanguageModelToolResultContent {
    fn from(value: SearchSemanticToolOutput) -> Self {
        match value {
            SearchSemanticToolOutput::Success(response) => response.into(),
            SearchSemanticToolOutput::Error { error } => error.into(),
        }
    }
}

pub struct SearchSemanticTool {
    project: Entity<Project>,
    http_client: Arc<HttpClientWithUrl>,
}

impl SearchSemanticTool {
    pub fn new(project: Entity<Project>, http_client: Arc<HttpClientWithUrl>) -> Self {
        Self {
            project,
            http_client,
        }
    }
}

impl AgentTool for SearchSemanticTool {
    type Input = SearchSemanticToolInput;
    type Output = SearchSemanticToolOutput;

    const NAME: &'static str = "search_semantic";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Search
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) => {
                format!(
                    "Searching codebase for {}",
                    MarkdownInlineCode(&input.query)
                )
            }
            Err(_) => "Searching codebase semantically".into(),
        }
        .into()
    }

    fn supports_provider(_provider: &LanguageModelProviderId) -> bool {
        true
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let project = self.project.clone();
        let http_client = self.http_client.clone();

        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|e| SearchSemanticToolOutput::Error {
                    error: e.to_string(),
                })?;

            let authorize = cx.update(|cx| {
                let context = crate::ToolPermissionContext::new(
                    SearchSemanticTool::NAME,
                    vec![input.query.clone()],
                );
                event_stream.authorize(
                    format!(
                        "Search codebase for {}",
                        MarkdownInlineCode(&input.query)
                    ),
                    context,
                    cx,
                )
            });
            authorize
                .await
                .map_err(|e| SearchSemanticToolOutput::Error {
                    error: e.to_string(),
                })?;

            // Collect project files in foreground
            let files = cx.update(|cx| {
                project.read_with(cx, |project, cx| {
                    tau_rag::indexer::get_project_files(project, cx)
                })
            })
            .map_err(|e| SearchSemanticToolOutput::Error {
                error: format!("Failed to get project files: {}", e),
            })?;

            // Index in background
            let index_result = tau_rag::ensure_indexed(files, &**http_client).await;
            if let Err(e) = index_result {
                let _ = event_stream.update_fields(
                    acp::ToolCallUpdateFields::new()
                        .title(format!("Indexing warning: {}", e)),
                );
            }

            // Search
            let search_result = tau_rag::search(&input.query, input.limit, input.file_filter.as_deref())
                .map_err(|e| SearchSemanticToolOutput::Error {
                    error: format!("Search failed: {}", e),
                })?;

            if search_result.is_empty() {
                event_stream.update_fields(
                    acp::ToolCallUpdateFields::new()
                        .title("Semantic search completed — no results found"),
                );
                return Ok(SearchSemanticToolOutput::Success(
                    "No matching code found. Try a different query or use `grep` for exact pattern matching.".into(),
                ));
            }

            let mut output = String::new();
            output.push_str(&format!(
                "Found {} relevant result(s) for \"{}\":\n\n",
                search_result.len(),
                input.query
            ));

            for (i, result) in search_result.iter().enumerate() {
                output.push_str(&format!(
                    "### {}. `{}` (L{}-L{}) — relevance: {:.2}\n```\n{}\n```\n\n",
                    i + 1,
                    result.file_path,
                    result.start_line + 1,
                    result.end_line + 1,
                    result.score,
                    result.snippet,
                ));
            }

            event_stream.update_fields(
                acp::ToolCallUpdateFields::new()
                    .title(format!("Found {} semantic matches", search_result.len())),
            );

            Ok(SearchSemanticToolOutput::Success(output))
        })
    }
}
