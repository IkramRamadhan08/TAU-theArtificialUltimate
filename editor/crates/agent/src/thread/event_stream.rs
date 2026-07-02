use super::thread_types::{
    AgentPlan, PlanStepStatus, ThreadEvent, ToolCallAuthorization,
    ToolPermissionContext, auto_resolve_permission_outcome,
};
use super::user_message::UserMessage;
use crate::{
    ToolPermissionDecision,
    decide_permission_from_settings,
};
use crate::sandboxing::{SandboxRequest, ThreadSandboxGrants};
use acp_thread;
use agent_client_protocol::schema as acp;
use agent_settings::AgentSettings;
use anyhow::{Result, anyhow};
use fs::Fs;
use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::FutureExt;
use gpui::{App, AsyncApp, Entity, SharedString, Task};
use language_model::LanguageModelToolUseId;
use settings::{
    Settings, SettingsStore, ToolPermissionMode, update_settings_file,
};
use std::sync::Arc;
use std::cell::RefCell;
use std::rc::Rc;
#[derive(Clone)]
pub(crate) struct ThreadEventStream(pub(crate) mpsc::UnboundedSender<Result<ThreadEvent>>);

impl ThreadEventStream {
    pub(crate) fn new(tx: mpsc::UnboundedSender<Result<ThreadEvent>>) -> Self {
        Self(tx)
    }
}

impl ThreadEventStream {
    pub(crate) fn send_user_message(&self, message: &UserMessage) {
        self.0
            .unbounded_send(Ok(ThreadEvent::UserMessage(message.clone())))
            .ok();
    }

    pub(crate) fn send_text(&self, text: &str) {
        self.0
            .unbounded_send(Ok(ThreadEvent::AgentText(text.to_string())))
            .ok();
    }

    pub(crate) fn send_thinking(&self, text: &str) {
        self.0
            .unbounded_send(Ok(ThreadEvent::AgentThinking(text.to_string())))
            .ok();
    }

    pub(crate) fn send_tool_call(
        &self,
        id: &LanguageModelToolUseId,
        tool_name: &str,
        title: SharedString,
        kind: acp::ToolKind,
        input: serde_json::Value,
    ) {
        self.0
            .unbounded_send(Ok(ThreadEvent::ToolCall(Self::initial_tool_call(
                id,
                tool_name,
                title.to_string(),
                kind,
                input,
            ))))
            .ok();
    }

    pub(crate) fn initial_tool_call(
        id: &LanguageModelToolUseId,
        tool_name: &str,
        title: String,
        kind: acp::ToolKind,
        input: serde_json::Value,
    ) -> acp::ToolCall {
        acp::ToolCall::new(id.to_string(), title)
            .kind(kind)
            .raw_input(input)
            .meta(acp_thread::meta_with_tool_name(tool_name))
    }

    pub(crate) fn update_tool_call_fields(
        &self,
        tool_use_id: &LanguageModelToolUseId,
        fields: acp::ToolCallUpdateFields,
        meta: Option<acp::Meta>,
    ) {
        self.0
            .unbounded_send(Ok(ThreadEvent::ToolCallUpdate(
                acp::ToolCallUpdate::new(tool_use_id.to_string(), fields)
                    .meta(meta)
                    .into(),
            )))
            .ok();
    }

    pub(crate) fn resolve_tool_call_authorization(
        &self,
        tool_use_id: &LanguageModelToolUseId,
        outcome: acp_thread::SelectedPermissionOutcome,
    ) {
        self.0
            .unbounded_send(Ok(ThreadEvent::ToolCallAuthorizationResolved {
                tool_call_id: acp::ToolCallId::new(tool_use_id.to_string()),
                outcome,
            }))
            .ok();
    }

    pub(crate) fn send_retry(&self, status: acp_thread::RetryStatus) {
        self.0.unbounded_send(Ok(ThreadEvent::Retry(status))).ok();
    }

    pub(crate) fn send_context_compaction(
        &self,
        id: acp_thread::ContextCompactionId,
        status: acp_thread::ContextCompactionStatus,
    ) {
        self.0
            .unbounded_send(Ok(ThreadEvent::ContextCompaction(
                acp_thread::ContextCompaction {
                    id,
                    status,
                    summary: None,
                },
            )))
            .ok();
    }

    pub(crate) fn send_context_compaction_update(
        &self,
        id: acp_thread::ContextCompactionId,
        summary_delta: &str,
    ) {
        self.0
            .unbounded_send(Ok(ThreadEvent::ContextCompactionUpdate(
                acp_thread::ContextCompactionUpdate {
                    id,
                    summary_delta: summary_delta.to_string(),
                    status: None,
                },
            )))
            .ok();
    }

    pub(crate) fn update_context_compaction_status(
        &self,
        id: acp_thread::ContextCompactionId,
        status: acp_thread::ContextCompactionStatus,
    ) {
        self.0
            .unbounded_send(Ok(ThreadEvent::ContextCompactionUpdate(
                acp_thread::ContextCompactionUpdate {
                    id,
                    summary_delta: String::new(),
                    status: Some(status),
                },
            )))
            .ok();
    }

    pub(crate) fn send_stop(&self, reason: acp::StopReason) {
        self.0.unbounded_send(Ok(ThreadEvent::Stop(reason))).ok();
    }

    pub(crate) fn send_canceled(&self) {
        self.0
            .unbounded_send(Ok(ThreadEvent::Stop(acp::StopReason::Cancelled)))
            .ok();
    }

    pub(crate) fn send_error(&self, error: impl Into<anyhow::Error>) {
        self.0.unbounded_send(Err(error.into())).ok();
    }

    pub(crate) fn send_plan(&self, plan: AgentPlan) {
        self.0.unbounded_send(Ok(ThreadEvent::Plan(plan))).ok();
    }

    pub(crate) fn send_plan_step_update(&self, step_description: String, status: PlanStepStatus) {
        self.0
            .unbounded_send(Ok(ThreadEvent::PlanStepUpdate(
                step_description,
                status,
            )))
            .ok();
    }
}

#[derive(Clone)]
pub struct ToolCallEventStream {
    tool_use_id: LanguageModelToolUseId,
    stream: ThreadEventStream,
    fs: Option<Arc<dyn Fs>>,
    cancellation_rx: watch::Receiver<bool>,
    /// Shared, thread-scoped sandbox grants (see [`Thread::sandbox_grants`]).
    sandbox_grants: Rc<RefCell<ThreadSandboxGrants>>,
    /// When true, tool calls show a confirmation dialog instead of auto-executing.
    require_verification: bool,
}

impl ToolCallEventStream {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> (Self, ToolCallEventStreamReceiver) {
        let (stream, receiver, _cancellation_tx) = Self::test_with_cancellation();
        (stream, receiver)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_with_cancellation() -> (Self, ToolCallEventStreamReceiver, watch::Sender<bool>) {
        let (events_tx, events_rx) = mpsc::unbounded::<Result<ThreadEvent>>();
        let (cancellation_tx, cancellation_rx) = watch::channel(false);

        let stream = ToolCallEventStream::new(
            "test_id".into(),
            ThreadEventStream(events_tx),
            None,
            cancellation_rx,
            Rc::new(RefCell::new(ThreadSandboxGrants::default())),
            false,
        );

        (
            stream,
            ToolCallEventStreamReceiver(events_rx),
            cancellation_tx,
        )
    }

    /// Signal cancellation for this event stream. Only available in tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn signal_cancellation_with_sender(cancellation_tx: &mut watch::Sender<bool>) {
        cancellation_tx.send(true).ok();
    }

    pub(crate) fn new(
        tool_use_id: LanguageModelToolUseId,
        stream: ThreadEventStream,
        fs: Option<Arc<dyn Fs>>,
        cancellation_rx: watch::Receiver<bool>,
        sandbox_grants: Rc<RefCell<ThreadSandboxGrants>>,
        require_verification: bool,
    ) -> Self {
        Self {
            tool_use_id,
            stream,
            fs,
            cancellation_rx,
            sandbox_grants,
            require_verification,
        }
    }

    /// Returns a future that resolves when the user cancels the tool call.
    /// Tools should select on this alongside their main work to detect user cancellation.
    pub fn cancelled_by_user(&self) -> impl std::future::Future<Output = ()> + '_ {
        let mut rx = self.cancellation_rx.clone();
        async move {
            loop {
                if *rx.borrow() {
                    return;
                }
                if rx.changed().await.is_err() {
                    // Sender dropped, will never be cancelled
                    std::future::pending::<()>().await;
                }
            }
        }
    }

    /// Returns true if the user has cancelled this tool call.
    /// This is useful for checking cancellation state after an operation completes,
    /// to determine if the completion was due to user cancellation.
    pub fn was_cancelled_by_user(&self) -> bool {
        *self.cancellation_rx.clone().borrow()
    }

    pub fn tool_use_id(&self) -> &LanguageModelToolUseId {
        &self.tool_use_id
    }

    pub fn update_fields(&self, fields: acp::ToolCallUpdateFields) {
        self.stream
            .update_tool_call_fields(&self.tool_use_id, fields, None);
    }

    pub fn update_fields_with_meta(
        &self,
        fields: acp::ToolCallUpdateFields,
        meta: Option<acp::Meta>,
    ) {
        self.stream
            .update_tool_call_fields(&self.tool_use_id, fields, meta);
    }

    pub fn resolve_authorization(&self, outcome: acp_thread::SelectedPermissionOutcome) {
        self.stream
            .resolve_tool_call_authorization(&self.tool_use_id, outcome);
    }

    pub fn update_diff(&self, diff: Entity<acp_thread::Diff>) {
        self.stream
            .0
            .unbounded_send(Ok(ThreadEvent::ToolCallUpdate(
                acp_thread::ToolCallUpdateDiff {
                    id: acp::ToolCallId::new(self.tool_use_id.to_string()),
                    diff,
                }
                .into(),
            )))
            .ok();
    }

    pub fn subagent_spawned(&self, id: acp::SessionId) {
        self.stream
            .0
            .unbounded_send(Ok(ThreadEvent::SubagentSpawned(id)))
            .ok();
    }

    /// Authorize a third-party tool (e.g., MCP tool from a context server).
    ///
    /// Unlike built-in tools, third-party tools don't support pattern-based permissions.
    /// They only support `default` (allow/deny/confirm) per tool.
    ///
    /// Uses the dropdown authorization flow with two granularities:
    /// - "Always for <display_name> MCP tool" → sets `tools.<tool_id>.default = "allow"` or "deny"
    /// - "Only this time" → allow/deny once
    pub fn authorize_third_party_tool(
        &self,
        title: impl Into<String>,
        tool_id: String,
        display_name: String,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let title = title.into();
        let options = acp_thread::PermissionOptions::Dropdown(vec![
            acp_thread::PermissionOptionChoice {
                allow: acp::PermissionOption::new(
                    acp::PermissionOptionId::new(format!("always_allow_mcp:{tool_id}")),
                    format!("Always for {display_name} MCP tool"),
                    acp::PermissionOptionKind::AllowAlways,
                ),
                deny: acp::PermissionOption::new(
                    acp::PermissionOptionId::new(format!("always_deny_mcp:{tool_id}")),
                    format!("Always for {display_name} MCP tool"),
                    acp::PermissionOptionKind::RejectAlways,
                ),
                sub_patterns: vec![],
            },
            acp_thread::PermissionOptionChoice {
                allow: acp::PermissionOption::new(
                    acp::PermissionOptionId::new("allow"),
                    "Only this time",
                    acp::PermissionOptionKind::AllowOnce,
                ),
                deny: acp::PermissionOption::new(
                    acp::PermissionOptionId::new("deny"),
                    "Only this time",
                    acp::PermissionOptionKind::RejectOnce,
                ),
                sub_patterns: vec![],
            },
        ]);

        // MCP tools are gated only by tool id (no per-input pattern
        // matching), so we pass a single empty input value just to satisfy
        // `decide_permission_from_settings`' signature.
        let require_verification = self.require_verification;
        let check_settings: Box<dyn Fn(&App) -> ToolPermissionDecision> =
            Box::new(move |cx: &App| {
                let settings = agent_settings::AgentSettings::get_global(cx);
                let decision =
                    decide_permission_from_settings(&tool_id, &[String::new()], settings);
                if require_verification {
                    match decision {
                        ToolPermissionDecision::Allow => ToolPermissionDecision::Confirm,
                        other => other,
                    }
                } else {
                    decision
                }
            });

        self.run_authorization_loop(title, options, None, Some(check_settings), cx)
    }

    /// Gate a tool call on user permission, driven by the agent's
    /// tool-permission settings.
    ///
    /// Evaluates the current settings up-front: returns `Ok(())` immediately
    /// if the tool is already allowed, an error if it is denied, and
    /// otherwise prompts the user for a decision. While a prompt is pending,
    /// a subscription to `SettingsStore` watches for changes (for example,
    /// when the user clicks "Always for …" on a sibling tool call and the
    /// new rule becomes globally visible). When settings change, the current
    /// prompt is dismissed and the decision is re-evaluated. This closes the
    /// gap where an "Always for …" decision on one pending tool call would
    /// not propagate to other pending tool calls in the same turn or in
    /// subagent turns.
    ///
    /// For authorizations that must always prompt regardless of settings
    /// (e.g. symlink-escape confirmations, sensitive settings-file edits),
    /// use [`Self::prompt`] instead.
    pub fn authorize(
        &self,
        title: impl Into<String>,
        context: ToolPermissionContext,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let title = title.into();
        let options = context.build_permission_options();

        let tool_name = context.tool_name.clone();
        let input_values = context.input_values.clone();
        let require_verification = self.require_verification;
        let check_settings: Box<dyn Fn(&App) -> ToolPermissionDecision> =
            Box::new(move |cx: &App| {
                let decision = decide_permission_from_settings(
                    &tool_name,
                    &input_values,
                    agent_settings::AgentSettings::get_global(cx),
                );
                if require_verification {
                    match decision {
                        ToolPermissionDecision::Allow => ToolPermissionDecision::Confirm,
                        other => other,
                    }
                } else {
                    decision
                }
            });

        self.run_authorization_loop(title, options, Some(context), Some(check_settings), cx)
    }

    /// Like [`Self::authorize`], but always prompts the user without
    /// consulting settings. Use this for authorizations that must be
    /// confirmed even when the user has configured `always_allow` rules —
    /// for example, symlink-escape confirmations or edits that target
    /// sensitive settings files.
    pub fn authorize_always_prompt(
        &self,
        title: impl Into<String>,
        context: ToolPermissionContext,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let title = title.into();
        let options = context.build_permission_options();
        self.run_authorization_loop(title, options, Some(context), None, cx)
    }

    /// Gate a sandbox *escalation* (network access, per-path writes, or full
    /// filesystem write access) on user approval.
    ///
    /// Offers the user three grant lifetimes — "once", "for the rest of this
    /// thread", and "always". Thread grants live in the shared, in-memory
    /// [`ThreadSandboxGrants`]. Always grants are persisted in agent settings
    /// and are also observed while a prompt is pending, matching the
    /// settings-driven authorization flow for regular tools.
    pub(crate) fn authorize_sandbox(
        &self,
        title: impl Into<String>,
        command: Option<String>,
        request: SandboxRequest,
        cx: &mut App,
    ) -> Task<Result<()>> {
        if Self::sandbox_request_covered_by_grants(&request, &self.sandbox_grants, cx) {
            return Task::ready(Ok(()));
        }

        let title = title.into();
        let sandbox_authorization_details = acp_thread::SandboxAuthorizationDetails {
            command,
            network: request.network,
            allow_fs_write_all: request.allow_fs_write_all,
            unsandboxed: request.unsandboxed,
            write_paths: request.write_paths.clone(),
        };
        let options = acp_thread::PermissionOptions::Flat(vec![
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(acp_thread::SandboxPermission::AllowOnce.as_id()),
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(acp_thread::SandboxPermission::AllowThread.as_id()),
                "Allow for this thread",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(acp_thread::SandboxPermission::AllowAlways.as_id()),
                "Allow always",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new(acp_thread::SandboxPermission::Deny.as_id()),
                "Deny",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ]);

        let fs = self.fs.clone();
        let stream = self.stream.clone();
        let tool_use_id = self.tool_use_id.clone();
        let sandbox_grants = self.sandbox_grants.clone();
        let auto_allow_outcome = match auto_resolve_permission_outcome(&options, true) {
            Ok(outcome) => outcome,
            Err(error) => return Task::ready(Err(error)),
        };
        cx.spawn(async move |cx| {
            let (response_tx, mut response_rx) = oneshot::channel();
            if let Err(error) = stream
                .0
                .unbounded_send(Ok(ThreadEvent::ToolCallAuthorization(
                    ToolCallAuthorization {
                        tool_call: acp::ToolCallUpdate::new(
                            tool_use_id.to_string(),
                            acp::ToolCallUpdateFields::new().title(title),
                        )
                        .meta(acp_thread::meta_with_sandbox_authorization(
                            sandbox_authorization_details,
                        )),
                        options,
                        response: response_tx,
                        context: None,
                        kind: acp_thread::AuthorizationKind::PermissionGrant,
                    },
                )))
            {
                log::error!("Failed to send sandbox authorization: {error}");
                return Err(anyhow!("Failed to send sandbox authorization: {error}"));
            }

            let (mut settings_tx, mut settings_rx) = watch::channel(());
            let _settings_subscription = cx.update(|cx| {
                cx.observe_global::<SettingsStore>(move |_cx| {
                    settings_tx.send(()).ok();
                })
            });

            loop {
                let settings_changed = async {
                    if settings_rx.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                };
                futures::select_biased! {
                    outcome = (&mut response_rx).fuse() => {
                        let outcome = outcome
                            .map_err(|_| anyhow!("authorization channel closed"))?;
                        return Self::handle_sandbox_permission_outcome(
                            &outcome,
                            &request,
                            sandbox_grants.clone(),
                            fs.clone(),
                            cx,
                        );
                    }
                    _ = settings_changed.fuse() => {
                        if cx.update(|cx| Self::sandbox_request_covered_by_grants(
                            &request,
                            &sandbox_grants,
                            cx,
                        )) {
                            drop(response_rx);
                            stream.resolve_tool_call_authorization(
                                &tool_use_id,
                                auto_allow_outcome.clone(),
                            );
                            return Ok(());
                        }
                    }
                }
            }
        })
    }

    pub(crate) fn sandbox_request_covered_by_grants(
        request: &SandboxRequest,
        sandbox_grants: &Rc<RefCell<ThreadSandboxGrants>>,
        cx: &App,
    ) -> bool {
        let settings = AgentSettings::get_global(cx);
        sandbox_grants
            .borrow()
            .covers_with_persistent(request, &settings.sandbox_permissions)
    }

    pub(crate) fn handle_sandbox_permission_outcome(
        outcome: &acp_thread::SelectedPermissionOutcome,
        request: &SandboxRequest,
        sandbox_grants: Rc<RefCell<ThreadSandboxGrants>>,
        fs: Option<Arc<dyn Fs>>,
        cx: &AsyncApp,
    ) -> Result<()> {
        debug_assert!(
            outcome.params.is_none(),
            "unexpected params for sandbox permission"
        );

        match acp_thread::SandboxPermission::from_id(outcome.option_id.0.as_ref()) {
            Some(acp_thread::SandboxPermission::AllowOnce) => Ok(()),
            Some(acp_thread::SandboxPermission::AllowThread) => {
                sandbox_grants.borrow_mut().record(request);
                Ok(())
            }
            Some(acp_thread::SandboxPermission::AllowAlways) => {
                sandbox_grants.borrow_mut().record(request);
                Self::persist_sandbox_always_permission(request, fs, cx);
                Ok(())
            }
            Some(acp_thread::SandboxPermission::Deny) => {
                Err(anyhow!("Permission to run tool denied by user"))
            }
            None => {
                let other = outcome.option_id.0.as_ref();
                debug_assert!(false, "unexpected sandbox permission option_id: {other}");
                Err(anyhow!("Permission to run tool denied by user"))
            }
        }
    }

    pub(crate) fn persist_sandbox_always_permission(
        request: &SandboxRequest,
        fs: Option<Arc<dyn Fs>>,
        cx: &AsyncApp,
    ) {
        let Some(fs) = fs else {
            log::error!(
                "Cannot persist \"allow always\" sandbox permission: no filesystem available"
            );
            return;
        };

        let request = request.clone();
        cx.update(|cx| {
            update_settings_file(fs, cx, move |settings, _| {
                let agent = settings.agent.get_or_insert_default();
                if request.network {
                    agent.allow_sandbox_network();
                }
                if request.allow_fs_write_all {
                    agent.allow_sandbox_fs_write_all();
                }
                if request.unsandboxed {
                    agent.allow_sandbox_unsandboxed();
                }
                for path in request.write_paths {
                    agent.add_sandbox_write_path(path);
                }
            });
        });
    }

    /// The sandbox permissions to actually enforce for a command: the union
    /// of this command's `request`, everything granted "for the rest of the
    /// conversation", and persistent "allow always" sandbox grants.
    ///
    /// Callers must apply this to the enforced sandbox policy (rather than
    /// the raw `request`) so standing grants keep working for later commands
    /// that write to a previously approved path without re-requesting it.
    pub(crate) fn effective_sandbox_request(
        &self,
        request: &SandboxRequest,
        persistent: &agent_settings::SandboxPermissions,
    ) -> SandboxRequest {
        self.sandbox_grants
            .borrow()
            .effective_with_persistent(request, persistent)
    }

    /// Prompts the user to choose between an explicit set of actions and
    /// returns the chosen `option_id`.
    ///
    /// Unlike [`Self::authorize`] / [`Self::authorize_always_prompt`], this
    /// does not interpret the user's choice as a permission grant — callers
    /// are responsible for handling each `option_id` explicitly. Use this
    /// when a tool needs the user to pick between several side-effecting
    /// actions (for example, "Save" vs "Discard" for a dirty buffer).
    pub fn prompt_for_decision(
        &self,
        title: Option<String>,
        message: Option<String>,
        options: Vec<acp::PermissionOption>,
        cx: &mut App,
    ) -> Task<Result<acp::PermissionOptionId>> {
        let options = acp_thread::PermissionOptions::Flat(options);
        let stream = self.stream.clone();
        let tool_use_id = self.tool_use_id.clone();
        cx.spawn(async move |_cx| {
            let mut fields = acp::ToolCallUpdateFields::new();
            if let Some(title) = title {
                fields = fields.title(title);
            }
            if let Some(message) = message {
                fields = fields.content(vec![acp::ToolCallContent::from(message)]);
            }

            let (response_tx, response_rx) = oneshot::channel();
            if let Err(error) = stream
                .0
                .unbounded_send(Ok(ThreadEvent::ToolCallAuthorization(
                    ToolCallAuthorization {
                        tool_call: acp::ToolCallUpdate::new(tool_use_id.to_string(), fields),
                        options,
                        response: response_tx,
                        context: None,
                        kind: acp_thread::AuthorizationKind::ActionChoice,
                    },
                )))
            {
                log::error!("Failed to send tool call decision prompt: {error}");
                return Err(anyhow!("Failed to send tool call decision prompt: {error}"));
            }

            let outcome = response_rx
                .await
                .map_err(|_| anyhow!("authorization channel closed"))?;
            Ok(outcome.option_id)
        })
    }

    /// Prompts the user for authorization.
    ///
    /// When `check_settings` is `Some`, this gate is settings-driven: the
    /// settings are evaluated up-front (an Allow or Deny result resolves the
    /// task immediately without prompting), and while a prompt is pending a
    /// `SettingsStore` subscription watches for changes. A subsequent Allow
    /// or Deny dismisses the prompt UI and resolves the task without user
    /// interaction.
    ///
    /// When `check_settings` is `None`, the user is always prompted and
    /// settings changes are ignored. This suits prompts that aren't
    /// settings-driven (e.g. symlink-escape confirmations).
    pub(crate) fn run_authorization_loop(
        &self,
        title: String,
        options: acp_thread::PermissionOptions,
        context: Option<ToolPermissionContext>,
        check_settings: Option<Box<dyn Fn(&App) -> ToolPermissionDecision>>,
        cx: &mut App,
    ) -> Task<Result<()>> {
        // Short-circuit when current settings yield a definitive answer.
        if let Some(check) = check_settings.as_ref() {
            match check(cx) {
                ToolPermissionDecision::Allow => return Task::ready(Ok(())),
                ToolPermissionDecision::Deny(reason) => {
                    return Task::ready(Err(anyhow!(reason)));
                }
                ToolPermissionDecision::Confirm => {}
            }
        }

        let fs = self.fs.clone();
        let stream = self.stream.clone();
        let tool_use_id = self.tool_use_id.clone();
        let auto_resolution_outcomes = if check_settings.is_some() {
            match (
                auto_resolve_permission_outcome(&options, true),
                auto_resolve_permission_outcome(&options, false),
            ) {
                (Ok(allow), Ok(deny)) => Some((allow, deny)),
                (Err(error), _) | (_, Err(error)) => return Task::ready(Err(error)),
            }
        } else {
            None
        };
        cx.spawn(async move |cx| {
            let (response_tx, mut response_rx) = oneshot::channel();
            if let Err(error) = stream
                .0
                .unbounded_send(Ok(ThreadEvent::ToolCallAuthorization(
                    ToolCallAuthorization {
                        tool_call: acp::ToolCallUpdate::new(
                            tool_use_id.to_string(),
                            acp::ToolCallUpdateFields::new().title(title),
                        ),
                        options,
                        response: response_tx,
                        context,
                        kind: acp_thread::AuthorizationKind::PermissionGrant,
                    },
                )))
            {
                log::error!("Failed to send tool call authorization: {error}");
                return Err(anyhow!("Failed to send tool call authorization: {error}"));
            }

            let Some(check_settings) = check_settings else {
                let outcome = response_rx
                    .await
                    .map_err(|_| anyhow!("authorization channel closed"))?;

                return Self::persist_permission_outcome(&outcome, fs, cx);
            };
            let Some((auto_allow_outcome, auto_deny_outcome)) = auto_resolution_outcomes else {
                return Err(anyhow!("missing auto-resolution outcomes"));
            };

            let (mut settings_tx, mut settings_rx) = watch::channel(());
            let _settings_subscription = cx.update(|cx| {
                cx.observe_global::<SettingsStore>(move |_cx| {
                    settings_tx.send(()).ok();
                })
            });

            // Race the user's response against settings changes. On each
            // settings change, re-evaluate `check_settings`: if it now
            // yields a definitive Allow or Deny, resolve the prompt
            // without user interaction. Otherwise keep waiting on the
            // same prompt.
            loop {
                let settings_changed = async {
                    if settings_rx.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                };
                futures::select_biased! {
                    outcome = (&mut response_rx).fuse() => {
                        let outcome = outcome
                            .map_err(|_| anyhow!("authorization channel closed"))?;
                        return Self::persist_permission_outcome(&outcome, fs.clone(), cx);
                    }
                    _ = settings_changed.fuse() => {
                        // On auto-resolve, we dismiss the prompt UI by
                        // resolving the tool call's `WaitingForConfirmation`
                        // status with an internal selected outcome. Dropping
                        // `response_rx` prevents the synthetic response from
                        // being delivered back into this loop.
                        match cx.update(|cx| check_settings(cx)) {
                            ToolPermissionDecision::Allow => {
                                drop(response_rx);
                                stream.resolve_tool_call_authorization(
                                    &tool_use_id,
                                    auto_allow_outcome.clone(),
                                );
                                return Ok(());
                            }
                            ToolPermissionDecision::Deny(reason) => {
                                drop(response_rx);
                                stream.resolve_tool_call_authorization(
                                    &tool_use_id,
                                    auto_deny_outcome.clone(),
                                );
                                return Err(anyhow!(reason));
                            }
                            ToolPermissionDecision::Confirm => continue,
                        }
                    }
                }
            }
        })
    }

    /// Interprets a `SelectedPermissionOutcome` and persists any settings changes.
    /// Returns `true` if the tool call should be allowed, `false` if denied.
    pub(crate) fn persist_permission_outcome(
        outcome: &acp_thread::SelectedPermissionOutcome,
        fs: Option<Arc<dyn Fs>>,
        cx: &AsyncApp,
    ) -> Result<()> {
        let option_id = outcome.option_id.0.as_ref();
        let err = || Err(anyhow!("Permission to run tool denied by user"));

        let always_permission = option_id
            .strip_prefix("always_allow:")
            .map(|tool| (tool, ToolPermissionMode::Allow))
            .or_else(|| {
                option_id
                    .strip_prefix("always_deny:")
                    .map(|tool| (tool, ToolPermissionMode::Deny))
            })
            .or_else(|| {
                option_id
                    .strip_prefix("always_allow_mcp:")
                    .map(|tool| (tool, ToolPermissionMode::Allow))
            })
            .or_else(|| {
                option_id
                    .strip_prefix("always_deny_mcp:")
                    .map(|tool| (tool, ToolPermissionMode::Deny))
            });

        if let Some((tool, mode)) = always_permission {
            let params = outcome.params.as_ref();
            Self::persist_always_permission(tool, mode, params, fs, cx);
            return if mode == ToolPermissionMode::Allow {
                Ok(())
            } else {
                err()
            };
        }

        // Handle simple "allow" / "deny" (once, no persistence)
        if option_id == "allow" || option_id == "deny" {
            debug_assert!(
                outcome.params.is_none(),
                "unexpected params for once-only permission"
            );
            return if option_id == "allow" { Ok(()) } else { err() };
        }

        debug_assert!(false, "unexpected permission option_id: {option_id}");

        err()
    }

    /// Persists an "always allow" or "always deny" permission, using sub_patterns
    /// from params when present.
    pub(crate) fn persist_always_permission(
        tool: &str,
        mode: ToolPermissionMode,
        params: Option<&acp_thread::SelectedPermissionParams>,
        fs: Option<Arc<dyn Fs>>,
        cx: &AsyncApp,
    ) {
        let Some(fs) = fs else {
            return;
        };

        match params {
            Some(acp_thread::SelectedPermissionParams::Terminal {
                patterns: sub_patterns,
            }) => {
                debug_assert!(
                    !sub_patterns.is_empty(),
                    "empty sub_patterns for tool {tool} — callers should pass None instead"
                );
                let tool = tool.to_string();
                let sub_patterns = sub_patterns.clone();
                cx.update(|cx| {
                    update_settings_file(fs, cx, move |settings, _| {
                        let agent = settings.agent.get_or_insert_default();
                        for pattern in sub_patterns {
                            match mode {
                                ToolPermissionMode::Allow => {
                                    agent.add_tool_allow_pattern(&tool, pattern);
                                }
                                ToolPermissionMode::Deny => {
                                    agent.add_tool_deny_pattern(&tool, pattern);
                                }
                                // If there's no matching pattern this will
                                // default to confirm, so falling through is
                                // fine here.
                                ToolPermissionMode::Confirm => (),
                            }
                        }
                    });
                });
            }
            None => {
                let tool = tool.to_string();
                cx.update(|cx| {
                    update_settings_file(fs, cx, move |settings, _| {
                        settings
                            .agent
                            .get_or_insert_default()
                            .set_tool_default_permission(&tool, mode);
                    });
                });
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct ToolCallEventStreamReceiver(mpsc::UnboundedReceiver<Result<ThreadEvent>>);

#[cfg(any(test, feature = "test-support"))]
impl ToolCallEventStreamReceiver {
    pub async fn expect_authorization(&mut self) -> ToolCallAuthorization {
        let event = self.0.next().await;
        if let Some(Ok(ThreadEvent::ToolCallAuthorization(auth))) = event {
            auth
        } else {
            panic!("Expected ToolCallAuthorization but got: {:?}", event);
        }
    }

    pub async fn expect_update_fields(&mut self) -> acp::ToolCallUpdateFields {
        let event = self.0.next().await;
        if let Some(Ok(ThreadEvent::ToolCallUpdate(acp_thread::ToolCallUpdate::UpdateFields(
            update,
        )))) = event
        {
            update.fields
        } else {
            panic!("Expected update fields but got: {:?}", event);
        }
    }

    pub async fn expect_authorization_resolved(
        &mut self,
    ) -> (acp::ToolCallId, acp_thread::SelectedPermissionOutcome) {
        let event = self.0.next().await;
        if let Some(Ok(ThreadEvent::ToolCallAuthorizationResolved {
            tool_call_id,
            outcome,
        })) = event
        {
            (tool_call_id, outcome)
        } else {
            panic!("Expected authorization resolved but got: {:?}", event);
        }
    }

    pub async fn expect_diff(&mut self) -> Entity<acp_thread::Diff> {
        let event = self.0.next().await;
        if let Some(Ok(ThreadEvent::ToolCallUpdate(acp_thread::ToolCallUpdate::UpdateDiff(
            update,
        )))) = event
        {
            update.diff
        } else {
            panic!("Expected diff but got: {:?}", event);
        }
    }

    pub async fn expect_terminal(&mut self) -> Entity<acp_thread::Terminal> {
        let event = self.0.next().await;
        if let Some(Ok(ThreadEvent::ToolCallUpdate(acp_thread::ToolCallUpdate::UpdateTerminal(
            update,
        )))) = event
        {
            update.terminal
        } else {
            panic!("Expected terminal but got: {:?}", event);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl std::ops::Deref for ToolCallEventStreamReceiver {
    type Target = mpsc::UnboundedReceiver<Result<ThreadEvent>>;

    pub(crate) fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(any(test, feature = "test-support"))]
impl std::ops::DerefMut for ToolCallEventStreamReceiver {
    pub(crate) fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

