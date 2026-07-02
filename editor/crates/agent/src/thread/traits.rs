use super::thread_types::{AvailableAgents, SiblingThreadInfo, SiblingThreadRequest};
use acp_thread;
use agent_client_protocol::schema as acp;
use anyhow::Result;
use gpui::{App, AsyncApp, Task};
use std::path::PathBuf;
use std::rc::Rc;
use futures::future::Shared;
pub trait TerminalHandle {
    fn id(&self, cx: &AsyncApp) -> Result<acp::TerminalId>;
    fn current_output(&self, cx: &AsyncApp) -> Result<acp::TerminalOutputResponse>;
    fn wait_for_exit(&self, cx: &AsyncApp) -> Result<Shared<Task<acp::TerminalExitStatus>>>;
    fn kill(&self, cx: &AsyncApp) -> Result<()>;
    fn was_stopped_by_user(&self, cx: &AsyncApp) -> Result<bool>;
}

pub trait SubagentHandle {
    /// The session ID of this subagent thread
    fn id(&self) -> acp::SessionId;
    /// The current number of entries in the thread.
    /// Useful for knowing where the next turn will begin
    fn num_entries(&self, cx: &App) -> usize;
    /// Runs a turn for a given message and returns both the response and the index of that output message.
    fn send(&self, message: String, cx: &AsyncApp) -> Task<Result<String>>;
}

pub trait ThreadEnvironment {
    fn create_terminal(
        &self,
        command: String,
        extra_env: Vec<acp::EnvVariable>,
        cwd: Option<PathBuf>,
        output_byte_limit: Option<u64>,
        sandbox_wrap: Option<acp_thread::SandboxWrap>,
        cx: &mut AsyncApp,
    ) -> Task<Result<Rc<dyn TerminalHandle>>>;

    fn create_subagent(&self, label: String, cx: &mut App) -> Result<Rc<dyn SubagentHandle>>;

    fn resume_subagent(
        &self,
        _session_id: acp::SessionId,
        _cx: &mut App,
    ) -> Result<Rc<dyn SubagentHandle>> {
        Err(anyhow::anyhow!(
            "Resuming subagent sessions is not supported"
        ))
    }

    /// Creates an independent sibling thread visible in the agent sidebar.
    /// Unlike subagents, sibling threads are first-class threads that persist
    /// and run in parallel without reporting results back to the parent.
    fn create_sibling_thread(
        &self,
        request: SiblingThreadRequest,
        cx: &mut AsyncApp,
    ) -> Task<Result<SiblingThreadInfo>> {
        let _ = request;
        let _ = cx;
        Task::ready(Err(anyhow::anyhow!(
            "Creating sibling threads is not supported in this environment"
        )))
    }

    /// Lists the agents and models available for use with `create_sibling_thread`.
    fn list_available_agents(&self, cx: &mut App) -> Result<AvailableAgents> {
        let _ = cx;
        Err(anyhow::anyhow!(
            "Listing available agents is not supported in this environment"
        ))
    }
}

