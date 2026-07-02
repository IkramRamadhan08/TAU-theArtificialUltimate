# AI & Agent

TAU has a deeply integrated AI agent that can write code, run commands, edit files, search code, and build projects — all from natural language.

## Quick Start

1. Open **AI Panel** (`Ctrl+Shift+A` / `Cmd+Shift+A`)
2. Click **Configure Provider** or edit `~/.config/tau/settings.json`
3. Enter your API key for a supported provider
4. Type a prompt — the agent does the rest

## Supported Providers

| Provider | Models | Configuration |
|---|---|---|
| **Mistral** | Mistral Small/Large, Codestral | `api_key` |
| **OpenAI** | GPT-4o, GPT-4, GPT-3.5 | `api_key` |
| **Anthropic** | Claude Sonnet 4, Claude 3.5 Haiku | `api_key` |
| **Google** | Gemini 2.0 Flash, Gemini 1.5 Pro | `api_key` |
| **Ollama** | Any local model | `base_url`, `model` |
| **OpenRouter** | Multi-provider access | `api_key`, `model` |
| **Copilot Chat** | GitHub Copilot models | GitHub auth |
| **DeepSeek** | DeepSeek V2, Coder | `api_key` |
| **xAI (Grok)** | Grok models | `api_key` |
| **AWS Bedrock** | Claude, Llama via AWS | AWS credentials |
| **LM Studio** | Local OpenAI-compatible | `base_url` |
| **Azure** | OpenAI via Azure | `api_key`, `endpoint` |

### Configuration

```json
{
  "language_models": {
    "mistral": {
      "api_key": "YOUR_API_KEY",
      "model": "mistral-small-latest"
    },
    "ollama": {
      "model": "codestral",
      "base_url": "http://localhost:11434"
    }
  }
}
```

## Agent Features

### Tools

The agent uses tools to interact with your system:

| Tool | Description |
|---|---|
| `terminal` | Run shell commands (build, test, install, git) |
| `read_file` | Read file contents |
| `write_file` | Create or overwrite files |
| `edit_file` | Make targeted edits to existing files |
| `create_directory` | Create new directories |
| `delete_path` | Delete files or directories |
| `move_path` | Move/rename files |
| `copy_path` | Copy files |
| `grep` | Search code with regex patterns |
| `find_path` | Find files by name pattern |
| `list_directory` | List directory contents |
| `search_semantic` | AI-powered semantic code search |
| `go_to_definition` | Navigate to symbol definitions |
| `find_references` | Find all references to a symbol |
| `rename_symbol` | Rename symbols across files |
| `diagnostics` | Show project errors and warnings |
| `get_code_actions` | Get available code actions |
| `apply_code_action` | Apply code actions |
| `git_status` | Show working tree status |
| `git_commit` | Commit changes |
| `git_push` | Push commits |
| `git_branch` | List/create branches |
| `git_log` | View commit history |
| `fetch` | Download URLs |
| `web_search` | Search the web |
| `skill` | Load agent skill instructions |
| `spawn_agent` | Delegate work to sub-agents |

When no project folder is open, only terminal, fetch, web_search, skill, spawn_agent, and create_thread are available — letting you build projects from scratch.

### Skills

Skills provide specialized instructions for common workflows. 14 built-in skills are available:

| Skill | When to Use |
|---|---|
| **brainstorming** | Before any creative work — explores intent, requirements, and design |
| **test-driven-development** | Before writing implementation — write tests first |
| **systematic-debugging** | When encountering bugs or test failures — structured investigation |
| **writing-plans** | Before multi-step tasks — create detailed implementation plans |
| **executing-plans** | When executing a written plan — review checkpoints |
| **subagent-driven-development** | When using sub-agents for parallel task execution |
| **dispatching-parallel-agents** | For 2+ independent tasks without shared state |
| **verification-before-completion** | Before claiming work is done — verify with evidence |
| **requesting-code-review** | Before merging — thorough review workflow |
| **receiving-code-review** | When processing review feedback — technical rigor |
| **finishing-a-development-branch** | When work is complete — merge/PR/cleanup decisions |
| **using-git-worktrees** | Before starting isolated feature work |
| **create-skill** | When creating or packaging reusable agent instructions |
| **writing-skills** | When creating, editing, or verifying custom skills |

Custom skills go in `~/.agents/skills/`.

### Verification Gate

After making file modifications, the agent automatically runs a verification gate:

1. Prompts the model to run `cargo check` (or equivalent build command)
2. Prompts the model to run `cargo test` (or equivalent test command)
3. Blocks the final summary until verification passes

This ensures changes compile and tests pass before the agent reports completion.

### Workflow

The agent follows a structured workflow for every task:

1. **Understand** — Read relevant files and gather context
2. **Plan** — Think through the approach
3. **Execute** — Use tools to make changes
4. **Validate** — Run build and test commands
5. **Report** — Summarize what changed and what passed

## Agent Profiles

Define multiple agent profiles with different models and tool sets:

```json
{
  "agent": {
    "profiles": {
      "default": {
        "enabled": true,
        "model": {
          "provider": "mistral",
          "model": "mistral-small-latest"
        },
        "tools": {
          "terminal": true,
          "read_file": true,
          "write_file": true,
          "grep": true,
          "web_search": false
        }
      },
      "fast": {
        "model": {
          "provider": "ollama",
          "model": "codestral"
        }
      }
    }
  }
}
```

## Agent Settings

Key agent settings in `~/.config/tau/settings.json`:

| Setting | Type | Default | Description |
|---|---|---|---|
| `agent.default_profile` | string | `"default"` | Active profile |
| `agent.request_timeout` | integer (secs) | `120` | Max time per agent request |
| `agent.circuit_breaker.enabled` | bool | `true` | Auto-backoff on API errors |
| `agent.circuit_breaker.max_retries` | integer | `3` | Max retries before cooldown |
| `agent.auto_compact` | bool | `true` | Automatically compact long conversations |
| `agent.sandbox.enabled` | bool | `false` | Sandbox terminal commands |

## Privacy

- **Local providers** (Ollama, LM Studio): all data stays on your machine
- **Cloud providers**: code and prompts are sent to the provider's API
- **Telemetry**: can be disabled in settings (`"telemetry": false`)

## Example Workflows

### Build a Rust CLI app from scratch

```
You: Create a CLI todo app in Rust

Agent: [Opens terminal]
       → cargo new todo-cli
       → cd todo-cli
       [Edits src/main.rs with CLI structure]
       [Edits Cargo.toml to add clap dependency]
       → cargo check
       → cargo test
       Reports: Created todo-cli with add/list/done commands
```

### Debug a failing test

```
You: The login test is failing

Agent: [Runs the test to see the error]
       [Reads the test file and implementation]
       → grep for related code
       [Identifies the bug]
       [Edits the fix]
       → cargo test (passes)
       Reports: Fixed token expiry check in auth.rs
```
