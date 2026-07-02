use super::Thread;
use super::event_stream::ThreadEventStream;
use super::thread_types::AgentPlan;
use super::ToolInputSender;
use super::ToolInputPayload;
use super::Message;
use super::AnyAgentTool;
use acp_thread;
use acp_thread::UserMessageId;
use agent_settings::{AutoCompactThreshold, SUMMARIZE_THREAD_PROMPT,
};
use anyhow::Result;
use collections::{HashMap, BTreeMap};
use gpui::{AsyncApp, EventEmitter, SharedString, Task};
use language_model::{
    CompletionIntent, LanguageModel, LanguageModelCompletionEvent,
    LanguageModelRequest, LanguageModelRequestMessage,
    LanguageModelToolUseId, MessageContent, Role, TokenUsage,
};
use std::marker::PhantomData;
use std::sync::Arc;
use futures::StreamExt;
use futures::channel::mpsc;
pub fn total_input_tokens(usage: TokenUsage) -> u64 {
    usage
        .input_tokens
        .saturating_add(usage.cache_creation_input_tokens)
        .saturating_add(usage.cache_read_input_tokens)
}

pub fn auto_compact_threshold_token_count(
    threshold: AutoCompactThreshold,
    max_token_count: u64,
) -> u64 {
    match threshold {
        AutoCompactThreshold::Percentage(percent) => {
            ((max_token_count as f64) * percent).ceil() as u64
        }
        AutoCompactThreshold::TokensUsed(tokens) => tokens,
        AutoCompactThreshold::TokensRemaining(tokens) => {
            max_token_count.saturating_sub(tokens).saturating_add(1)
        }
    }
}

pub(crate) struct CompactionTelemetry {
    pub(crate) trigger: &'static str,
    pub(crate) thread_id: String,
    pub(crate) parent_thread_id: Option<String>,
    pub(crate) prompt_id: String,
    pub(crate) model: String,
    pub(crate) model_provider: String,
    pub(crate) thinking_effort: Option<String>,
    pub(crate) max_tokens: u64,
    pub(crate) tokens_before: Option<u64>,
    pub(crate) auto_compact_enabled: bool,
    pub(crate) auto_compact_threshold: String,
    pub(crate) auto_compact_threshold_tokens: u64,
    pub(crate) retries: u32,
}

impl CompactionTelemetry {
    pub(crate) fn emit(self, status: &'static str, error: Option<String>, tokens_after: Option<u64>) {
        telemetry::event!(
            "Agent Compaction Completed",
            trigger = self.trigger,
            status = status,
            error = error,
            thread_id = self.thread_id,
            parent_thread_id = self.parent_thread_id,
            prompt_id = self.prompt_id,
            model = self.model,
            model_provider = self.model_provider,
            thinking_effort = self.thinking_effort,
            max_tokens = self.max_tokens,
            tokens_before = self.tokens_before,
            tokens_after = tokens_after,
            auto_compact_enabled = self.auto_compact_enabled,
            auto_compact_threshold = self.auto_compact_threshold,
            auto_compact_threshold_tokens = self.auto_compact_threshold_tokens,
            retries = self.retries,
        );
    }
}

pub(crate) fn user_message_byte_len(message: &LanguageModelRequestMessage) -> usize {
    message
        .content
        .iter()
        .map(|content| match content {
            MessageContent::Text(text) => text.len(),
            MessageContent::Image(image) => image.len(),
            // These can never occur in a user message
            MessageContent::Thinking { .. }
            | MessageContent::RedactedThinking(_)
            | MessageContent::ToolResult(_)
            | MessageContent::ToolUse(_) => 0,
        })
        .sum()
}

pub(crate) fn truncate_user_message_to_byte_budget(
    mut message: LanguageModelRequestMessage,
    byte_budget: usize,
) -> Option<LanguageModelRequestMessage> {
    let mut remaining_bytes = byte_budget;
    let mut content = Vec::with_capacity(message.content.len());

    for item in message.content {
        match item {
            MessageContent::Text(text) => {
                let fits = text.len() <= remaining_bytes;
                if let Some(text) = take_text_within_byte_budget(text, &mut remaining_bytes) {
                    content.push(MessageContent::Text(text));
                }
                if !fits {
                    break;
                }
            }
            MessageContent::Image(image) => {
                let byte_len = image.len();
                if let Some(bytes) = remaining_bytes.checked_sub(byte_len) {
                    remaining_bytes = bytes;
                    content.push(MessageContent::Image(image));
                } else {
                    break;
                }
            }
            // These can never occur in a user message
            MessageContent::Thinking { .. }
            | MessageContent::RedactedThinking(_)
            | MessageContent::ToolResult(_)
            | MessageContent::ToolUse(_) => {}
        }
    }

    if content.is_empty() {
        None
    } else {
        message.content = content;
        Some(message)
    }
}

pub(crate) fn take_text_within_byte_budget(text: String, remaining_bytes: &mut usize) -> Option<String> {
    if text.is_empty() || *remaining_bytes == 0 {
        return None;
    }

    if let Some(bytes) = remaining_bytes.checked_sub(text.len()) {
        *remaining_bytes = bytes;
        return Some(text);
    }

    let end = text.floor_char_boundary((*remaining_bytes).min(text.len()));
    *remaining_bytes = 0;

    let text = text[..end].to_string();

    if text.is_empty() { None } else { Some(text) }
}

/// Describes where a streamed compaction summary should land in the thread
/// once it completes successfully.
pub(crate) enum CompactionInsertion {
    /// Automatic compaction inserts the summary at an index computed up front
    /// (which may be before a trailing not-yet-answered user message).
    Auto { insertion_ix: usize },
    /// Manual `/compact` appends a zero-content user message followed by the summary.
    Manual { marker_id: UserMessageId },
}

pub(crate) struct RunningTurn {
    /// Holds the task that handles agent interaction until the end of the turn.
    /// Survives across multiple requests as the model performs tool calls and
    /// we run tools, report their results.
    pub(crate) _task: Task<()>,
    /// The current event stream for the running turn. Used to report a final
    /// cancellation event if we cancel the turn.
    pub(crate) event_stream: ThreadEventStream,
    /// The tools that are enabled for the current iteration of the turn.
    /// Refreshed at the start of each iteration via `refresh_turn_tools`.
    pub(crate) tools: BTreeMap<SharedString, Arc<dyn AnyAgentTool>>,
    /// Sender to signal tool cancellation. When cancel is called, this is
    /// set to true so all tools can detect user-initiated cancellation.
    pub(crate) cancellation_tx: watch::Sender<bool>,
    /// Senders for tools that support input streaming and have already been
    /// started but are still receiving input from the LLM.
    pub(crate) streaming_tool_inputs: HashMap<LanguageModelToolUseId, ToolInputSender>,
    /// The plan for the current turn, generated before execution starts.
    /// Used for verification and self-healing.
    pub(crate) plan: Option<AgentPlan>,
    /// Tracks which plan steps need verification after tool execution.
    pub(crate) pending_verification: Vec<String>,
    /// Whether any file-modifying tools (write_file, edit_file, etc.)
    /// were called during this turn. Used by the verification gate.
    pub(crate) has_modifications: bool,
    /// Whether the verification gate has already fired. Prevents
    /// re-triggering after the model responds to the gate prompt.
    pub(crate) gate_triggered: bool,
}

impl RunningTurn {
    pub(crate) fn new(
        event_stream: ThreadEventStream,
        tools: BTreeMap<SharedString, Arc<dyn AnyAgentTool>>,
        cancellation_tx: watch::Sender<bool>,
        task: Task<()>,
    ) -> Self {
        Self {
            _task: task,
            event_stream,
            tools,
            cancellation_tx,
            streaming_tool_inputs: HashMap::default(),
            plan: None,
            pending_verification: Vec::new(),
            has_modifications: false,
            gate_triggered: false,
        }
    }

    pub(crate) fn cancel(mut self) -> Task<()> {
        log::debug!("Cancelling in progress turn");
        self.cancellation_tx.send(true).ok();
        self.event_stream.send_canceled();
        self._task
    }
}

pub(crate) fn messages_to_markdown(messages: &[Arc<Message>]) -> String {
    let mut markdown = String::new();
    for (ix, message) in messages.iter().enumerate() {
        if ix > 0 {
            markdown.push('\n');
        }
        match &**message {
            Message::User(_) => markdown.push_str("## User\n\n"),
            Message::Agent(_) => markdown.push_str("## Assistant\n\n"),
            Message::Resume | Message::Compaction(_) => {}
        }
        markdown.push_str(&message.to_markdown());
    }
    markdown
}

pub fn build_thread_title_request(
    messages: &[Arc<Message>],
    temperature: Option<f32>,
) -> LanguageModelRequest {
    let mut request = LanguageModelRequest {
        intent: Some(CompletionIntent::ThreadSummarization),
        temperature,
        ..Default::default()
    };
    for message in messages {
        request.messages.extend(message.to_request());
    }
    request.messages.push(LanguageModelRequestMessage {
        role: Role::User,
        content: vec![SUMMARIZE_THREAD_PROMPT.into()],
        cache: false,
        reasoning_details: None,
    });
    request
}

pub async fn stream_thread_title(
    model: Arc<dyn LanguageModel>,
    request: LanguageModelRequest,
    cx: &AsyncApp,
) -> Result<String> {
    let mut title = String::new();
    let mut events = model.stream_completion(request, cx).await?;
    while let Some(event) = events.next().await {
        let LanguageModelCompletionEvent::Text(text) = event? else {
            continue;
        };
        if let Some(newline_ix) = text.find(|ch| ch == '\n' || ch == '\r') {
            title.push_str(&text[..newline_ix]);
            break;
        }
        title.push_str(&text);
    }
    Ok(title)
}

pub struct TokenUsageUpdated(pub Option<acp_thread::TokenUsage>);

impl EventEmitter<TokenUsageUpdated> for Thread {}

pub struct TitleUpdated;

impl EventEmitter<TitleUpdated> for Thread {}

/// A channel-based wrapper that delivers tool input to a running tool.
///
/// For non-streaming tools, created via `ToolInput::ready()` so `.recv()` resolves immediately.
/// For streaming tools, partial JSON snapshots arrive via `.recv_partial()` as the LLM streams
/// them, followed by the final complete input available through `.recv()`.
pub struct ToolInput<T> {
    pub(crate) rx: mpsc::UnboundedReceiver<ToolInputPayload<serde_json::Value>>,
    pub(crate) _phantom: PhantomData<T>,
}

