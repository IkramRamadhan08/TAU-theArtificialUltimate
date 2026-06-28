# AI & Agent

TAU has a built-in AI agent for code generation, editing, and analysis.

## Setup

1. Open **AI Panel** (`Ctrl+Shift+A` / `Cmd+Shift+A`)
2. Click **Configure Provider**
3. Enter your API key for one of the supported providers

## Supported Providers

| Provider | Model |
|----------|-------|
| OpenAI | GPT-4o, GPT-4, GPT-3.5 |
| Anthropic | Claude 3.5 Sonnet, Claude 3 Opus |
| Google | Gemini 1.5 Pro, Gemini 1.5 Flash |
| Ollama | Local models (Llama 3, Mistral, etc.) |
| LM Studio | Local models (OpenAI-compatible) |
| DeepSeek | DeepSeek V2, DeepSeek Coder |
| OpenRouter | Multi-provider access |

## Usage

### Inline Assistant (`Ctrl+I` / `Cmd+I`)

Select code and press `Ctrl+I` to:

- Explain code
- Refactor
- Fix bugs
- Add documentation
- Generate tests

### AI Panel (`Ctrl+Shift+A` / `Cmd+Shift+A`)

Full chat interface for:

- Code generation from natural language
- Multi-file editing
- Project-wide analysis
- Review and refactoring

### Agent Tools

The agent can:

- Read and edit files in your project
- Search code with regex/semantic search
- Run terminal commands
- List and inspect directory structures

## Example

```text
You: Add a REST API endpoint for user registration in main.py

TAU: [Reads main.py, analyzes imports and existing routes]
     [Edits main.py to add registration endpoint]
     [Creates tests/test_auth.py with test cases]
```

## Privacy

- Using local providers (Ollama, LM Studio): all data stays on your machine
- Cloud providers: code is sent to the provider's API
- Telemetry can be disabled in settings
