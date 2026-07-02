use crate::{
    ApplyCodeActionTool, CodeActionStore, ContextServerRegistry, CopyPathTool, CreateDirectoryTool,
    CreateThreadTool, DbLanguageModel, DbThread, DeletePathTool, DiagnosticsTool, EditFileTool,
    FetchTool, FindPathTool, FindReferencesTool, GitBranchTool, GitCommitTool, GitLogTool,
    GitPushTool, GitStatusTool, GetCodeActionsTool, GoToDefinitionTool, GrepTool,
    ListAgentsAndModelsTool, ListDirectoryTool, MovePathTool, ProjectSnapshot, ReadFileTool,
    RenameTool, SandboxedTerminalTool, SearchSemanticTool, SpawnAgentTool, SystemPromptTemplate,
    Template, Templates, TerminalTool, WebSearchTool, WriteFileTool,
};
use acp_thread::UserMessageId;
use action_log::ActionLog;
use agent_settings::UserAgentsMd;

use crate::sandboxing::{ThreadSandboxGrants, sandboxing_enabled};
use agent_client_protocol::schema as acp;
use agent_settings::{
    AgentProfileId, AgentSettings, COMPACTION_PROMPT,
    SUMMARIZE_THREAD_DETAILED_PROMPT,
};
use anyhow::{Context as _, Result, anyhow};
use chrono::{DateTime, Local, Utc};
use client::UserStore;
use cloud_api_types::Plan;
use collections::{HashMap, HashSet};
use futures::{
    FutureExt,
    channel::mpsc,
    future::Shared,
    stream::FuturesUnordered,
};
use futures::{StreamExt, stream};
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, SharedString, Task, WeakEntity,
};
use heck::ToSnakeCase as _;
use language_model::{
    CompletionIntent, LanguageModel, LanguageModelCompletionError, LanguageModelCompletionEvent,
    LanguageModelId, LanguageModelProviderId, LanguageModelRegistry,
    LanguageModelRequest, LanguageModelRequestMessage, LanguageModelRequestTool,
    LanguageModelToolResult, LanguageModelToolResultContent,
    LanguageModelToolUse, LanguageModelToolUseId, Role, SelectedModel, Speed,
    StopReason, TokenUsage, TAU_CLOUD_PROVIDER_ID,
};
use project::Project;
use prompt_store::ProjectContext;
use settings::{
    LanguageModelSelection, Settings,
};
use std::{cell::RefCell, ops::ControlFlow};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use util::{ResultExt, paths::PathStyle};
use uuid::Uuid;

const TOOL_CANCELED_MESSAGE: &str = "Tool canceled by user";
pub const MAX_TOOL_NAME_LENGTH: usize = 64;
pub const MAX_SUBAGENT_DEPTH: u8 = 1;

/// Auto-compaction is only available for models whose context window is at least
/// this large. For smaller models there isn't enough headroom for a compaction
/// pass to be worthwhile, so we leave the thread uncompacted and let the UI warn
/// the user instead.
pub const MIN_COMPACTION_CONTEXT_WINDOW: u64 = 80_000;

// Using the heuristic that 1 token is about 4 bytes, keep the last 80K bytes of user-message content (~20k tokens).
const COMPACTION_RETAINED_USER_MESSAGES_BYTE_BUDGET: usize = 80_000;

pub mod types;
pub mod user_message;
pub mod agent_message;
pub mod traits;
pub mod thread_types;
pub mod compaction;
pub mod tool_infra;
pub mod event_stream;
pub mod conversions;
#[cfg(test)]
pub mod tests;

pub use types::*;
pub use user_message::*;
pub use agent_message::*;
pub use traits::*;
pub use thread_types::*;
pub use compaction::*;
pub use tool_infra::*;
pub use event_stream::*;

pub struct Thread {
    id: acp::SessionId,
    prompt_id: PromptId,
    updated_at: DateTime<Utc>,
    title: Option<SharedString>,
    pending_title_generation: Option<Task<()>>,
    title_generation_failed: bool,
    pending_summary_generation: Option<Shared<Task<Option<SharedString>>>>,
    summary: Option<SharedString>,
    messages: Vec<Arc<Message>>,
    user_store: Entity<UserStore>,
    /// Holds the task that handles agent interaction until the end of the turn.
    /// Survives across multiple requests as the model performs tool calls and
    /// we run tools, report their results.
    running_turn: Option<RunningTurn>,
    /// Flag indicating the UI has a queued message waiting to be sent.
    /// Used to signal that the turn should end at the next message boundary.
    has_queued_message: bool,
    pending_message: Option<AgentMessage>,
    pub(crate) tools: BTreeMap<SharedString, Arc<dyn AnyAgentTool>>,
    request_token_usage: HashMap<UserMessageId, language_model::TokenUsage>,
    cumulative_token_usage: TokenUsage,
    /// The per-field maximum usage snapshot already added to
    /// `cumulative_token_usage` for the in-flight completion request. Reset at
    /// the start of each request.
    current_request_token_usage: TokenUsage,
    pending_compaction_telemetry: Option<CompactionTelemetry>,
    #[allow(unused)]
    initial_project_snapshot: Shared<Task<Option<Arc<ProjectSnapshot>>>>,
    pub(crate) context_server_registry: Entity<ContextServerRegistry>,
    profile_id: AgentProfileId,
    project_context: Entity<ProjectContext>,
    pub(crate) templates: Arc<Templates>,
    model: Option<Arc<dyn LanguageModel>>,
    summarization_model: Option<Arc<dyn LanguageModel>>,
    thinking_enabled: bool,
    thinking_effort: Option<String>,
    speed: Option<Speed>,
    prompt_capabilities_tx: watch::Sender<acp::PromptCapabilities>,
    pub(crate) prompt_capabilities_rx: watch::Receiver<acp::PromptCapabilities>,
    pub(crate) project: Entity<Project>,
    pub(crate) action_log: Entity<ActionLog>,
    /// True if this thread was imported from a shared thread and can be synced.
    imported: bool,
    /// If this is a subagent thread, contains context about the parent
    subagent_context: Option<SubagentContext>,
    /// The user's unsent prompt text, persisted so it can be restored when reloading the thread.
    draft_prompt: Option<Vec<acp::ContentBlock>>,
    ui_scroll_position: Option<gpui::ListOffset>,
    /// Weak references to running subagent threads for cancellation propagation
    running_subagents: Vec<WeakEntity<Thread>>,
    inherits_parent_model_settings: bool,
    sandboxed_terminal_temp_dir: Option<PathBuf>,
    /// Sandbox permissions the user approved "for the rest of the thread".
    /// Shared with each tool call's event stream so repeated requests for
    /// already-granted permissions skip the approval prompt.
    /// Never persisted — lives and dies with this thread.
    sandbox_grants: Rc<RefCell<ThreadSandboxGrants>>,
    /// When true, agent asks user for verbal confirmation ("shall I run?")
    /// before executing tools, instead of just doing it.
    pub(crate) require_verification: bool,
    /// Per-provider circuit breakers that trip after 5 consecutive failures
    /// and cool down for 60 seconds, preventing wasted API calls when a
    /// provider is unresponsive.
    circuit_breakers: HashMap<LanguageModelProviderId, crate::circuit_breaker::CircuitBreaker>,
}

impl Thread {
    fn prompt_capabilities(model: Option<&dyn LanguageModel>) -> acp::PromptCapabilities {
        let image = model.map_or(true, |model| model.supports_images());
        acp::PromptCapabilities::new()
            .image(image)
            .embedded_context(true)
    }

    pub fn new_subagent(parent_thread: &Entity<Thread>, cx: &mut Context<Self>) -> Self {
        let project = parent_thread.read(cx).project.clone();
        let project_context = parent_thread.read(cx).project_context.clone();
        let context_server_registry = parent_thread.read(cx).context_server_registry.clone();
        let templates = parent_thread.read(cx).templates.clone();
        let model = parent_thread.read(cx).model().cloned();
        let parent_action_log = parent_thread.read(cx).action_log().clone();
        let action_log =
            cx.new(|_cx| ActionLog::new(project.clone()).with_linked_action_log(parent_action_log));
        let mut thread = Self::new_internal(
            project,
            project_context,
            context_server_registry,
            templates,
            model,
            action_log,
            cx,
        );
        thread.subagent_context = Some(SubagentContext {
            parent_thread_id: parent_thread.read(cx).id().clone(),
            depth: parent_thread.read(cx).depth() + 1,
        });
        thread.inherit_parent_settings(parent_thread, cx);
        if let Some(subagent_model) = AgentSettings::get_global(cx).subagent_model.clone() {
            thread.inherits_parent_model_settings = false;
            thread.apply_model_selection(&subagent_model, cx);
        }
        thread
    }

    pub fn new(
        project: Entity<Project>,
        project_context: Entity<ProjectContext>,
        context_server_registry: Entity<ContextServerRegistry>,
        templates: Arc<Templates>,
        model: Option<Arc<dyn LanguageModel>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_internal(
            project.clone(),
            project_context,
            context_server_registry,
            templates,
            model,
            cx.new(|_cx| ActionLog::new(project)),
            cx,
        )
    }

    fn new_internal(
        project: Entity<Project>,
        project_context: Entity<ProjectContext>,
        context_server_registry: Entity<ContextServerRegistry>,
        templates: Arc<Templates>,
        model: Option<Arc<dyn LanguageModel>>,
        action_log: Entity<ActionLog>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = AgentSettings::get_global(cx);
        let profile_id = settings.default_profile.clone();
        let enable_thinking = settings
            .default_model
            .as_ref()
            .is_some_and(|model| model.enable_thinking);
        let thinking_effort = settings
            .default_model
            .as_ref()
            .and_then(|model| model.effort.clone());
        let speed = settings
            .default_model
            .as_ref()
            .and_then(|model| model.speed);
        let (prompt_capabilities_tx, prompt_capabilities_rx) =
            watch::channel(Self::prompt_capabilities(model.as_deref()));
        Self {
            id: acp::SessionId::new(uuid::Uuid::new_v4().to_string()),
            prompt_id: PromptId::new(),
            updated_at: Utc::now(),
            title: None,
            pending_title_generation: None,
            title_generation_failed: false,
            pending_summary_generation: None,
            summary: None,
            messages: Vec::new(),
            user_store: project.read(cx).user_store(),
            running_turn: None,
            has_queued_message: false,
            pending_message: None,
            tools: BTreeMap::default(),
            request_token_usage: HashMap::default(),
            cumulative_token_usage: TokenUsage::default(),
            current_request_token_usage: TokenUsage::default(),
            pending_compaction_telemetry: None,
            initial_project_snapshot: {
                let project_snapshot = Self::project_snapshot(project.clone(), cx);
                cx.foreground_executor()
                    .spawn(async move { Some(project_snapshot.await) })
                    .shared()
            },
            context_server_registry,
            profile_id,
            project_context,
            templates,
            model,
            summarization_model: None,
            thinking_enabled: enable_thinking,
            speed,
            thinking_effort,
            prompt_capabilities_tx,
            prompt_capabilities_rx,
            project,
            action_log,
            imported: false,
            subagent_context: None,
            draft_prompt: None,
            ui_scroll_position: None,
            running_subagents: Vec::new(),
            inherits_parent_model_settings: true,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: Rc::new(RefCell::new(ThreadSandboxGrants::default())),
            require_verification: false,
            circuit_breakers: HashMap::default(),
        }
    }

    /// Copies runtime-mutable settings from the parent thread so that
    /// subagents start with the same configuration the user selected.
    /// Every property that `set_*` propagates to `running_subagents`
    /// should be inherited here as well.
    fn inherit_parent_settings(&mut self, parent_thread: &Entity<Thread>, cx: &mut Context<Self>) {
        let parent = parent_thread.read(cx);
        self.speed = parent.speed;
        self.thinking_enabled = parent.thinking_enabled;
        self.thinking_effort = parent.thinking_effort.clone();
        self.summarization_model = parent.summarization_model.clone();
        self.profile_id = parent.profile_id.clone();
        self.require_verification = parent.require_verification;
    }

    fn apply_model_selection(
        &mut self,
        selection: &LanguageModelSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = Self::resolve_model_from_selection(selection, cx) else {
            log::warn!(
                "failed to resolve configured subagent model: {}/{}",
                selection.provider.0,
                selection.model
            );
            return;
        };

        self.model = Some(model.clone());
        self.thinking_enabled = selection.enable_thinking && model.supports_thinking();
        self.thinking_effort = selection.effort.clone();
        self.speed = selection.speed.filter(|_| model.supports_fast_mode());
        self.prompt_capabilities_tx
            .send(Self::prompt_capabilities(self.model.as_deref()))
            .log_err();
    }

    pub fn id(&self) -> &acp::SessionId {
        &self.id
    }

    pub(crate) fn sandboxed_terminal_temp_dir(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf> {
        if let Some(temp_dir) = &self.sandboxed_terminal_temp_dir {
            std::fs::create_dir_all(temp_dir).with_context(|| {
                format!(
                    "failed to recreate sandboxed terminal temp directory {}",
                    temp_dir.display()
                )
            })?;
            return Ok(temp_dir.clone());
        }

        let temp_dir = tempfile::Builder::new()
            .prefix("tau-agent-terminal-")
            .tempdir()
            .context("failed to create sandboxed terminal temp directory")?;
        let temp_dir = temp_dir.keep();
        self.sandboxed_terminal_temp_dir = Some(temp_dir.clone());
        cx.notify();
        Ok(temp_dir)
    }

    /// Returns true if this thread was imported from a shared thread.
    pub fn is_imported(&self) -> bool {
        self.imported
    }

    pub fn replay(
        &mut self,
        cx: &mut Context<Self>,
    ) -> mpsc::UnboundedReceiver<Result<ThreadEvent>> {
        let (tx, rx) = mpsc::unbounded();
        let stream = ThreadEventStream::new(tx);
        for (message_ix, message) in self.messages.iter().enumerate() {
            match &**message {
                Message::User(user_message) => stream.send_user_message(user_message),
                Message::Agent(assistant_message) => {
                    for content in &assistant_message.content {
                        match content {
                            AgentMessageContent::Text(text) => stream.send_text(text),
                            AgentMessageContent::Thinking { text, .. } => {
                                stream.send_thinking(text)
                            }
                            AgentMessageContent::RedactedThinking(_) => {}
                            AgentMessageContent::ToolUse(tool_use) => {
                                self.replay_tool_call(
                                    tool_use,
                                    assistant_message.tool_results.get(&tool_use.id),
                                    &stream,
                                    cx,
                                );
                            }
                        }
                    }
                }
                Message::Resume => {}
                Message::Compaction(info) => {
                    let compaction_id = acp_thread::ContextCompactionId(
                        format!("replay-compaction-{message_ix}").into(),
                    );
                    match info {
                        CompactionInfo::Summary(summary) => {
                            stream.send_context_compaction(
                                compaction_id.clone(),
                                acp_thread::ContextCompactionStatus::Completed,
                            );
                            stream.send_context_compaction_update(compaction_id.clone(), summary);
                        }
                        CompactionInfo::ProviderNative { .. } => {
                            stream.send_context_compaction(
                                compaction_id,
                                acp_thread::ContextCompactionStatus::Completed,
                            );
                        }
                    }
                }
            }
        }
        rx
    }

    fn replay_tool_call(
        &self,
        tool_use: &LanguageModelToolUse,
        tool_result: Option<&LanguageModelToolResult>,
        stream: &ThreadEventStream,
        cx: &mut Context<Self>,
    ) {
        // A tool call left only with the canceled sentinel produced nothing useful
        // (the sentinel is model-facing only, and is inserted exactly when a tool
        // had no real result). Don't replay it into the UI at all.
        if tool_result.is_some_and(Self::is_canceled_tool_result) {
            return;
        }

        let output = tool_result
            .as_ref()
            .and_then(|result| result.output.clone());
        let replay_content = tool_result.and_then(Self::tool_result_content_for_replay);
        let status = tool_result
            .as_ref()
            .map_or(acp::ToolCallStatus::Failed, |result| {
                if result.is_error {
                    acp::ToolCallStatus::Failed
                } else {
                    acp::ToolCallStatus::Completed
                }
            });

        // Recorded tool calls use the model-facing name, so a terminal call is
        // always keyed as `terminal` and resolves to the non-sandboxed
        // `TerminalTool` here, even if it originally ran under
        // `SandboxedTerminalTool`. That's safe because both variants share the
        // same `replay` behavior; replay only reconstructs UI state and never
        // re-runs the command or re-applies sandbox policy.
        let tool = self.tools.get(tool_use.name.as_ref()).cloned().or_else(|| {
            self.context_server_registry
                .read(cx)
                .servers()
                .find_map(|(_, tools)| {
                    if let Some(tool) = tools.get(tool_use.name.as_ref()) {
                        Some(tool.clone())
                    } else {
                        None
                    }
                })
        });

        let Some(tool) = tool else {
            // Tool not found (e.g., MCP server not connected after restart),
            // but still display the saved result if available.
            // We need to send both ToolCall and ToolCallUpdate events because the UI
            // only converts raw_output to displayable content in update_fields, not from_acp.
            stream
                .0
                .unbounded_send(Ok(ThreadEvent::ToolCall(
                    acp::ToolCall::new(tool_use.id.to_string(), tool_use.name.to_string())
                        .status(status)
                        .raw_input(tool_use.input.clone()),
                )))
                .ok();
            let mut fields = acp::ToolCallUpdateFields::new()
                .status(status)
                .raw_output(output);
            if let Some(content) = replay_content {
                fields = fields.content(content);
            }
            stream.update_tool_call_fields(&tool_use.id, fields, None);
            return;
        };

        let title = tool.initial_title(tool_use.input.clone(), cx);
        let kind = tool.kind();
        stream.send_tool_call(
            &tool_use.id,
            &tool_use.name,
            title,
            kind,
            tool_use.input.clone(),
        );

        if let Some(content) = replay_content {
            stream.update_tool_call_fields(
                &tool_use.id,
                acp::ToolCallUpdateFields::new().content(content),
                None,
            );
        }

        if let Some(output) = output.clone() {
            // For replay, we use a dummy cancellation receiver since the tool already completed
            let (_cancellation_tx, cancellation_rx) = watch::channel(false);
            let tool_event_stream = ToolCallEventStream::new(
                tool_use.id.clone(),
                stream.clone(),
                Some(self.project.read(cx).fs().clone()),
                cancellation_rx,
                self.sandbox_grants.clone(),
                self.require_verification,
            );
            tool.replay(tool_use.input.clone(), output, tool_event_stream, cx)
                .log_err();
        }

        stream.update_tool_call_fields(
            &tool_use.id,
            acp::ToolCallUpdateFields::new()
                .status(status)
                .raw_output(output),
            None,
        );
    }

    /// A canceled tool result carries only the model-facing `TOOL_CANCELED_MESSAGE`
    /// sentinel (inserted exactly when a tool had no real result). It's never
    /// meaningful to the user, so we detect it to skip replaying the tool call.
    fn is_canceled_tool_result(tool_result: &LanguageModelToolResult) -> bool {
        tool_result.is_error
            && matches!(
                tool_result.content.as_slice(),
                [LanguageModelToolResultContent::Text(text)]
                    if text.as_ref() == TOOL_CANCELED_MESSAGE
            )
    }

    fn tool_result_content_for_replay(
        tool_result: &LanguageModelToolResult,
    ) -> Option<Vec<acp::ToolCallContent>> {
        let has_image = tool_result
            .content
            .iter()
            .any(|part| matches!(part, LanguageModelToolResultContent::Image(_)));
        if !has_image && tool_result.output.is_some() {
            return None;
        }

        let content = tool_result
            .content
            .iter()
            .filter_map(|part| match part {
                LanguageModelToolResultContent::Text(text) => {
                    if text.is_empty() {
                        None
                    } else {
                        Some(acp::ToolCallContent::Content(acp::Content::new(
                            acp::ContentBlock::Text(acp::TextContent::new(text.to_string())),
                        )))
                    }
                }
                LanguageModelToolResultContent::Image(image) => Some(
                    acp::ToolCallContent::Content(acp::Content::new(acp::ContentBlock::Image(
                        acp::ImageContent::new(image.source.clone(), "image/png"),
                    ))),
                ),
            })
            .collect::<Vec<_>>();

        if content.is_empty() {
            None
        } else {
            Some(content)
        }
    }

    pub fn from_db(
        id: acp::SessionId,
        db_thread: DbThread,
        project: Entity<Project>,
        project_context: Entity<ProjectContext>,
        context_server_registry: Entity<ContextServerRegistry>,
        templates: Arc<Templates>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = AgentSettings::get_global(cx);
        let profile_id = db_thread
            .profile
            .unwrap_or_else(|| settings.default_profile.clone());

        let mut model = LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
            let default_model = registry.default_model();

            // Prefer the current global default model. Only fall back to the
            // persisted thread model when the default can't be resolved (e.g.
            // the model configured in settings was removed or renamed). This
            // ensures that changing the default model in settings takes effect
            // on existing threads without needing to create a new thread.
            default_model
                .into_iter()
                .chain(
                    db_thread
                        .model
                        .and_then(|model| {
                            let model = SelectedModel {
                                provider: model.provider.clone().into(),
                                model: model.model.into(),
                            };
                            registry.select_model(&model, cx)
                        })
                        .into_iter(),
                )
                .next()
                .map(|model| model.model)
        });

        if model.is_none() {
            model = Self::resolve_profile_model(&profile_id, cx);
        }
        if model.is_none() {
            model = LanguageModelRegistry::global(cx).update(cx, |registry, _cx| {
                registry.default_model().map(|model| model.model)
            });
        }

        let (prompt_capabilities_tx, prompt_capabilities_rx) =
            watch::channel(Self::prompt_capabilities(model.as_deref()));

        let action_log = cx.new(|_| ActionLog::new(project.clone()));

        Self {
            id,
            prompt_id: PromptId::new(),
            title: if db_thread.title.is_empty() {
                None
            } else {
                Some(db_thread.title.clone())
            },
            pending_title_generation: None,
            title_generation_failed: false,
            pending_summary_generation: None,
            summary: db_thread.detailed_summary,
            messages: db_thread.messages,
            user_store: project.read(cx).user_store(),
            running_turn: None,
            has_queued_message: false,
            pending_message: None,
            tools: BTreeMap::default(),
            request_token_usage: db_thread.request_token_usage.clone(),
            cumulative_token_usage: db_thread.cumulative_token_usage,
            current_request_token_usage: TokenUsage::default(),
            pending_compaction_telemetry: None,
            initial_project_snapshot: Task::ready(db_thread.initial_project_snapshot).shared(),
            context_server_registry,
            profile_id,
            project_context,
            templates,
            model,
            summarization_model: None,
            thinking_enabled: db_thread.thinking_enabled,
            thinking_effort: db_thread.thinking_effort,
            speed: db_thread.speed,
            project,
            action_log,
            updated_at: db_thread.updated_at,
            prompt_capabilities_tx,
            prompt_capabilities_rx,
            imported: db_thread.imported,
            subagent_context: db_thread.subagent_context,
            draft_prompt: db_thread.draft_prompt,
            ui_scroll_position: db_thread.ui_scroll_position.map(|sp| gpui::ListOffset {
                item_ix: sp.item_ix,
                offset_in_item: gpui::px(sp.offset_in_item),
            }),
            running_subagents: Vec::new(),
            inherits_parent_model_settings: true,
            sandboxed_terminal_temp_dir: db_thread.sandboxed_terminal_temp_dir,
            sandbox_grants: Rc::new(RefCell::new(ThreadSandboxGrants::default())),
            require_verification: false,
            circuit_breakers: HashMap::default(),
        }
    }

    pub fn to_db(&self, cx: &App) -> Task<DbThread> {
        let initial_project_snapshot = self.initial_project_snapshot.clone();
        let mut thread = DbThread {
            title: self.title().unwrap_or_default(),
            messages: self.messages.clone(),
            updated_at: self.updated_at,
            detailed_summary: self.summary.clone(),
            initial_project_snapshot: None,
            cumulative_token_usage: self.cumulative_token_usage,
            request_token_usage: self.request_token_usage.clone(),
            model: self.model.as_ref().map(|model| DbLanguageModel {
                provider: model.provider_id().to_string(),
                model: model.id().0.to_string(),
            }),
            profile: Some(self.profile_id.clone()),
            imported: self.imported,
            subagent_context: self.subagent_context.clone(),
            speed: self.speed,
            thinking_enabled: self.thinking_enabled,
            thinking_effort: self.thinking_effort.clone(),
            draft_prompt: self.draft_prompt.clone(),
            ui_scroll_position: self.ui_scroll_position.map(|lo| {
                crate::db::SerializedScrollPosition {
                    item_ix: lo.item_ix,
                    offset_in_item: lo.offset_in_item.as_f32(),
                }
            }),
            sandboxed_terminal_temp_dir: self.sandboxed_terminal_temp_dir.clone(),
        };

        cx.background_spawn(async move {
            let initial_project_snapshot = initial_project_snapshot.await;
            thread.initial_project_snapshot = initial_project_snapshot;
            thread
        })
    }

    /// Create a snapshot of the current project state including git information and unsaved buffers.
    fn project_snapshot(
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Task<Arc<ProjectSnapshot>> {
        let task = project::telemetry_snapshot::TelemetrySnapshot::new(&project, cx);
        cx.spawn(async move |_, _| {
            let snapshot = task.await;

            Arc::new(ProjectSnapshot {
                worktree_snapshots: snapshot.worktree_snapshots,
                timestamp: Utc::now(),
            })
        })
    }

    pub fn project_context(&self) -> &Entity<ProjectContext> {
        &self.project_context
    }

    pub fn project(&self) -> &Entity<Project> {
        &self.project
    }

    pub fn action_log(&self) -> &Entity<ActionLog> {
        &self.action_log
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.title.is_none()
    }

    pub fn draft_prompt(&self) -> Option<&[acp::ContentBlock]> {
        self.draft_prompt.as_deref()
    }

    pub fn set_draft_prompt(&mut self, prompt: Option<Vec<acp::ContentBlock>>) {
        self.draft_prompt = prompt;
    }

    pub fn ui_scroll_position(&self) -> Option<gpui::ListOffset> {
        self.ui_scroll_position
    }

    pub fn set_ui_scroll_position(&mut self, position: Option<gpui::ListOffset>) {
        self.ui_scroll_position = position;
    }

    pub fn model(&self) -> Option<&Arc<dyn LanguageModel>> {
        self.model.as_ref()
    }

    pub fn set_model(&mut self, model: Arc<dyn LanguageModel>, cx: &mut Context<Self>) {
        let old_usage = self.latest_token_usage();
        self.model = Some(model.clone());
        let new_caps = Self::prompt_capabilities(self.model.as_deref());
        let new_usage = self.latest_token_usage();
        if old_usage != new_usage {
            cx.emit(TokenUsageUpdated(new_usage));
        }
        self.prompt_capabilities_tx.send(new_caps).log_err();

        for subagent in &self.running_subagents {
            subagent
                .update(cx, |thread, cx| {
                    if thread.inherits_parent_model_settings {
                        thread.set_model(model.clone(), cx);
                    }
                })
                .ok();
        }

        cx.notify()
    }

    pub fn summarization_model(&self) -> Option<&Arc<dyn LanguageModel>> {
        self.summarization_model.as_ref()
    }

    pub fn set_summarization_model(
        &mut self,
        model: Option<Arc<dyn LanguageModel>>,
        cx: &mut Context<Self>,
    ) {
        self.summarization_model = model.clone();

        for subagent in &self.running_subagents {
            subagent
                .update(cx, |thread, cx| {
                    thread.set_summarization_model(model.clone(), cx)
                })
                .ok();
        }
        cx.notify()
    }

    pub fn thinking_enabled(&self) -> bool {
        self.thinking_enabled
    }

    pub fn set_thinking_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.thinking_enabled = enabled;

        for subagent in &self.running_subagents {
            subagent
                .update(cx, |thread, cx| {
                    if thread.inherits_parent_model_settings {
                        thread.set_thinking_enabled(enabled, cx);
                    }
                })
                .ok();
        }
        cx.notify();
    }

    pub fn thinking_effort(&self) -> Option<&String> {
        self.thinking_effort.as_ref()
    }

    pub fn set_thinking_effort(&mut self, effort: Option<String>, cx: &mut Context<Self>) {
        self.thinking_effort = effort.clone();

        for subagent in &self.running_subagents {
            subagent
                .update(cx, |thread, cx| {
                    if thread.inherits_parent_model_settings {
                        thread.set_thinking_effort(effort.clone(), cx)
                    }
                })
                .ok();
        }
        cx.notify();
    }

    pub fn speed(&self) -> Option<Speed> {
        self.speed
    }

    pub fn set_speed(&mut self, speed: Speed, cx: &mut Context<Self>) {
        self.speed = Some(speed);

        for subagent in &self.running_subagents {
            subagent
                .update(cx, |thread, cx| {
                    if thread.inherits_parent_model_settings {
                        thread.set_speed(speed, cx);
                    }
                })
                .ok();
        }
        cx.notify();
    }

    pub fn last_message(&self) -> Option<&Message> {
        self.messages.last().map(std::ops::Deref::deref)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn last_received_or_pending_message(&self) -> Option<Arc<Message>> {
        if let Some(message) = self.pending_message.clone() {
            Some(Arc::new(Message::Agent(message)))
        } else {
            self.messages.last().cloned()
        }
    }

    pub fn add_default_tools(
        &mut self,
        environment: Rc<dyn ThreadEnvironment>,
        cx: &mut Context<Self>,
    ) {
        // Only update the agent location for the root thread, not for subagents.
        let update_agent_location = self.parent_thread_id().is_none();

        let language_registry = self.project.read(cx).languages().clone();
        self.add_tool(CopyPathTool::new(self.project.clone()));
        self.add_tool(CreateDirectoryTool::new(self.project.clone()));
        self.add_tool(DeletePathTool::new(
            self.project.clone(),
            self.action_log.clone(),
        ));
        self.add_tool(EditFileTool::new(
            self.project.clone(),
            cx.weak_entity(),
            self.action_log.clone(),
            language_registry.clone(),
        ));
        self.add_tool(WriteFileTool::new(
            self.project.clone(),
            cx.weak_entity(),
            self.action_log.clone(),
            language_registry,
        ));
        self.add_tool(FetchTool::new(self.project.read(cx).client().http_client()));
        self.add_tool(FindPathTool::new(self.project.clone()));
        self.add_tool(GrepTool::new(self.project.clone()));
        self.add_tool(ListDirectoryTool::new(self.project.clone()));
        self.add_tool(MovePathTool::new(self.project.clone()));
        self.add_tool(ReadFileTool::new(
            self.project.clone(),
            self.action_log.clone(),
            update_agent_location,
        ));
        // Register terminal tool variants; `enabled_tools` exposes the one
        // matching the current sandbox state to the model as `terminal`.
        self.add_tool(TerminalTool::new(self.project.clone(), environment.clone()));
        self.add_tool(SandboxedTerminalTool::new(
            self.project.clone(),
            environment.clone(),
        ));
        self.add_tool(WebSearchTool);

        self.add_tool(DiagnosticsTool::new(self.project.clone()));
        self.add_tool(GitStatusTool::new(self.project.clone()));
        self.add_tool(GitLogTool::new(self.project.clone()));
        self.add_tool(GitBranchTool::new(self.project.clone()));
        self.add_tool(GitCommitTool::new(self.project.clone()));
        self.add_tool(GitPushTool::new(self.project.clone()));

        let code_action_store: CodeActionStore = cx.new(|_cx| None);
        self.add_tool(FindReferencesTool::new(self.project.clone()));
        self.add_tool(GetCodeActionsTool::new(
            self.project.clone(),
            code_action_store.clone(),
        ));
        self.add_tool(ApplyCodeActionTool::new(
            self.project.clone(),
            code_action_store,
        ));
        self.add_tool(GoToDefinitionTool::new(self.project.clone()));
        self.add_tool(RenameTool::new(self.project.clone()));
        self.add_tool(SearchSemanticTool::new(
            self.project.clone(),
            self.project.read(cx).client().http_client(),
        ));

        if self.depth() < MAX_SUBAGENT_DEPTH {
            self.add_tool(SpawnAgentTool::new(environment.clone()));
        }

        // Sibling-thread tools are exposed at every depth: a subagent should
        // still be able to kick off independent sibling work on behalf of the
        // user, even when it can no longer nest further subagents. Visibility
        // to the model is gated by `CreateThreadToolFeatureFlag` in
        // `Thread::enabled_tools`.
        self.add_tool(CreateThreadTool::new(environment.clone()));
        self.add_tool(ListAgentsAndModelsTool::new(environment));
    }

    pub fn add_tool<T: AgentTool>(&mut self, tool: T) {
        debug_assert!(
            !self.tools.contains_key(T::NAME),
            "Duplicate tool name: {}",
            T::NAME,
        );
        self.tools.insert(T::NAME.into(), tool.erase());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn remove_tool(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    pub fn profile(&self) -> &AgentProfileId {
        &self.profile_id
    }

    pub fn set_profile(&mut self, profile_id: AgentProfileId, cx: &mut Context<Self>) {
        if self.profile_id == profile_id {
            return;
        }

        self.profile_id = profile_id.clone();

        // Swap to the profile's preferred model when available.
        if let Some(model) = Self::resolve_profile_model(&self.profile_id, cx) {
            self.set_model(model, cx);
        }

        for subagent in &self.running_subagents {
            subagent
                .update(cx, |thread, cx| thread.set_profile(profile_id.clone(), cx))
                .ok();
        }
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) -> Task<()> {
        for subagent in self.running_subagents.drain(..) {
            if let Some(subagent) = subagent.upgrade() {
                subagent.update(cx, |thread, cx| thread.cancel(cx)).detach();
            }
        }

        let Some(running_turn) = self.running_turn.take() else {
            self.flush_pending_message(cx);
            return Task::ready(());
        };

        let turn_task = running_turn.cancel();

        cx.spawn(async move |this, cx| {
            turn_task.await;
            this.update(cx, |this, cx| {
                this.flush_pending_message(cx);
            })
            .ok();
        })
    }

    pub fn set_has_queued_message(&mut self, has_queued: bool) {
        self.has_queued_message = has_queued;
    }

    pub fn has_queued_message(&self) -> bool {
        self.has_queued_message
    }

    fn accumulate_token_usage(&mut self, update: language_model::TokenUsage) {
        let previous_accounted_usage = self.current_request_token_usage;
        let current_accounted_usage = TokenUsage {
            input_tokens: previous_accounted_usage
                .input_tokens
                .max(update.input_tokens),
            output_tokens: previous_accounted_usage
                .output_tokens
                .max(update.output_tokens),
            cache_creation_input_tokens: previous_accounted_usage
                .cache_creation_input_tokens
                .max(update.cache_creation_input_tokens),
            cache_read_input_tokens: previous_accounted_usage
                .cache_read_input_tokens
                .max(update.cache_read_input_tokens),
        };
        self.current_request_token_usage = current_accounted_usage;
        self.cumulative_token_usage = self.cumulative_token_usage
            + TokenUsage {
                input_tokens: current_accounted_usage
                    .input_tokens
                    .saturating_sub(previous_accounted_usage.input_tokens),
                output_tokens: current_accounted_usage
                    .output_tokens
                    .saturating_sub(previous_accounted_usage.output_tokens),
                cache_creation_input_tokens: current_accounted_usage
                    .cache_creation_input_tokens
                    .saturating_sub(previous_accounted_usage.cache_creation_input_tokens),
                cache_read_input_tokens: current_accounted_usage
                    .cache_read_input_tokens
                    .saturating_sub(previous_accounted_usage.cache_read_input_tokens),
            };
    }

    fn update_token_usage(&mut self, update: language_model::TokenUsage, cx: &mut Context<Self>) {
        self.accumulate_token_usage(update);

        let Some(last_user_message) = self.last_user_message() else {
            return;
        };

        self.request_token_usage
            .insert(last_user_message.id.clone(), update);
        cx.emit(TokenUsageUpdated(self.latest_token_usage()));
        cx.notify();
    }

    pub fn truncate(&mut self, message_id: UserMessageId, cx: &mut Context<Self>) -> Result<()> {
        self.cancel(cx).detach();
        // Clear pending message since cancel will try to flush it asynchronously,
        // and we don't want that content to be added after we truncate
        self.pending_message.take();
        let Some(position) = self.messages.iter().position(
            |msg| matches!(&**msg, Message::User(UserMessage { id, .. }) if id == &message_id),
        ) else {
            return Err(anyhow!("Message not found"));
        };

        for message in self.messages.drain(position..) {
            match &*message {
                Message::User(message) => {
                    self.request_token_usage.remove(&message.id);
                }
                Message::Agent(_) | Message::Resume | Message::Compaction(_) => {}
            }
        }
        self.clear_summary();
        cx.notify();
        Ok(())
    }

    pub fn latest_request_token_usage(&self) -> Option<language_model::TokenUsage> {
        let last_user_message = self.last_user_message()?;
        let tokens = self.request_token_usage.get(&last_user_message.id)?;
        Some(*tokens)
    }

    pub fn cumulative_token_usage(&self) -> language_model::TokenUsage {
        self.cumulative_token_usage
    }

    pub fn latest_token_usage(&self) -> Option<acp_thread::TokenUsage> {
        let usage = self.latest_request_token_usage()?;
        let model = self.model.clone()?;
        let input_tokens = total_input_tokens(usage);

        Some(acp_thread::TokenUsage {
            max_tokens: model.max_token_count(),
            max_output_tokens: model.max_output_tokens(),
            used_tokens: usage.total_tokens(),
            input_tokens,
            output_tokens: usage.output_tokens,
        })
    }

    /// Get the total input token count as of the message before the given message.
    ///
    /// Returns `None` if:
    /// - `target_id` is the first message (no previous message)
    /// - The previous message hasn't received a response yet (no usage data)
    /// - `target_id` is not found in the messages
    pub fn tokens_before_message(&self, target_id: &UserMessageId) -> Option<u64> {
        let mut previous_user_message_id: Option<&UserMessageId> = None;

        for message in &self.messages {
            if let Message::User(user_msg) = &**message {
                if &user_msg.id == target_id {
                    let prev_id = previous_user_message_id?;
                    let usage = self.request_token_usage.get(prev_id)?;
                    return Some(total_input_tokens(*usage));
                }
                previous_user_message_id = Some(&user_msg.id);
            }
        }
        None
    }

    /// Look up the active profile and resolve its preferred model if one is configured.
    fn resolve_profile_model(
        profile_id: &AgentProfileId,
        cx: &mut Context<Self>,
    ) -> Option<Arc<dyn LanguageModel>> {
        let selection = AgentSettings::get_global(cx)
            .profiles
            .get(profile_id)?
            .default_model
            .clone()?;
        Self::resolve_model_from_selection(&selection, cx)
    }

    /// Translate a stored model selection into the configured model from the registry.
    fn resolve_model_from_selection(
        selection: &LanguageModelSelection,
        cx: &mut Context<Self>,
    ) -> Option<Arc<dyn LanguageModel>> {
        let selected = SelectedModel {
            provider: LanguageModelProviderId::from(selection.provider.0.clone()),
            model: LanguageModelId::from(selection.model.clone()),
        };
        LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
            registry
                .select_model(&selected, cx)
                .map(|configured| configured.model)
        })
    }

    pub fn resume(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<mpsc::UnboundedReceiver<Result<ThreadEvent>>> {
        self.messages.push(Arc::new(Message::Resume));
        cx.notify();

        log::debug!("Total messages in thread: {}", self.messages.len());
        self.run_turn(cx)
    }

    /// Sending a message results in the model streaming a response, which could include tool calls.
    /// After calling tools, the model will stops and waits for any outstanding tool calls to be completed and their results sent.
    /// The returned channel will report all the occurrences in which the model stops before erroring or ending its turn.
    pub fn send<T>(
        &mut self,
        id: UserMessageId,
        content: impl IntoIterator<Item = T>,
        cx: &mut Context<Self>,
    ) -> Result<mpsc::UnboundedReceiver<Result<ThreadEvent>>>
    where
        T: Into<UserMessageContent>,
    {
        let content = content.into_iter().map(Into::into).collect::<Arc<_>>();
        log::debug!("Thread::send content: {:?}", content);

        self.messages
            .push(Arc::new(Message::User(UserMessage { id, content })));
        cx.notify();

        self.send_existing(cx)
    }

    pub fn send_existing(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<mpsc::UnboundedReceiver<Result<ThreadEvent>>> {
        let model = self
            .model()
            .ok_or_else(|| anyhow!(NoModelConfiguredError))?;

        log::info!("Thread::send called with model: {}", model.name().0);
        self.advance_prompt_id();

        log::debug!("Total messages in thread: {}", self.messages.len());
        self.run_turn(cx)
    }

    /// Force a manual context compaction using the summary strategy,
    /// regardless of the current token usage or context window size.
    pub fn compact(
        &mut self,
        id: UserMessageId,
        cx: &mut Context<Self>,
    ) -> Result<mpsc::UnboundedReceiver<Result<ThreadEvent>>> {
        let model = self
            .model
            .clone()
            .ok_or_else(|| anyhow!(NoModelConfiguredError))?;

        // Flush any pending message and cancel an in-flight turn before we
        // start, mirroring `run_turn` so a stray completion can't race with the
        // compaction we're about to perform.
        self.flush_pending_message(cx);
        self.cancel(cx).detach();

        let compaction = self.forced_compaction_target_ix().map(|request_end_ix| {
            self.advance_prompt_id();
            let request = self.build_compaction_request(request_end_ix, &model, cx);
            self.current_request_token_usage = TokenUsage::default();
            (model, request)
        });

        if compaction.is_some() {
            self.pending_compaction_telemetry = self.build_compaction_telemetry("manual", cx);
        }

        self.clear_summary();
        cx.notify();

        let (events_tx, events_rx) = mpsc::unbounded::<Result<ThreadEvent>>();
        let event_stream = ThreadEventStream::new(events_tx);
        let (cancellation_tx, mut cancellation_rx) = watch::channel(false);
        let task = cx.spawn({
            let event_stream = event_stream.clone();
            async move |this, cx| {
                let result = if let Some((model, request)) = compaction {
                    Self::stream_compaction(
                        &this,
                        &event_stream,
                        cancellation_rx.clone(),
                        model,
                        request,
                        CompactionInsertion::Manual { marker_id: id },
                        cx,
                    )
                    .await
                } else {
                    Ok(ControlFlow::Continue(()))
                };

                // If we were cancelled, `cancel()` already took `running_turn`
                // (possibly for a new turn), so leave it alone.
                if *cancellation_rx.borrow() {
                    this.update(cx, |this, _| {
                        this.emit_compaction_telemetry_outcome("canceled", None)
                    })
                    .log_err();
                    return;
                }

                match result {
                    // On success, the telemetry event is deferred until the next
                    // completion reports usage (see `handle_completion_event`),
                    // so we leave `pending_compaction_telemetry` in place here.
                    Ok(_) => event_stream.send_stop(acp::StopReason::EndTurn),
                    Err(error) => {
                        log::error!("Manual compaction failed: {:?}", error);
                        this.update(cx, |this, _| {
                            this.emit_compaction_telemetry_outcome(
                                "failed",
                                Some(error.to_string()),
                            )
                        })
                        .log_err();
                        event_stream.send_error(error);
                    }
                }

                _ = this.update(cx, |this, _| this.running_turn.take());
            }
        });
        self.running_turn = Some(RunningTurn::new(
            event_stream,
            BTreeMap::default(),
            cancellation_tx,
            task,
        ));

        Ok(events_rx)
    }

    pub fn push_acp_user_block(
        &mut self,
        id: UserMessageId,
        blocks: impl IntoIterator<Item = acp::ContentBlock>,
        path_style: PathStyle,
        cx: &mut Context<Self>,
    ) {
        let content = blocks
            .into_iter()
            .map(|block| UserMessageContent::from_content_block(block, path_style))
            .collect::<Arc<_>>();
        self.messages
            .push(Arc::new(Message::User(UserMessage { id, content })));
        cx.notify();
    }

    pub fn push_acp_agent_block(&mut self, block: acp::ContentBlock, cx: &mut Context<Self>) {
        let text = match block {
            acp::ContentBlock::Text(text_content) => text_content.text,
            acp::ContentBlock::Image(_) => "[image]".to_string(),
            acp::ContentBlock::Audio(_) => "[audio]".to_string(),
            acp::ContentBlock::ResourceLink(resource_link) => resource_link.uri,
            acp::ContentBlock::Resource(resource) => match resource.resource {
                acp::EmbeddedResourceResource::TextResourceContents(resource) => resource.uri,
                acp::EmbeddedResourceResource::BlobResourceContents(resource) => resource.uri,
                _ => "[resource]".to_string(),
            },
            _ => "[unknown]".to_string(),
        };

        self.messages.push(Arc::new(Message::Agent(AgentMessage {
            content: vec![AgentMessageContent::Text(text)],
            ..Default::default()
        })));
        cx.notify();
    }

    fn run_turn(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<mpsc::UnboundedReceiver<Result<ThreadEvent>>> {
        // Flush the old pending message synchronously before cancelling,
        // to avoid a race where the detached cancel task might flush the NEW
        // turn's pending message instead of the old one.
        self.flush_pending_message(cx);
        self.cancel(cx).detach();

        let (events_tx, events_rx) = mpsc::unbounded::<Result<ThreadEvent>>();
        let event_stream = ThreadEventStream::new(events_tx);
        let message_ix = self.messages.len().saturating_sub(1);
        self.clear_summary();
        let tools = self.enabled_tools(cx);
        let (cancellation_tx, mut cancellation_rx) = watch::channel(false);
        let task = cx.spawn({
            let event_stream = event_stream.clone();
            async move |this, cx| {
                log::debug!("Starting agent turn execution");

                let turn_result =
                    Self::run_turn_internal(&this, &event_stream, cancellation_rx.clone(), cx)
                        .await;

                // Check if we were cancelled - if so, cancel() already took running_turn
                // and we shouldn't touch it (it might be a NEW turn now)
                let was_cancelled = *cancellation_rx.borrow();
                if was_cancelled {
                    log::debug!("Turn was cancelled, skipping cleanup");
                    return;
                }

                _ = this.update(cx, |this, cx| this.flush_pending_message(cx));

                match turn_result {
                    Ok(()) => {
                        log::debug!("Turn execution completed");
                        event_stream.send_stop(acp::StopReason::EndTurn);
                    }
                    Err(error) => {
                        log::error!("Turn execution failed: {:?}", error);
                        match error.downcast::<CompletionError>() {
                            Ok(CompletionError::Refusal) => {
                                event_stream.send_stop(acp::StopReason::Refusal);
                                _ = this.update(cx, |this, _| this.messages.truncate(message_ix));
                            }
                            Ok(CompletionError::MaxTokens) => {
                                event_stream.send_stop(acp::StopReason::MaxTokens);
                            }
                            Ok(CompletionError::Other(error)) | Err(error) => {
                                event_stream.send_error(error);
                            }
                        }
                    }
                }

                _ = this.update(cx, |this, _| this.running_turn.take());
            }
        });
        self.running_turn = Some(RunningTurn::new(event_stream, tools, cancellation_tx, task));
        Ok(events_rx)
    }

    async fn run_turn_internal(
        this: &WeakEntity<Self>,
        event_stream: &ThreadEventStream,
        mut cancellation_rx: watch::Receiver<bool>,
        cx: &mut AsyncApp,
    ) -> Result<()> {
        let mut attempt = 0;
        let mut intent = CompletionIntent::UserPrompt;
        // Set when a refusal fallback occurs so subsequent iterations use the fallback model.
        let mut refusal_fallback_model: Option<Arc<dyn LanguageModel>> = None;
        loop {
            match Self::perform_compaction_if_needed(
                this,
                event_stream,
                cancellation_rx.clone(),
                cx,
            )
            .await
            {
                // On success the telemetry event is deferred until the
                // completion below reports usage, so we can record an
                // accurate post-compaction context size (see
                // `handle_completion_event`).
                Ok(ControlFlow::Continue(())) => {}
                Ok(ControlFlow::Break(())) => {
                    this.update(cx, |this, _| {
                        this.emit_compaction_telemetry_outcome("canceled", None)
                    })?;
                    return Ok(());
                }
                Err(error) => {
                    log::error!("Compaction failed: {}", error);
                    let error_message = error.to_string();
                    match error.downcast::<LanguageModelCompletionError>() {
                        Ok(error) => {
                            attempt += 1;
                            match Self::retry_completion_error(
                                this,
                                event_stream,
                                &mut cancellation_rx,
                                error,
                                attempt,
                                cx,
                            )
                            .await
                            {
                                Ok(ControlFlow::Break(())) => {
                                    this.update(cx, |this, _| {
                                        this.emit_compaction_telemetry_outcome("canceled", None)
                                    })?;
                                    return Ok(());
                                }
                                Ok(ControlFlow::Continue(())) => {
                                    this.update(cx, |this, _| {
                                        if let Some(telemetry) =
                                            this.pending_compaction_telemetry.as_mut()
                                        {
                                            telemetry.retries += 1;
                                        }
                                    })?;
                                    continue;
                                }
                                Err(retry_error) => {
                                    this.update(cx, |this, _| {
                                        this.emit_compaction_telemetry_outcome(
                                            "failed",
                                            Some(error_message),
                                        )
                                    })?;
                                    return Err(retry_error);
                                }
                            }
                        }
                        Err(error) => {
                            this.update(cx, |this, _| {
                                this.emit_compaction_telemetry_outcome(
                                    "failed",
                                    Some(error_message),
                                )
                            })?;
                            return Err(error);
                        }
                    }
                }
            }

            // Re-read the model and refresh tools on each iteration so that
            // mid-turn changes (e.g. the user switches model, toggles tools,
            // or changes profile) take effect between tool-call rounds.
            // If a refusal fallback is active, use that model instead.
            let (model, request) = this.update(cx, |this, cx| {
                let model = refusal_fallback_model
                    .clone()
                    .or_else(|| this.model.clone())
                    .ok_or_else(|| anyhow!(NoModelConfiguredError))?;
                this.refresh_turn_tools(cx);

                // Inject verification context if tools had retryable errors
                if let Some(running_turn) = this.running_turn.as_mut() {
                    if !running_turn.pending_verification.is_empty() {
                        let context = format!(
                            "[System] The following tools reported failures: {}. Their output may contain errors. Verify and retry if needed.",
                            running_turn.pending_verification.join(", ")
                        );
                        this.messages.push(Arc::new(Message::User(UserMessage {
                            id: UserMessageId::new(),
                            content: Arc::from([UserMessageContent::Text(context)]),
                        })));
                        running_turn.pending_verification.clear();
                    }
                }

                let request = this.build_completion_request(intent, cx)?;
                this.current_request_token_usage = TokenUsage::default();
                anyhow::Ok((model, request))
            })??;

            telemetry::event!(
                "Agent Thread Completion",
                thread_id = this.read_with(cx, |this, _| this.id.to_string())?,
                parent_thread_id = this.read_with(cx, |this, _| this
                    .parent_thread_id()
                    .map(|id| id.to_string()))?,
                prompt_id = this.read_with(cx, |this, _| this.prompt_id.to_string())?,
                model = model.telemetry_id(),
                model_provider = model.provider_id().to_string(),
                attempt
            );

            log::debug!("Calling model.stream_completion, attempt {}", attempt);

            // Circuit breaker: check before making the API call
            let provider_id = model.provider_id();
            let (mut events, mut error) = {
                let circuit_open = this.read_with(cx, |this, _| {
                    this.circuit_breakers
                        .get(&provider_id)
                        .map_or(false, |cb| cb.is_open())
                })?;
                if circuit_open {
                    let retry_after = this.read_with(cx, |this, _| {
                        this.circuit_breakers
                            .get(&provider_id)
                            .map(|cb| cb.retry_after())
                            .unwrap_or(Duration::from_secs(60))
                    })?;
                    (
                        stream::empty().boxed().fuse(),
                        Some(LanguageModelCompletionError::ServerOverloaded {
                            provider: model.provider_name(),
                            retry_after: Some(retry_after),
                        }),
                    )
                } else {
                    let request_timeout_secs = cx.update(|cx| AgentSettings::get_global(cx).request_timeout_secs);
                    let completion_timeout = cx.background_executor().timer(Duration::from_secs(request_timeout_secs));
                    match futures::select! {
                        result = model.stream_completion(request, cx).fuse() => result,
                        _ = completion_timeout.fuse() => {
                            Err(LanguageModelCompletionError::Other(
                                anyhow!("Request timed out after {} seconds", request_timeout_secs)
                            ))
                        }
                    } {
                        Ok(stream) => {
                            this.update(cx, |this, _| {
                                this.circuit_breakers
                                    .entry(provider_id)
                                    .or_insert_with(crate::circuit_breaker::CircuitBreaker::new)
                                    .record_success();
                            })?;
                            (stream.fuse(), None)
                        }
                        Err(err) => {
                            this.update(cx, |this, _| {
                                this.circuit_breakers
                                    .entry(provider_id)
                                    .or_insert_with(crate::circuit_breaker::CircuitBreaker::new)
                                    .record_failure();
                            })?;
                            (stream::empty().boxed().fuse(), Some(err))
                        }
                    }
                }
            };
            let mut tool_results: FuturesUnordered<Task<LanguageModelToolResult>> =
                FuturesUnordered::new();
            let mut early_tool_results: Vec<LanguageModelToolResult> = Vec::new();
            let mut cancelled = false;
            let mut had_refusal = false;
            loop {
                // Race between getting the first event, tool completion, and cancellation.
                let first_event = futures::select! {
                    event = events.next().fuse() => event,
                    tool_result = futures::StreamExt::select_next_some(&mut tool_results) => {
                        let is_error = tool_result.is_error;
                        let is_still_streaming = this
                            .read_with(cx, |this, _cx| {
                                this.running_turn
                                    .as_ref()
                                    .and_then(|turn| turn.streaming_tool_inputs.get(&tool_result.tool_use_id))
                                    .map_or(false, |inputs| !inputs.has_received_final())
                            })
                            .unwrap_or(false);

                        early_tool_results.push(tool_result);

                        // Only break if the tool errored and we are still
                        // streaming the input of the tool. If the tool errored
                        // but we are no longer streaming its input (i.e. there
                        // are parallel tool calls) we want to continue
                        // processing those tool inputs.
                        if is_error && is_still_streaming {
                            break;
                        }
                        continue;
                    }
                    _ = cancellation_rx.changed().fuse() => {
                        if *cancellation_rx.borrow() {
                            cancelled = true;
                            break;
                        }
                        continue;
                    }
                };
                let Some(first_event) = first_event else {
                    break;
                };

                // Collect all immediately available events to process as a batch
                let mut batch = vec![first_event];
                while let Some(event) = events.next().now_or_never().flatten() {
                    batch.push(event);
                }

                // Process the batch in a single update
                let batch_result = this.update(cx, |this, cx| {
                    let mut batch_tool_results = Vec::new();
                    let mut batch_error = None;

                    for event in batch {
                        log::trace!("Received completion event: {:?}", event);
                        match event {
                            Ok(event) => {
                                match this.handle_completion_event(
                                    event,
                                    event_stream,
                                    cancellation_rx.clone(),
                                    cx,
                                ) {
                                    Ok(Some(task)) => batch_tool_results.push(task),
                                    Ok(None) => {}
                                    Err(err) => {
                                        batch_error = Some(err);
                                        break;
                                    }
                                }
                            }
                            Err(err) => {
                                batch_error = Some(err.into());
                                break;
                            }
                        }
                    }

                    cx.notify();
                    (batch_tool_results, batch_error)
                })?;

                tool_results.extend(batch_result.0);
                if let Some(err) = batch_result.1 {
                    let is_refusal = err
                        .downcast_ref::<CompletionError>()
                        .is_some_and(|e| matches!(e, CompletionError::Refusal));
                    if is_refusal {
                        log::info!("Model refused request; checking for fallback model");
                        had_refusal = true;
                        break;
                    }
                    error = Some(err.downcast()?);
                    break;
                }
            }

            // Generate a plan from the first batch of tool calls, if not already generated.
            let needs_plan = this.read_with(cx, |this, _| {
                this.running_turn
                    .as_ref()
                    .and_then(|turn| turn.plan.as_ref())
                    .is_none()
            })?;
            if needs_plan {
                let (goal, steps): (String, Vec<PlanStep>) = this.update(cx, |this, _| {
                    let pending = this.pending_message();
                    let goal = pending
                        .content
                        .iter()
                        .filter_map(|c| {
                            if let crate::thread::agent_message::AgentMessageContent::Thinking {
                                text,
                                ..
                            } = c
                            {
                                // Use first ~200 chars of thinking as plan goal
                                let truncated: String =
                                    text.chars().take(200).collect();
                                Some(truncated)
                            } else {
                                None
                            }
                        })
                        .next()
                        .unwrap_or_default();
                    let steps: Vec<PlanStep> = pending
                        .content
                        .iter()
                        .filter_map(|c| {
                            if let crate::thread::agent_message::AgentMessageContent::ToolUse(
                                tool_use,
                            ) = c
                            {
                                Some(PlanStep {
                                    description: tool_use.name.to_string(),
                                    tool_name: Some(tool_use.name.to_string()),
                                    status: PlanStepStatus::Pending,
                                })
                            } else {
                                None
                            }
                        })
                        .collect();
                    (goal, steps)
                })?;
                if !steps.is_empty() {
                    let plan = AgentPlan { goal, steps };
                    event_stream.send_plan(plan.clone());
                    this.update(cx, |this, _| {
                        if let Some(running_turn) = this.running_turn.as_mut() {
                            running_turn.plan = Some(plan);
                        }
                    })?;
                }
            }

            // Drop the stream to release the rate limit permit before tool execution.
            // The stream holds a semaphore guard that limits concurrent requests.
            // Without this, the permit would be held during potentially long-running
            // tool execution, which could cause deadlocks when tools spawn subagents
            // that need their own permits.
            drop(events);

            // Drop streaming tool input senders that never received their final input.
            // This prevents deadlock when the LLM stream ends (e.g. because of an error)
            // before sending a tool use with `is_input_complete: true`.
            this.update(cx, |this, _cx| {
                if let Some(running_turn) = this.running_turn.as_mut() {
                    if running_turn.streaming_tool_inputs.is_empty() {
                        return;
                    }
                    log::warn!("Dropping partial tool inputs because the stream ended");
                    running_turn.streaming_tool_inputs.drain();
                }
            })?;

            if had_refusal {
                let maybe_fallback = this.update(cx, |this, cx| -> Option<Arc<dyn LanguageModel>> {
                    let current_model = refusal_fallback_model.as_ref().or(this.model.as_ref())?;
                    let fallback_id = match current_model.refusal_fallback_model_id() {
                        Some(id) => id,
                        None => {
                            log::info!(
                                "Refusal fallback: no fallback configured for model {} (provider {})",
                                current_model.id().0,
                                current_model.provider_id()
                            );
                            return None;
                        }
                    };
                    let provider_id = current_model.provider_id();
                    let found = LanguageModelRegistry::global(cx)
                        .read(cx)
                        .available_models(cx)
                        .find(|m| {
                            m.provider_id() == provider_id && m.id().0.as_ref() == fallback_id
                        });
                    if found.is_none() {
                        log::info!(
                            "Refusal fallback: fallback model {}/{} not found in available models",
                            provider_id,
                            fallback_id
                        );
                    }
                    found
                })?;

                if let Some(fallback) = maybe_fallback {
                    log::info!("Refusal fallback: retrying with {}", fallback.id().0);
                    let fallback_name = fallback.name().0.clone();
                    this.update(cx, |this, cx| {
                        this.pending_message = None;
                        this.set_model(fallback.clone(), cx);
                    })?;
                    event_stream.send_retry(acp_thread::RetryStatus {
                        last_error: "Safety filter triggered".into(),
                        attempt: 1,
                        max_attempts: 1,
                        started_at: Instant::now(),
                        duration: Duration::MAX,
                        meta: Some(acp_thread::meta_with_refusal_fallback(&fallback_name)),
                    });
                    refusal_fallback_model = Some(fallback);
                    continue;
                }
                log::info!("Request refused with no fallback model available");
                return Err(CompletionError::Refusal.into());
            }

            let end_turn = tool_results.is_empty() && early_tool_results.is_empty();

            // Process early tool results (those that completed before the LLM stream ended)
            for tool_result in early_tool_results {
                Self::process_tool_result(this, event_stream, cx, tool_result)?;
            }

            // Process remaining tool results as they complete
            while let Some(tool_result) = tool_results.next().await {
                Self::process_tool_result(this, event_stream, cx, tool_result)?;
            }

            // Self-healing: plan updates and verification tracking are handled in process_tool_result.

            this.update(cx, |this, cx| {
                this.flush_pending_message(cx);
                if this.title.is_none() {
                    this.generate_title(cx);
                }
            })?;

            if cancelled {
                log::debug!("Turn cancelled by user, exiting");
                return Ok(());
            }

            if let Some(error) = error {
                attempt += 1;
                match Self::retry_completion_error(
                    this,
                    event_stream,
                    &mut cancellation_rx,
                    error,
                    attempt,
                    cx,
                )
                .await?
                {
                    ControlFlow::Break(_) => return Ok(()),
                    ControlFlow::Continue(_) => {}
                }
                this.update(cx, |this, _cx| {
                    if let Some(Message::Agent(message)) = this.last_message() {
                        if message.tool_results.is_empty() {
                            intent = CompletionIntent::UserPrompt;
                            this.messages.push(Arc::new(Message::Resume));
                        }
                    }
                })?;
            } else if end_turn {
                // Verification gate: after the model finishes its response
                // (no more tool calls), check if modifications were made and
                // the gate hasn't fired yet. If so, inject a verification
                // prompt to force cargo check + test before concluding.
                let should_gate = this.update(cx, |this, _| {
                    this.running_turn.as_ref().map_or(false, |turn| {
                        turn.has_modifications && !turn.gate_triggered
                    })
                })?;
                if should_gate {
                    this.update(cx, |this, cx| {
                        if let Some(turn) = this.running_turn.as_mut() {
                            turn.gate_triggered = true;
                        }
                        this.flush_pending_message(cx);
                        this.pending_message = Some(AgentMessage {
                            content: vec![AgentMessageContent::Text(
                                "## Verification Required\n\nBefore concluding, you must verify your changes work:\n1. Run `cargo check` (or the project's build command) to confirm compilation.\n2. Run `cargo test` (or the project's test command) to confirm tests pass.\n3. Report what you ran and the results.\n\nDo NOT skip this step.".into()
                            )],
                            ..Default::default()
                        });
                        this.flush_pending_message(cx);
                    })?;
                    intent = CompletionIntent::ToolResults;
                    attempt = 0;
                } else {
                    return Ok(());
                }
            } else {
                let has_queued = this.update(cx, |this, _| this.has_queued_message())?;
                if has_queued {
                    log::debug!("Queued message found, ending turn at message boundary");
                    return Ok(());
                }
                intent = CompletionIntent::ToolResults;
                attempt = 0;
            }
        }
    }

    /// Computes the retry status for a failed completion, notifies listeners,
    /// and waits out the backoff delay (or returns early if the turn is
    /// cancelled while waiting). Returns an error if the completion is not
    /// retryable or retries are exhausted.
    async fn retry_completion_error(
        this: &WeakEntity<Self>,
        event_stream: &ThreadEventStream,
        cancellation_rx: &mut watch::Receiver<bool>,
        error: LanguageModelCompletionError,
        attempt: u8,
        cx: &mut AsyncApp,
    ) -> Result<ControlFlow<()>> {
        let retry = this.update(cx, |this, cx| {
            let user_store = this.user_store.read(cx);
            this.handle_completion_error(error, attempt, user_store.plan())
        })??;
        let timer = cx.background_executor().timer(retry.duration);
        event_stream.send_retry(retry);
        futures::select! {
            _ = timer.fuse() => {}
            _ = cancellation_rx.changed().fuse() => {
                if *cancellation_rx.borrow() {
                    log::debug!("Turn cancelled during retry delay, exiting");
                    return Ok(ControlFlow::Break(()));
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    async fn perform_compaction_if_needed(
        this: &WeakEntity<Self>,
        event_stream: &ThreadEventStream,
        cancellation_rx: watch::Receiver<bool>,
        cx: &mut AsyncApp,
    ) -> Result<ControlFlow<()>> {
        let Some((model, request, insertion_ix)) = this.update(cx, |this, cx| {
            let insertion_ix = this.compaction_message_target_ix(cx)?;
            let model = this.model.clone()?;
            let request = this.build_compaction_request(insertion_ix, &model, cx);
            this.current_request_token_usage = TokenUsage::default();
            // Preserve telemetry across retries so the retry count keeps
            // accumulating rather than resetting on each attempt.
            if this.pending_compaction_telemetry.is_none() {
                this.pending_compaction_telemetry = this.build_compaction_telemetry("auto", cx);
            }
            Some((model, request, insertion_ix))
        })?
        else {
            return Ok(ControlFlow::Continue(()));
        };

        Self::stream_compaction(
            this,
            event_stream,
            cancellation_rx,
            model,
            request,
            CompactionInsertion::Auto { insertion_ix },
            cx,
        )
        .await
    }

    async fn stream_compaction(
        this: &WeakEntity<Self>,
        event_stream: &ThreadEventStream,
        mut cancellation_rx: watch::Receiver<bool>,
        model: Arc<dyn LanguageModel>,
        request: LanguageModelRequest,
        insertion: CompactionInsertion,
        cx: &mut AsyncApp,
    ) -> Result<ControlFlow<()>> {
        log::debug!("Running compaction");
        let compaction_id = acp_thread::ContextCompactionId(Uuid::new_v4().to_string().into());
        event_stream.send_context_compaction(
            compaction_id.clone(),
            acp_thread::ContextCompactionStatus::InProgress,
        );
        let request_timeout_secs = cx.update(|cx| AgentSettings::get_global(cx).request_timeout_secs);
        let compaction_timeout = cx.background_executor().timer(Duration::from_secs(request_timeout_secs));
        let stream = futures::select! {
            result = model.stream_completion(request, cx).fuse() => result,
            _ = compaction_timeout.fuse() => {
                Err(LanguageModelCompletionError::Other(anyhow!("Compaction request timed out after {} seconds", request_timeout_secs)))
            }
            _ = cancellation_rx.changed().fuse() => {
                if *cancellation_rx.borrow() {
                    log::debug!("Compaction cancelled before request started");
                    return Ok(ControlFlow::Break(()));
                }
                return Ok(ControlFlow::Continue(()));
            }
        };
        let mut stream = stream?;

        let mut summary = String::new();
        loop {
            let event = futures::select! {
                event = stream.next().fuse() => event,
                _ = cancellation_rx.changed().fuse() => {
                    if *cancellation_rx.borrow() {
                        log::debug!("Compaction cancelled while summarizing");
                        return Ok(ControlFlow::Break(()));
                    }
                    continue;
                }
            };

            let Some(event) = event else {
                break;
            };

            match event? {
                LanguageModelCompletionEvent::Text(text) => {
                    summary.push_str(&text);
                    event_stream.send_context_compaction_update(compaction_id.clone(), &text);
                }
                LanguageModelCompletionEvent::UsageUpdate(usage) => {
                    this.update(cx, |this, _cx| {
                        this.accumulate_token_usage(usage);
                    })?;
                }
                LanguageModelCompletionEvent::Stop(_)
                | LanguageModelCompletionEvent::Started
                | LanguageModelCompletionEvent::Queued { .. }
                | LanguageModelCompletionEvent::Thinking { .. }
                | LanguageModelCompletionEvent::RedactedThinking { .. }
                | LanguageModelCompletionEvent::ReasoningDetails(_)
                | LanguageModelCompletionEvent::ToolUse(_)
                | LanguageModelCompletionEvent::ToolUseJsonParseError { .. }
                | LanguageModelCompletionEvent::StartMessage { .. } => {}
            }
        }

        if *cancellation_rx.borrow() {
            log::debug!("Compaction cancelled after summarizing");
            return Ok(ControlFlow::Break(()));
        }

        let summary = summary.trim().to_string();
        if summary.is_empty() {
            log::warn!("Compaction produced an empty summary");
            return Err(anyhow::anyhow!("Compaction produced an empty summary"));
        }

        log::debug!("Compaction succeeded:\n{summary}");
        event_stream.update_context_compaction_status(
            compaction_id,
            acp_thread::ContextCompactionStatus::Completed,
        );

        this.update(cx, |this, cx| {
            let compaction = Arc::new(Message::Compaction(CompactionInfo::Summary(summary.into())));
            match insertion {
                CompactionInsertion::Auto { insertion_ix } => {
                    if insertion_ix <= this.messages.len() {
                        this.messages.insert(insertion_ix, compaction);
                    } else {
                        this.messages.push(compaction);
                    }
                }
                CompactionInsertion::Manual { marker_id } => {
                    this.messages.push(Arc::new(Message::User(UserMessage {
                        id: marker_id,
                        content: Arc::from([]),
                    })));
                    this.messages.push(compaction);
                }
            }
            cx.notify();
        })?;

        Ok(ControlFlow::Continue(()))
    }

    fn process_tool_result(
        this: &WeakEntity<Thread>,
        event_stream: &ThreadEventStream,
        cx: &mut AsyncApp,
        tool_result: LanguageModelToolResult,
    ) -> Result<(), anyhow::Error> {
        log::debug!("Tool finished {:?}", tool_result);
        let is_error = tool_result.is_error;

        let tool_name = tool_result.tool_name.to_string();
        let output_text = tool_result.text_contents();

        // Output verification: check for hidden failures in Ok results
        let suspicious_patterns = ["failed", "error", "timed out", "denied", "not found"];
        let has_hidden_failure = !is_error
            && !output_text.is_empty()
            && suspicious_patterns
                .iter()
                .any(|p| output_text.to_lowercase().contains(p));

        event_stream.update_tool_call_fields(
            &tool_result.tool_use_id,
            acp::ToolCallUpdateFields::new()
                .status(if is_error || has_hidden_failure {
                    acp::ToolCallStatus::Failed
                } else {
                    acp::ToolCallStatus::Completed
                })
                .raw_output(tool_result.output.clone()),
            None,
        );
        this.update(cx, |this, _cx| {
            this.pending_message()
                .tool_results
                .insert(tool_result.tool_use_id.clone(), tool_result);

            // Track file-modifying tools for the verification gate
            const MODIFICATION_TOOLS: &[&str] = &[
                "write_file",
                "edit_file",
                "create_directory",
                "delete_path",
                "move_path",
                "copy_path",
                "rename_symbol",
                "apply_code_action",
            ];
            if MODIFICATION_TOOLS.contains(&tool_name.as_str()) {
                if let Some(running_turn) = this.running_turn.as_mut() {
                    running_turn.has_modifications = true;
                }
            }

            // Update plan step status and track verification
            if let Some(running_turn) = this.running_turn.as_mut() {
                if let Some(plan) = &mut running_turn.plan {
                    for step in &mut plan.steps {
                        if step.tool_name.as_deref() == Some(&tool_name) {
                            if is_error || has_hidden_failure {
                                let reason = if is_error {
                                    format!("Tool {} failed", tool_name)
                                } else {
                                    format!(
                                        "Tool {} returned suspicious output",
                                        tool_name
                                    )
                                };
                                let status = PlanStepStatus::Failed(reason);
                                step.status = status.clone();
                                event_stream
                                    .send_plan_step_update(tool_name.clone(), status);
                            } else {
                                step.status = PlanStepStatus::Completed;
                                event_stream.send_plan_step_update(
                                    tool_name.clone(),
                                    PlanStepStatus::Completed,
                                );
                            }
                        }
                    }
                }
                if is_error || has_hidden_failure {
                    running_turn.pending_verification.push(tool_name);
                }
            }
        })?;
        Ok(())
    }

    fn handle_completion_error(
        &mut self,
        error: LanguageModelCompletionError,
        attempt: u8,
        plan: Option<Plan>,
    ) -> Result<acp_thread::RetryStatus> {
        let Some(model) = self.model.as_ref() else {
            return Err(anyhow!(error));
        };

        let auto_retry = if model.provider_id() == TAU_CLOUD_PROVIDER_ID {
            plan.is_some()
        } else {
            true
        };

        if !auto_retry {
            return Err(anyhow!(error));
        }

        let Some(strategy) = Self::retry_strategy_for(&error) else {
            return Err(anyhow!(error));
        };

        let max_attempts = match &strategy {
            RetryStrategy::ExponentialBackoff { max_attempts, .. } => *max_attempts,
            RetryStrategy::Fixed { max_attempts, .. } => *max_attempts,
        };

        if attempt > max_attempts {
            return Err(anyhow!(error));
        }

        let delay = match &strategy {
            RetryStrategy::ExponentialBackoff { initial_delay, .. } => {
                let delay_secs = initial_delay.as_secs() * 2u64.pow((attempt - 1) as u32);
                Duration::from_secs(delay_secs)
            }
            RetryStrategy::Fixed { delay, .. } => *delay,
        };
        log::debug!("Retry attempt {attempt} with delay {delay:?}");

        Ok(acp_thread::RetryStatus {
            last_error: error.to_string().into(),
            attempt: attempt as usize,
            max_attempts: max_attempts as usize,
            started_at: Instant::now(),
            duration: delay,
            meta: None,
        })
    }

    /// A helper method that's called on every streamed completion event.
    /// Returns an optional tool result task, which the main agentic loop will
    /// send back to the model when it resolves.
    fn handle_completion_event(
        &mut self,
        event: LanguageModelCompletionEvent,
        event_stream: &ThreadEventStream,
        cancellation_rx: watch::Receiver<bool>,
        cx: &mut Context<Self>,
    ) -> Result<Option<Task<LanguageModelToolResult>>> {
        log::trace!("Handling streamed completion event: {:?}", event);
        use LanguageModelCompletionEvent::*;

        match event {
            StartMessage { .. } => {
                self.flush_pending_message(cx);
                self.pending_message = Some(AgentMessage::default());
            }
            Text(new_text) => self.handle_text_event(new_text, event_stream),
            Thinking { text, signature } => {
                self.handle_thinking_event(text, signature, event_stream)
            }
            RedactedThinking { data } => self.handle_redacted_thinking_event(data),
            ReasoningDetails(details) => {
                let last_message = self.pending_message();
                // Store the last non-empty reasoning_details (overwrites earlier ones)
                // This ensures we keep the encrypted reasoning with signatures, not the early text reasoning
                if let serde_json::Value::Array(arr) = &details {
                    if !arr.is_empty() {
                        last_message.reasoning_details = Some(Arc::new(details));
                    }
                } else {
                    last_message.reasoning_details = Some(Arc::new(details));
                }
            }
            ToolUse(tool_use) => {
                return Ok(self.handle_tool_use_event(tool_use, event_stream, cancellation_rx, cx));
            }
            ToolUseJsonParseError {
                id,
                tool_name,
                raw_input,
                json_parse_error,
            } => {
                return Ok(self.handle_tool_use_json_parse_error_event(
                    id,
                    tool_name,
                    raw_input,
                    json_parse_error,
                    event_stream,
                    cancellation_rx,
                    cx,
                ));
            }
            UsageUpdate(usage) => {
                telemetry::event!(
                    "Agent Thread Completion Usage Updated",
                    thread_id = self.id.to_string(),
                    parent_thread_id = self.parent_thread_id().map(|id| id.to_string()),
                    prompt_id = self.prompt_id.to_string(),
                    model = self.model.as_ref().map(|m| m.telemetry_id()),
                    model_provider = self.model.as_ref().map(|m| m.provider_id().to_string()),
                    input_tokens = usage.input_tokens,
                    output_tokens = usage.output_tokens,
                    cache_creation_input_tokens = usage.cache_creation_input_tokens,
                    cache_read_input_tokens = usage.cache_read_input_tokens,
                );
                // A successful compaction defers its telemetry until the first
                // completion that follows it, so `tokens_after` reflects the
                // real post-compaction context size.
                if let Some(telemetry) = self.pending_compaction_telemetry.take() {
                    telemetry.emit("succeeded", None, Some(total_input_tokens(usage)));
                }
                self.update_token_usage(usage, cx);
            }
            Stop(StopReason::Refusal) => return Err(CompletionError::Refusal.into()),
            Stop(StopReason::MaxTokens) => return Err(CompletionError::MaxTokens.into()),
            Stop(StopReason::ToolUse | StopReason::EndTurn) => {}
            Started | Queued { .. } => {}
        }

        Ok(None)
    }

    fn handle_text_event(&mut self, new_text: String, event_stream: &ThreadEventStream) {
        event_stream.send_text(&new_text);

        let last_message = self.pending_message();
        if let Some(AgentMessageContent::Text(text)) = last_message.content.last_mut() {
            text.push_str(&new_text);
        } else {
            last_message
                .content
                .push(AgentMessageContent::Text(new_text));
        }
    }

    fn handle_thinking_event(
        &mut self,
        new_text: String,
        new_signature: Option<String>,
        event_stream: &ThreadEventStream,
    ) {
        event_stream.send_thinking(&new_text);

        let last_message = self.pending_message();
        if let Some(AgentMessageContent::Thinking { text, signature }) =
            last_message.content.last_mut()
        {
            text.push_str(&new_text);
            *signature = new_signature.or(signature.take());
        } else {
            last_message.content.push(AgentMessageContent::Thinking {
                text: new_text,
                signature: new_signature,
            });
        }
    }

    fn handle_redacted_thinking_event(&mut self, data: String) {
        let last_message = self.pending_message();
        last_message
            .content
            .push(AgentMessageContent::RedactedThinking(data));
    }

    fn handle_tool_use_event(
        &mut self,
        tool_use: LanguageModelToolUse,
        event_stream: &ThreadEventStream,
        cancellation_rx: watch::Receiver<bool>,
        cx: &mut Context<Self>,
    ) -> Option<Task<LanguageModelToolResult>> {
        cx.notify();

        let tool = self.tool(tool_use.name.as_ref());
        let mut title = SharedString::from(&tool_use.name);
        let mut kind = acp::ToolKind::Other;
        if let Some(tool) = tool.as_ref() {
            title = tool.initial_title(tool_use.input.clone(), cx);
            kind = tool.kind();
        }

        self.send_or_update_tool_use(&tool_use, title, kind, event_stream);

        let Some(tool) = tool else {
            let content = format!("No tool named {} exists", tool_use.name);
            return Some(Task::ready(LanguageModelToolResult {
                content: vec![LanguageModelToolResultContent::Text(Arc::from(content))],
                tool_use_id: tool_use.id,
                tool_name: tool_use.name,
                is_error: true,
                output: None,
            }));
        };

        if !tool_use.is_input_complete {
            if tool.supports_input_streaming() {
                let running_turn = self.running_turn.as_mut()?;
                if let Some(sender) = running_turn.streaming_tool_inputs.get_mut(&tool_use.id) {
                    sender.send_partial(tool_use.input);
                    return None;
                }

                let (mut sender, tool_input) = ToolInputSender::channel();
                sender.send_partial(tool_use.input);
                running_turn
                    .streaming_tool_inputs
                    .insert(tool_use.id.clone(), sender);

                let tool = tool.clone();
                log::debug!("Running streaming tool {}", tool_use.name);
                return Some(self.run_tool(
                    tool,
                    tool_input,
                    tool_use.id,
                    tool_use.name,
                    event_stream,
                    cancellation_rx,
                    cx,
                ));
            } else {
                return None;
            }
        }

        if let Some(mut sender) = self
            .running_turn
            .as_mut()?
            .streaming_tool_inputs
            .remove(&tool_use.id)
        {
            sender.send_full(tool_use.input);
            return None;
        }

        log::debug!("Running tool {}", tool_use.name);
        let tool_input = ToolInput::ready(tool_use.input);
        Some(self.run_tool(
            tool,
            tool_input,
            tool_use.id,
            tool_use.name,
            event_stream,
            cancellation_rx,
            cx,
        ))
    }

    fn run_tool(
        &self,
        tool: Arc<dyn AnyAgentTool>,
        tool_input: ToolInput<serde_json::Value>,
        tool_use_id: LanguageModelToolUseId,
        tool_name: Arc<str>,
        event_stream: &ThreadEventStream,
        cancellation_rx: watch::Receiver<bool>,
        cx: &mut Context<Self>,
    ) -> Task<LanguageModelToolResult> {
        let fs = self.project.read(cx).fs().clone();
        let tool_event_stream = ToolCallEventStream::new(
            tool_use_id.clone(),
            event_stream.clone(),
            Some(fs),
            cancellation_rx,
            self.sandbox_grants.clone(),
            self.require_verification,
        );
        tool_event_stream.update_fields(
            acp::ToolCallUpdateFields::new().status(acp::ToolCallStatus::InProgress),
        );
        let supports_images = self.model().is_some_and(|model| model.supports_images());
        let tool_result = tool.run(tool_input, tool_event_stream, cx);
        cx.foreground_executor().spawn(async move {
            let (is_error, output) = match tool_result.await {
                Ok(mut output) => {
                    if let Some(ref tool_error) = output.error {
                        if let Some(suggestion) = tool_error.suggestion() {
                            output.llm_output.push(LanguageModelToolResultContent::Text(Arc::from(format!("Suggestion: {}", suggestion))));
                        }
                        (true, output)
                    } else {
                        let contains_image = output.llm_output.iter().any(|part| matches!(part, LanguageModelToolResultContent::Image(_)));
                        if contains_image && !supports_images {
                            let placeholder = LanguageModelToolResultContent::Text(Arc::from("[Tool responded with an image, but this model doesn't support images]"));
                            let has_non_image = output.llm_output.iter().any(|part| !matches!(part, LanguageModelToolResultContent::Image(_)));
                            if has_non_image {
                                output.llm_output = output.llm_output.into_iter().map(|part| match part { LanguageModelToolResultContent::Image(_) => placeholder.clone(), other => other }).collect();
                                (false, output)
                            } else {
                                (true, anyhow::anyhow!("Attempted to read an image, but this model doesn't support it.").into())
                            }
                        } else {
                            (false, output)
                        }
                    }
                }
                Err(output) => (true, output),
            };

            LanguageModelToolResult {
                tool_use_id,
                tool_name,
                is_error,
                content: output.llm_output,
                output: Some(output.raw_output),
            }
        })
    }

    fn handle_tool_use_json_parse_error_event(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        tool_name: Arc<str>,
        raw_input: Arc<str>,
        json_parse_error: String,
        event_stream: &ThreadEventStream,
        cancellation_rx: watch::Receiver<bool>,
        cx: &mut Context<Self>,
    ) -> Option<Task<LanguageModelToolResult>> {
        let tool_use = LanguageModelToolUse {
            id: tool_use_id,
            name: tool_name,
            raw_input: raw_input.to_string(),
            input: serde_json::json!({}),
            is_input_complete: true,
            thought_signature: None,
        };
        self.send_or_update_tool_use(
            &tool_use,
            SharedString::from(&tool_use.name),
            acp::ToolKind::Other,
            event_stream,
        );

        let tool = self.tool(tool_use.name.as_ref());

        let Some(tool) = tool else {
            let content = format!("No tool named {} exists", tool_use.name);
            return Some(Task::ready(LanguageModelToolResult {
                content: vec![LanguageModelToolResultContent::Text(Arc::from(content))],
                tool_use_id: tool_use.id,
                tool_name: tool_use.name,
                is_error: true,
                output: None,
            }));
        };

        let error_message = format!("Error parsing input JSON: {json_parse_error}");

        if tool.supports_input_streaming()
            && let Some(mut sender) = self
                .running_turn
                .as_mut()?
                .streaming_tool_inputs
                .remove(&tool_use.id)
        {
            sender.send_invalid_json(error_message);
            return None;
        }

        log::debug!("Running tool {}. Received invalid JSON", tool_use.name);
        let tool_input = ToolInput::invalid_json(error_message);
        Some(self.run_tool(
            tool,
            tool_input,
            tool_use.id,
            tool_use.name,
            event_stream,
            cancellation_rx,
            cx,
        ))
    }

    fn send_or_update_tool_use(
        &mut self,
        tool_use: &LanguageModelToolUse,
        title: SharedString,
        kind: acp::ToolKind,
        event_stream: &ThreadEventStream,
    ) {
        // Ensure the last message ends in the current tool use
        let last_message = self.pending_message();

        let has_tool_use = last_message.content.iter_mut().rev().any(|content| {
            if let AgentMessageContent::ToolUse(last_tool_use) = content {
                if last_tool_use.id == tool_use.id {
                    *last_tool_use = tool_use.clone();
                    return true;
                }
            }
            false
        });

        if !has_tool_use {
            event_stream.send_tool_call(
                &tool_use.id,
                &tool_use.name,
                title,
                kind,
                tool_use.input.clone(),
            );
            last_message
                .content
                .push(AgentMessageContent::ToolUse(tool_use.clone()));
        } else {
            event_stream.update_tool_call_fields(
                &tool_use.id,
                acp::ToolCallUpdateFields::new()
                    .title(title.as_str())
                    .kind(kind)
                    .raw_input(tool_use.input.clone()),
                None,
            );
        }
    }

    pub fn title(&self) -> Option<SharedString> {
        self.title.clone()
    }

    pub fn is_generating_summary(&self) -> bool {
        self.pending_summary_generation.is_some()
    }

    pub fn is_generating_title(&self) -> bool {
        self.pending_title_generation.is_some()
    }

    pub fn has_failed_title_generation(&self) -> bool {
        self.title_generation_failed
    }

    pub fn can_generate_title(&self) -> bool {
        self.pending_title_generation.is_none() && self.summarization_model.is_some()
    }

    pub fn summary(&mut self, cx: &mut Context<Self>) -> Shared<Task<Option<SharedString>>> {
        if let Some(summary) = self.summary.as_ref() {
            return Task::ready(Some(summary.clone())).shared();
        }
        if let Some(task) = self.pending_summary_generation.clone() {
            return task;
        }
        let Some(model) = self.summarization_model.clone() else {
            log::error!("No summarization model available");
            return Task::ready(None).shared();
        };
        let mut request = LanguageModelRequest {
            intent: Some(CompletionIntent::ThreadContextSummarization),
            temperature: AgentSettings::temperature_for_model(&model, cx),
            ..Default::default()
        };

        for message in &self.messages {
            request.messages.extend(message.to_request());
        }

        request.messages.push(LanguageModelRequestMessage {
            role: Role::User,
            content: vec![SUMMARIZE_THREAD_DETAILED_PROMPT.into()],
            cache: false,
            reasoning_details: None,
        });

        let task = cx
            .spawn(async move |this, cx| {
                let mut summary = String::new();
                let mut messages = model.stream_completion(request, cx).await.log_err()?;
                while let Some(event) = messages.next().await {
                    let event = event.log_err()?;
                    let text = match event {
                        LanguageModelCompletionEvent::Text(text) => text,
                        _ => continue,
                    };

                    let mut lines = text.lines();
                    summary.extend(lines.next());
                }

                log::debug!("Setting summary: {}", summary);
                let summary = SharedString::from(summary);

                this.update(cx, |this, cx| {
                    this.summary = Some(summary.clone());
                    this.pending_summary_generation = None;
                    cx.notify()
                })
                .ok()?;

                Some(summary)
            })
            .shared();
        self.pending_summary_generation = Some(task.clone());
        task
    }

    pub fn generate_title(&mut self, cx: &mut Context<Self>) {
        if !self.can_generate_title() {
            return;
        }
        let Some(model) = self.summarization_model.clone() else {
            return;
        };
        self.spawn_title_generation(model, None, cx);
    }

    pub fn regenerate_title(&mut self, cx: &mut Context<Self>) -> bool {
        self.regenerate_title_with_callback(cx, |_title, _cx| {})
    }

    pub fn regenerate_title_with_callback(
        &mut self,
        cx: &mut Context<Self>,
        on_generated_title: impl FnOnce(SharedString, &mut Context<Self>) + 'static,
    ) -> bool {
        if self.pending_title_generation.is_some() {
            return false;
        }

        let Some(model) = self.summarization_model.clone() else {
            return false;
        };

        self.spawn_title_generation(model, Some(Box::new(on_generated_title)), cx);

        true
    }

    fn spawn_title_generation(
        &mut self,
        model: Arc<dyn LanguageModel>,
        on_generated_title: Option<Box<dyn FnOnce(SharedString, &mut Context<Self>)>>,
        cx: &mut Context<Self>,
    ) {
        self.title_generation_failed = false;
        log::debug!("Generating title with model: {:?}", model.name());

        let temperature = AgentSettings::temperature_for_model(&model, cx);
        let request = build_thread_title_request(&self.messages, temperature);

        let title_generation = cx.spawn(async move |_this, cx| {
            stream_thread_title(model, request, cx)
                .await
                .context("failed to generate thread title")
                .map(SharedString::from)
                .log_err()
        });

        self.pending_title_generation = Some(cx.spawn(async move |this, cx| {
            let title = title_generation.await;
            _ = this.update(cx, |this, cx| {
                this.pending_title_generation = None;
                if let Some(title) = title {
                    this.set_title(title.clone(), cx);
                    if let Some(on_generated_title) = on_generated_title {
                        on_generated_title(title, cx);
                    }
                } else {
                    this.title_generation_failed = true;
                    cx.emit(TitleUpdated);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
        self.pending_title_generation = None;
        self.title_generation_failed = false;
        if Some(&title) != self.title.as_ref() {
            self.title = Some(title);
            cx.emit(TitleUpdated);
            cx.notify();
        }
    }

    fn clear_summary(&mut self) {
        self.summary = None;
        self.pending_summary_generation = None;
    }

    fn last_user_message(&self) -> Option<&UserMessage> {
        self.messages
            .iter()
            .rev()
            .find_map(|message| match &**message {
                Message::User(user_message) => Some(user_message),
                Message::Agent(_) | Message::Resume | Message::Compaction(_) => None,
            })
    }

    fn pending_message(&mut self) -> &mut AgentMessage {
        self.pending_message.get_or_insert_default()
    }

    fn flush_pending_message(&mut self, cx: &mut Context<Self>) {
        let Some(mut message) = self.pending_message.take() else {
            return;
        };

        if message.content.is_empty() {
            return;
        }

        for content in &message.content {
            let AgentMessageContent::ToolUse(tool_use) = content else {
                continue;
            };

            if !message.tool_results.contains_key(&tool_use.id) {
                message.tool_results.insert(
                    tool_use.id.clone(),
                    LanguageModelToolResult {
                        tool_use_id: tool_use.id.clone(),
                        tool_name: tool_use.name.clone(),
                        is_error: true,
                        content: vec![LanguageModelToolResultContent::Text(
                            TOOL_CANCELED_MESSAGE.into(),
                        )],
                        output: None,
                    },
                );
            }
        }

        self.messages.push(Arc::new(Message::Agent(message)));
        self.updated_at = Utc::now();
        self.clear_summary();
        cx.notify()
    }

    pub(crate) fn build_completion_request(
        &self,
        completion_intent: CompletionIntent,
        cx: &App,
    ) -> Result<LanguageModelRequest> {
        let completion_intent =
            if self.is_subagent() && completion_intent == CompletionIntent::UserPrompt {
                CompletionIntent::Subagent
            } else {
                completion_intent
            };

        let model = self
            .model()
            .ok_or_else(|| anyhow!(NoModelConfiguredError))?;
        let tools = if let Some(turn) = self.running_turn.as_ref() {
            turn.tools
                .iter()
                .filter_map(|(tool_name, tool)| {
                    log::trace!("Including tool: {}", tool_name);
                    Some(LanguageModelRequestTool {
                        name: tool_name.to_string(),
                        description: tool.description().to_string(),
                        input_schema: tool.input_schema(model.tool_input_format()).log_err()?,
                        use_input_streaming: tool.supports_input_streaming(),
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        log::debug!("Building completion request");
        log::debug!("Completion intent: {:?}", completion_intent);

        let available_tools: Vec<_> = self
            .running_turn
            .as_ref()
            .map(|turn| turn.tools.keys().cloned().collect())
            .unwrap_or_default();

        log::debug!("Request includes {} tools", available_tools.len());
        let messages = self.build_request_messages(available_tools, cx);
        log::debug!("Request will include {} messages", messages.len());

        let request = LanguageModelRequest {
            thread_id: Some(self.id.to_string()),
            prompt_id: Some(self.prompt_id.to_string()),
            intent: Some(completion_intent),
            messages,
            tools,
            tool_choice: None,
            stop: Vec::new(),
            temperature: AgentSettings::temperature_for_model(model, cx),
            // Models that can't run with thinking disabled ignore the
            // toggle state, which may be stale from a previously selected
            // model that could.
            thinking_allowed: self.thinking_enabled || !model.supports_disabling_thinking(),
            thinking_effort: self.thinking_effort.clone(),
            speed: self.speed(),
        };

        log::debug!("Completion request built successfully");
        Ok(request)
    }

    fn enabled_tools(&self, cx: &App) -> BTreeMap<SharedString, Arc<dyn AnyAgentTool>> {
        let Some(model) = self.model.as_ref() else {
            return BTreeMap::new();
        };
        let Some(profile) = AgentSettings::get_global(cx).profiles.get(&self.profile_id) else {
            return BTreeMap::new();
        };
        fn truncate(tool_name: &SharedString) -> SharedString {
            if tool_name.len() > MAX_TOOL_NAME_LENGTH {
                let mut truncated = tool_name.to_string();
                truncated.truncate(MAX_TOOL_NAME_LENGTH);
                truncated.into()
            } else {
                tool_name.clone()
            }
        }

        // Terminal variants are configured by users under the canonical
        // `terminal` name. Expose the one matching the current sandbox state
        // to the model under that name.
        let use_sandboxed_terminal = sandboxing_enabled(cx);

        let mut tools = self
            .tools
            .iter()
            .filter_map(|(tool_name, tool)| {
                let terminal_variant = matches!(
                    tool_name.as_ref(),
                    TerminalTool::NAME | SandboxedTerminalTool::NAME
                );
                let profile_tool_name = if terminal_variant {
                    TerminalTool::NAME
                } else {
                    tool_name.as_ref()
                };

                if tool.supports_provider(&model.provider_id())
                    && profile.is_tool_enabled(profile_tool_name)
                {
                    match (tool_name.as_ref(), use_sandboxed_terminal) {
                        (TerminalTool::NAME, false) | (SandboxedTerminalTool::NAME, true) => {
                            Some((SharedString::from(TerminalTool::NAME), tool.clone()))
                        }
                        (TerminalTool::NAME | SandboxedTerminalTool::NAME, _) => None,
                        _ => Some((truncate(tool_name), tool.clone())),
                    }
                } else {
                    None
                }
            })
            .filter(|(tool_name, _)| crate::tools::tool_feature_flag_enabled(tool_name, cx))
            .collect::<BTreeMap<_, _>>();

        // When no project worktree is open, remove tools that depend on one.
        // The model can still use terminal, fetch, web_search, skill, etc.
        if self.project.read(cx).visible_worktrees(cx).next().is_none() {
            const PROJECT_TOOLS: &[&str] = &[
                "read_file",
                "write_file",
                "edit_file",
                "create_directory",
                "delete_path",
                "move_path",
                "copy_path",
                "grep",
                "find_path",
                "list_directory",
                "search_semantic",
                "go_to_definition",
                "find_references",
                "rename_symbol",
                "diagnostics",
                "get_code_actions",
                "apply_code_action",
                "git_status",
                "git_commit",
                "git_push",
                "git_branch",
                "git_log",
            ];
            tools.retain(|name, _| !PROJECT_TOOLS.contains(&name.as_str()));
        }

        let mut context_server_tools = Vec::new();
        let mut seen_tools = tools.keys().cloned().collect::<HashSet<_>>();
        let mut duplicate_tool_names = HashSet::default();
        for (server_id, server_tools) in self.context_server_registry.read(cx).servers() {
            for (tool_name, tool) in server_tools {
                if profile.is_context_server_tool_enabled(&server_id.0, &tool_name) {
                    let tool_name = truncate(tool_name);
                    if !seen_tools.insert(tool_name.clone()) {
                        duplicate_tool_names.insert(tool_name.clone());
                    }
                    context_server_tools.push((server_id.clone(), tool_name, tool.clone()));
                }
            }
        }

        // When there are duplicate tool names, disambiguate by prefixing them
        // with the server ID (converted to snake_case for API compatibility).
        // In the rare case there isn't enough space for the disambiguated tool
        // name, keep only the last tool with this name.
        for (server_id, tool_name, tool) in context_server_tools {
            if duplicate_tool_names.contains(&tool_name) {
                let available = MAX_TOOL_NAME_LENGTH.saturating_sub(tool_name.len());
                if available >= 2 {
                    let mut disambiguated = server_id.0.to_snake_case();
                    disambiguated.truncate(available - 1);
                    disambiguated.push('_');
                    disambiguated.push_str(&tool_name);
                    tools.insert(disambiguated.into(), tool.clone());
                } else {
                    tools.insert(tool_name, tool.clone());
                }
            } else {
                tools.insert(tool_name, tool.clone());
            }
        }

        tools
    }

    fn refresh_turn_tools(&mut self, cx: &App) {
        let tools = self.enabled_tools(cx);
        if let Some(turn) = self.running_turn.as_mut() {
            turn.tools = tools;
        }
    }

    fn tool(&self, name: &str) -> Option<Arc<dyn AnyAgentTool>> {
        self.running_turn.as_ref()?.tools.get(name).cloned()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.running_turn
            .as_ref()
            .is_some_and(|turn| turn.tools.contains_key(name))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_registered_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub(crate) fn register_running_subagent(&mut self, subagent: WeakEntity<Thread>) {
        self.running_subagents.push(subagent);
    }

    pub(crate) fn unregister_running_subagent(
        &mut self,
        subagent_session_id: &acp::SessionId,
        cx: &App,
    ) {
        self.running_subagents.retain(|s| {
            s.upgrade()
                .map_or(false, |s| s.read(cx).id() != subagent_session_id)
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn running_subagent_ids(&self, cx: &App) -> Vec<acp::SessionId> {
        self.running_subagents
            .iter()
            .filter_map(|s| s.upgrade().map(|s| s.read(cx).id().clone()))
            .collect()
    }

    pub fn is_subagent(&self) -> bool {
        self.subagent_context.is_some()
    }

    pub fn parent_thread_id(&self) -> Option<acp::SessionId> {
        self.subagent_context
            .as_ref()
            .map(|c| c.parent_thread_id.clone())
    }

    pub fn depth(&self) -> u8 {
        self.subagent_context.as_ref().map(|c| c.depth).unwrap_or(0)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_subagent_context(&mut self, context: SubagentContext) {
        self.subagent_context = Some(context);
    }

    pub fn is_turn_complete(&self) -> bool {
        self.running_turn.is_none()
    }

    fn build_request_messages(
        &self,
        available_tools: Vec<SharedString>,
        cx: &App,
    ) -> Vec<LanguageModelRequestMessage> {
        let mut messages =
            self.build_request_messages_until(available_tools, self.messages.len(), cx);

        if let Some(message) = self.pending_message.as_ref() {
            messages.extend(message.to_request());
        }

        messages
    }

    fn build_request_messages_until(
        &self,
        available_tools: Vec<SharedString>,
        end_ix: usize,
        cx: &App,
    ) -> Vec<LanguageModelRequestMessage> {
        let end_ix = end_ix.min(self.messages.len());
        log::trace!("Building request messages from {} thread messages", end_ix);

        let user_agents_md = UserAgentsMd::global(cx).and_then(|s| s.content().cloned());
        let system_prompt = SystemPromptTemplate {
            project: self.project_context.read(cx),
            available_tools,
            model_name: self.model.as_ref().map(|m| m.name().0.to_string()),
            date: Local::now().format("%Y-%m-%d").to_string(),
            user_agents_md,
            sandboxing: crate::sandboxing::sandboxing_enabled(cx),
            require_verification: self.require_verification,
        }
        .render(&self.templates)
        .context("failed to build system prompt")
        .expect("Invalid template");
        let mut messages = vec![LanguageModelRequestMessage {
            role: Role::System,
            content: vec![system_prompt.into()],
            cache: false,
            reasoning_details: None,
        }];
        self.extend_request_history_until(&mut messages, end_ix);

        if let Some(last_message) = messages.last_mut() {
            last_message.cache = true;
        }

        messages
    }

    fn extend_request_history_until(
        &self,
        messages: &mut Vec<LanguageModelRequestMessage>,
        end_ix: usize,
    ) {
        let Some(compaction_ix) = self.latest_compaction_message_ix_before(end_ix) else {
            for message in &self.messages[..end_ix] {
                messages.extend(message.to_request());
            }
            return;
        };

        if matches!(
            &*self.messages[compaction_ix],
            Message::Compaction(CompactionInfo::Summary(_))
        ) {
            messages.extend(self.retained_user_request_messages_before(compaction_ix));
        }

        for message in &self.messages[compaction_ix..end_ix] {
            messages.extend(message.to_request());
        }
    }

    fn latest_compaction_message_ix_before(&self, end_ix: usize) -> Option<usize> {
        self.messages[..end_ix]
            .iter()
            .rposition(|message| matches!(&**message, Message::Compaction(_)))
    }

    /// Captures the data for an `"Agent Compaction Completed"` telemetry event
    /// at the moment a compaction starts. Returns `None` if there's no model.
    fn build_compaction_telemetry(
        &self,
        trigger: &'static str,
        cx: &App,
    ) -> Option<CompactionTelemetry> {
        let model = self.model.as_ref()?;
        let auto_compact = AgentSettings::get_global(cx).auto_compact;
        let max_tokens = model.max_token_count();
        let tokens_before = self
            .latest_request_token_usage()
            .map(|usage| total_input_tokens(usage).saturating_add(usage.output_tokens));
        Some(CompactionTelemetry {
            trigger,
            thread_id: self.id.to_string(),
            parent_thread_id: self.parent_thread_id().map(|id| id.to_string()),
            prompt_id: self.prompt_id.to_string(),
            model: model.telemetry_id(),
            model_provider: model.provider_id().to_string(),
            thinking_effort: self.thinking_effort.clone(),
            max_tokens,
            tokens_before,
            auto_compact_enabled: auto_compact.enabled,
            auto_compact_threshold: auto_compact.threshold.to_string(),
            auto_compact_threshold_tokens: auto_compact_threshold_token_count(
                auto_compact.threshold,
                max_tokens,
            ),
            retries: 0,
        })
    }

    /// Emits a pending compaction telemetry event for a non-success outcome
    /// (`"failed"` or `"canceled"`), with no post-compaction token count. A
    /// no-op if no compaction telemetry is pending.
    fn emit_compaction_telemetry_outcome(&mut self, status: &'static str, error: Option<String>) {
        if let Some(telemetry) = self.pending_compaction_telemetry.take() {
            telemetry.emit(status, error, None);
        }
    }

    fn compaction_message_target_ix(&self, cx: &App) -> Option<usize> {
        let auto_compact = AgentSettings::get_global(cx).auto_compact;
        if !auto_compact.enabled {
            return None;
        }

        let model = self.model.as_ref()?;
        let max_token_count = model.max_token_count();
        // Models with a small context window don't leave enough headroom for a
        // compaction pass; the UI warns the user about the token limit instead.
        if max_token_count < MIN_COMPACTION_CONTEXT_WINDOW {
            return None;
        }
        let (usage_ix, usage) = {
            let this = &self;
            this.messages
                .iter()
                .enumerate()
                .rev()
                .find_map(|(ix, message)| {
                    let Message::User(user_message) = &**message else {
                        return None;
                    };
                    this.request_token_usage
                        .get(&user_message.id)
                        .copied()
                        .map(|usage| (ix, usage))
                })
        }?;
        if self
            .latest_compaction_message_ix_before(self.messages.len())
            .is_some_and(|compaction_ix| compaction_ix > usage_ix)
        {
            return None;
        }

        let active_tokens = total_input_tokens(usage).saturating_add(usage.output_tokens);
        let compaction_threshold =
            auto_compact_threshold_token_count(auto_compact.threshold, max_token_count);
        if active_tokens < compaction_threshold {
            return None;
        }

        let insertion_ix = match self.messages.last() {
            Some(message)
                if matches!(
                    &**message,
                    Message::User(UserMessage { id, .. }) if !self.request_token_usage.contains_key(id)
                ) =>
            {
                self.messages.len().saturating_sub(1)
            }
            _ => self.messages.len(),
        };
        Some(insertion_ix)
    }

    /// Insertion point for a manually-triggered compaction.
    /// Returns `None` only when there is nothing to summarize (no messages, or the thread already ends in a compaction).
    fn forced_compaction_target_ix(&self) -> Option<usize> {
        if matches!(
            self.messages.last().map(|message| &**message),
            None | Some(Message::Compaction(_))
        ) {
            return None;
        }
        Some(self.messages.len())
    }

    fn build_compaction_request(
        &self,
        insertion_ix: usize,
        model: &Arc<dyn LanguageModel>,
        cx: &App,
    ) -> LanguageModelRequest {
        let mut request = LanguageModelRequest {
            thread_id: Some(self.id.to_string()),
            prompt_id: Some(self.prompt_id.to_string()),
            intent: Some(CompletionIntent::ThreadContextSummarization),
            temperature: AgentSettings::temperature_for_model(model, cx),
            messages: self.build_request_messages_until(Vec::new(), insertion_ix, cx),
            ..Default::default()
        };

        request.messages.push(LanguageModelRequestMessage {
            role: Role::User,
            content: vec![COMPACTION_PROMPT.into()],
            cache: false,
            reasoning_details: None,
        });

        request
    }

    fn retained_user_request_messages_before(
        &self,
        compaction_ix: usize,
    ) -> Vec<LanguageModelRequestMessage> {
        let mut remaining_bytes = COMPACTION_RETAINED_USER_MESSAGES_BYTE_BUDGET;
        let mut retained_messages = Vec::new();

        for message in self.messages[..compaction_ix].iter().rev() {
            let Message::User(user_message) = &**message else {
                continue;
            };
            if user_message.content.is_empty() {
                continue;
            }

            let request_message = user_message.to_request();
            let byte_count = user_message_byte_len(&request_message);
            if let Some(bytes) = remaining_bytes.checked_sub(byte_count) {
                remaining_bytes = bytes;
                retained_messages.push(request_message);
            } else {
                if remaining_bytes > 0
                    && let Some(request_message) =
                        truncate_user_message_to_byte_budget(request_message, remaining_bytes)
                {
                    retained_messages.push(request_message);
                }
                break;
            }
        }

        retained_messages.reverse();
        retained_messages
    }

    pub fn to_markdown(&self) -> String {
        let mut markdown = messages_to_markdown(&self.messages);

        if let Some(message) = self.pending_message.as_ref() {
            markdown.push_str("\n## Assistant\n\n");
            markdown.push_str(&message.to_markdown());
        }

        markdown
    }

    fn advance_prompt_id(&mut self) {
        self.prompt_id = PromptId::new();
    }

    fn retry_strategy_for(error: &LanguageModelCompletionError) -> Option<RetryStrategy> {
        use LanguageModelCompletionError::*;
        use http_client::StatusCode;

        // General strategy here:
        // - If retrying won't help (e.g. invalid API key or payload too large), return None so we don't retry at all.
        // - If it's a time-based issue (e.g. server overloaded, rate limit exceeded), retry up to 4 times with exponential backoff.
        // - If it's an issue that *might* be fixed by retrying (e.g. internal server error), retry up to 3 times.
        match error {
            HttpResponseError {
                status_code: StatusCode::TOO_MANY_REQUESTS,
                ..
            } => Some(RetryStrategy::ExponentialBackoff {
                initial_delay: BASE_RETRY_DELAY,
                max_attempts: MAX_RETRY_ATTEMPTS,
            }),
            ServerOverloaded { retry_after, .. } | RateLimitExceeded { retry_after, .. } => {
                Some(RetryStrategy::Fixed {
                    delay: retry_after.unwrap_or(BASE_RETRY_DELAY),
                    max_attempts: MAX_RETRY_ATTEMPTS,
                })
            }
            UpstreamProviderError {
                status,
                retry_after,
                ..
            } => match *status {
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => {
                    Some(RetryStrategy::Fixed {
                        delay: retry_after.unwrap_or(BASE_RETRY_DELAY),
                        max_attempts: MAX_RETRY_ATTEMPTS,
                    })
                }
                StatusCode::INTERNAL_SERVER_ERROR => Some(RetryStrategy::Fixed {
                    delay: retry_after.unwrap_or(BASE_RETRY_DELAY),
                    // Internal Server Error could be anything, retry up to 3 times.
                    max_attempts: 3,
                }),
                status => {
                    // There is no StatusCode variant for the unofficial HTTP 529 ("The service is overloaded"),
                    // but we frequently get them in practice. See https://http.dev/529
                    if status.as_u16() == 529 {
                        Some(RetryStrategy::Fixed {
                            delay: retry_after.unwrap_or(BASE_RETRY_DELAY),
                            max_attempts: MAX_RETRY_ATTEMPTS,
                        })
                    } else {
                        Some(RetryStrategy::Fixed {
                            delay: retry_after.unwrap_or(BASE_RETRY_DELAY),
                            max_attempts: 2,
                        })
                    }
                }
            },
            ApiInternalServerError { .. } => Some(RetryStrategy::Fixed {
                delay: BASE_RETRY_DELAY,
                max_attempts: 3,
            }),
            ApiReadResponseError { .. }
            | HttpSend { .. }
            | DeserializeResponse { .. }
            | BadRequestFormat { .. } => Some(RetryStrategy::Fixed {
                delay: BASE_RETRY_DELAY,
                max_attempts: 3,
            }),
            // Retrying these errors definitely shouldn't help.
            HttpResponseError {
                status_code:
                    StatusCode::PAYLOAD_TOO_LARGE | StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED,
                ..
            }
            | AuthenticationError { .. }
            | PermissionError { .. }
            | NoApiKey { .. }
            | ApiEndpointNotFound { .. }
            | PromptTooLarge { .. } => None,
            // These errors might be transient, so retry them
            SerializeRequest { .. } | BuildRequestBody { .. } | StreamEndedUnexpectedly { .. } => {
                Some(RetryStrategy::Fixed {
                    delay: BASE_RETRY_DELAY,
                    max_attempts: 1,
                })
            }
            // Retry all other 4xx and 5xx errors once.
            HttpResponseError { status_code, .. }
                if status_code.is_client_error() || status_code.is_server_error() =>
            {
                Some(RetryStrategy::Fixed {
                    delay: BASE_RETRY_DELAY,
                    max_attempts: 3,
                })
            }
            // Retrying won't help for Payment Required errors.
            PaymentRequired => None,
            // Retrying won't help until the user consents to data retention
            // or switches models.
            DataRetentionConsentRequired { .. } => None,
            // Conservatively assume that any other errors are non-retryable
            HttpResponseError { .. } | Other(..) => Some(RetryStrategy::Fixed {
                delay: BASE_RETRY_DELAY,
                max_attempts: 2,
            }),
        }
    }
}
