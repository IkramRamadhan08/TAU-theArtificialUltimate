use crate::{AgentTool, ToolCallEventStream, ToolInput};
use agent_client_protocol::schema as acp;
use anyhow::{Context as _, Result};
use gpui::{App, Entity, SharedString, Task};
use project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn project_root(project: &Entity<Project>, cx: &App) -> Result<std::path::PathBuf> {
    project
        .read(cx)
        .visible_worktrees(cx)
        .next()
        .map(|wt| wt.read(cx).abs_path().to_path_buf())
        .context("no project worktree found")
}

fn run_git(cwd: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout.trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(anyhow::anyhow!("git {} failed: {}", args.join(" "), stderr.trim()))
    }
}

// --- git_status ---

/// Show the working tree status (modified, staged, untracked files).
///
/// Returns a summary of the current git status, similar to `git status --short`.
/// Includes staged, unstaged, and untracked files.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GitStatusToolInput {}

pub struct GitStatusTool {
    project: Entity<Project>,
}

impl GitStatusTool {
    pub fn new(project: Entity<Project>) -> Self {
        Self { project }
    }
}

impl AgentTool for GitStatusTool {
    type Input = GitStatusToolInput;
    type Output = String;

    const NAME: &'static str = "git_status";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn initial_title(&self, _input: Result<Self::Input, serde_json::Value>, _cx: &mut App) -> SharedString {
        "Git Status".into()
    }

    fn run(
        self: Arc<Self>,
        _input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let root = match project_root(&self.project, cx) {
            Ok(r) => r,
            Err(e) => return Task::ready(Err(e.to_string())),
        };

        let status = run_git(&root, &["status", "--short", "--branch"]);
        let detailed = run_git(&root, &["status"]);

        Task::ready(match (status, detailed) {
            (Ok(s), Ok(d)) => Ok(format!("{}\n\n{}", s, d)),
            (Ok(s), Err(_)) => Ok(s),
            (Err(_), Ok(d)) => Ok(d),
            (Err(e), _) => Err(e.to_string()),
        })
    }
}

// --- git_commit ---

/// Stage files and create a commit.
///
/// Stages the specified files (or all changes) and creates a commit with the given message.
/// Use `"files": ["."]` or omit files to stage all changes.
/// Use `"files": ["path/to/file.rs"]` to stage specific files.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GitCommitToolInput {
    /// Commit message describing the changes.
    pub message: String,
    /// Files to stage. Use `["."]` or omit to stage all changes.
    /// Provide specific file paths to stage only those files.
    #[serde(default)]
    pub files: Vec<String>,
}

pub struct GitCommitTool {
    project: Entity<Project>,
}

impl GitCommitTool {
    pub fn new(project: Entity<Project>) -> Self {
        Self { project }
    }
}

impl AgentTool for GitCommitTool {
    type Input = GitCommitToolInput;
    type Output = String;

    const NAME: &'static str = "git_commit";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Edit
    }

    fn initial_title(&self, input: Result<Self::Input, serde_json::Value>, _cx: &mut App) -> SharedString {
        if let Ok(ref input) = input {
            format!("Git Commit: {}", input.message).into()
        } else {
            "Git Commit".into()
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let root = match project_root(&self.project, cx) {
            Ok(r) => r,
            Err(e) => return Task::ready(Err(e.to_string())),
        };

        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| e.to_string())?;

            let files: Vec<&str> = if input.files.is_empty() {
                vec!["."]
            } else {
                input.files.iter().map(|s| s.as_str()).collect()
            };

            let mut args = vec!["add", "--"];
            args.extend(files);

            run_git(&root, &args)
                .map_err(|e| format!("stage failed: {}", e))?;

            let output = run_git(&root, &["commit", "-m", &input.message])
                .map_err(|e| format!("commit failed: {}", e))?;

            Ok(output)
        })
    }
}

// --- git_push ---

/// Push commits to a remote branch.
///
/// Pushes the current branch to the specified remote and branch.
/// If remote and branch are omitted, pushes to the configured upstream.
/// Use `force: true` for force push (use with caution).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GitPushToolInput {
    /// Remote name (default: "origin").
    #[serde(default = "default_remote")]
    pub remote: String,
    /// Branch to push. If omitted, pushes the current branch.
    pub branch: Option<String>,
    /// Whether to force push with lease (safer than force push).
    /// This uses `--force-with-lease` to avoid overwriting others' work.
    #[serde(default)]
    pub force: bool,
    /// Set upstream tracking (-u flag).
    #[serde(default)]
    pub set_upstream: bool,
}

fn default_remote() -> String {
    "origin".to_string()
}

pub struct GitPushTool {
    project: Entity<Project>,
}

impl GitPushTool {
    pub fn new(project: Entity<Project>) -> Self {
        Self { project }
    }
}

impl AgentTool for GitPushTool {
    type Input = GitPushToolInput;
    type Output = String;

    const NAME: &'static str = "git_push";

    fn kind() -> acp::ToolKind {
acp::ToolKind::Edit
    }

    fn initial_title(&self, _input: Result<Self::Input, serde_json::Value>, _cx: &mut App) -> SharedString {
        "Git Push".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let root = match project_root(&self.project, cx) {
            Ok(r) => r,
            Err(e) => return Task::ready(Err(e.to_string())),
        };

        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| e.to_string())?;

            let mut args = vec!["push", &input.remote];
            if input.force {
                args.push("--force-with-lease");
            }
            if input.set_upstream {
                args.push("-u");
            }
            if let Some(ref branch) = input.branch {
                args.push(branch);
            }

            let output = run_git(&root, &args).map_err(|e| format!("push failed: {}", e))?;
            Ok(output)
        })
    }
}

// --- git_branch
///
/// Without arguments, lists all branches.
/// Provide a name to create a new branch (optionally with a base branch).
/// Use `delete: true` to delete a branch (safe: refuses if unmerged).
/// Use `delete_force: true` to force-delete a branch.
/// Use `switch: true` to switch to an existing branch (like git checkout).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GitBranchToolInput {
    /// Name of the branch to create, switch to, or delete.
    /// When empty, lists all branches.
    pub name: Option<String>,
    /// When creating a branch, the base branch/commit to branch from.
    /// Defaults to the current HEAD.
    pub base: Option<String>,
    /// Switch to the specified branch.
    #[serde(default)]
    pub switch: bool,
    /// Delete the specified branch (safe mode - refuses if unmerged).
    #[serde(default)]
    pub delete: bool,
    /// Force delete the specified branch (discards unmerged changes).
    #[serde(default)]
    pub delete_force: bool,
}

pub struct GitBranchTool {
    project: Entity<Project>,
}

impl GitBranchTool {
    pub fn new(project: Entity<Project>) -> Self {
        Self { project }
    }
}

impl AgentTool for GitBranchTool {
    type Input = GitBranchToolInput;
    type Output = String;

    const NAME: &'static str = "git_branch";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Edit
    }

    fn initial_title(&self, _input: Result<Self::Input, serde_json::Value>, _cx: &mut App) -> SharedString {
        "Git Branch".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let root = match project_root(&self.project, cx) {
            Ok(r) => r,
            Err(e) => return Task::ready(Err(e.to_string())),
        };

        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| e.to_string())?;

            let name = match input.name {
                Some(ref n) => n.clone(),
                None => {
                    return run_git(&root, &["branch", "--list", "-a"])
                        .map_err(|e| format!("list branches failed: {}", e));
                }
            };

            if input.delete {
                return run_git(&root, &["branch", "--delete", &name])
                    .map_err(|e| format!("delete branch failed: {}", e));
            }

            if input.delete_force {
                return run_git(&root, &["branch", "--delete", "--force", &name])
                    .map_err(|e| format!("force delete branch failed: {}", e));
            }

            if input.switch {
                return run_git(&root, &["checkout", &name])
                    .map_err(|e| format!("switch branch failed: {}", e));
            }

            let mut args = vec!["checkout", "-b", &name];
            if let Some(ref base) = input.base {
                args.push(base);
            }
            run_git(&root, &args).map_err(|e| format!("create branch failed: {}", e))
        })
    }
}

// --- git_log ---

/// Show commit history.
///
/// Returns recent commits with their hashes, authors, dates, and messages.
/// You can specify a limit and a path to filter by.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GitLogToolInput {
    /// Maximum number of commits to show (default: 10).
    #[serde(default = "default_log_limit")]
    pub limit: usize,
    /// Optional file path to filter commits affecting this file.
    pub path: Option<String>,
}

fn default_log_limit() -> usize {
    10
}

pub struct GitLogTool {
    project: Entity<Project>,
}

impl GitLogTool {
    pub fn new(project: Entity<Project>) -> Self {
        Self { project }
    }
}

impl AgentTool for GitLogTool {
    type Input = GitLogToolInput;
    type Output = String;

    const NAME: &'static str = "git_log";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn initial_title(&self, _input: Result<Self::Input, serde_json::Value>, _cx: &mut App) -> SharedString {
        "Git Log".into()
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        let root = match project_root(&self.project, cx) {
            Ok(r) => r,
            Err(e) => return Task::ready(Err(e.to_string())),
        };

        cx.spawn(async move |_cx| {
            let input = input.recv().await.map_err(|e| e.to_string())?;

            let log_limit = format!("--max-count={}", input.limit);
            let mut args = vec!["log", &log_limit, "--oneline", "--decorate"];
            if let Some(ref path) = input.path {
                args.push("--");
                args.push(path);
            }

            run_git(&root, &args).map_err(|e| format!("log failed: {}", e))
        })
    }
}
