# Contributing to TAU

Thank you for your interest in contributing to TAU! This guide will help you get started.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Ways to Contribute](#ways-to-contribute)
- [Development Setup](#development-setup)
- [Coding Guidelines](#coding-guidelines)
- [Pull Request Process](#pull-request-process)
- [Project Structure](#project-structure)
- [Getting Help](#getting-help)

---

## Code of Conduct

We are committed to providing a welcoming and inclusive experience for everyone. Be respectful, constructive, and considerate in all interactions.

## Ways to Contribute

### Bug Reports

- Check if the bug has already been reported in [Issues](https://github.com/IkramRamadhan08/TAU-theArtificialUltimate/issues)
- Include TAU version, OS, LLM provider, and steps to reproduce
- Attach logs if possible (`tau --foreground` captures logs)

### Feature Requests

- Open an issue describing the feature and use case
- Explain why it would be valuable to other users
- Tag with `enhancement`

### Pull Requests

- Start with an issue to discuss the change before coding
- Keep PRs focused — one feature/fix per PR
- Include tests for new functionality
- Update documentation when changing behavior

### Documentation

- Fix typos, clarify confusing sections, add examples
- Translate guides to other languages
- Write doc comments for public APIs

---

## Development Setup

### Prerequisites

- **Rust 1.95.0** ([rustup.rs](https://rustup.rs))
- **macOS:** Xcode Command Line Tools (`xcode-select --install`)
- **Windows:** Visual Studio Build Tools with "Desktop development with C++"
- **Linux:** System dependencies (see [release.yml](.github/workflows/release.yml))

### Build and Run

```bash
git clone https://github.com/IkramRamadhan08/TAU-theArtificialUltimate.git
cd TAU-theArtificialUltimate/editor
cargo run --release --bin tau
```

### Development Build (faster iteration)

```bash
cargo run --bin tau
```

This uses debug mode and is significantly faster to compile.

### Run Tests

```bash
cargo test --release        # Full test suite
cargo test -p agent         # Agent-specific tests
cargo test -p gpui          # GPUI framework tests
```

### Linting

```bash
./script/clippy             # Uses the project's clippy configuration
```

---

## Coding Guidelines

### Rust

- Follow standard Rust idioms and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` before committing
- Run `./script/clippy` and address warnings
- Write doc comments (`///`) for all public items
- Avoid `unwrap()` — prefer `?`, `context()`, or `.log_err()`
- No organizational comments that restate the code — explain *why*, not *what*

### GPUI

TAU uses the GPUI framework for its UI. Key patterns:

- **Entities** are reference-counted state containers: `Entity<T>`
- **Contexts** provide access to services: `App`, `Context<T>`, `AsyncApp`
- **Render** trait converts state into UI elements
- **Actions** are dispatched keyboard shortcuts
- Use `cx.notify()` to trigger re-renders after state changes

See the [GPUI documentation](editor/crates/gpui/docs/contexts.md) for details.

### Agent

- The agent lives in `editor/crates/agent/`
- Tools are defined in `editor/crates/agent/src/tools/`
- Agent settings schema is in `editor/crates/agent_settings/`
- Built-in skills are in `editor/crates/agent_skills/builtin/`
- System prompt templates use Handlebars (`.hbs` files)
- New tools should implement the `AnyAgentTool` trait

### Commit Messages

```
crate-name: Brief description (50 char max)

Optional longer description with context.

Closes #123
```

Prefix with the crate name when the change is focused on one area.

---

## Pull Request Process

1. **Create an issue** describing what you're fixing or adding
2. **Fork the repository** and create a feature branch
3. **Make your changes** following the coding guidelines
4. **Write or update tests** to cover your changes
5. **Run tests** to ensure nothing is broken
6. **Submit a PR** referencing the issue number
7. **Address review feedback** — all PRs require at least one review

### PR Checklist

- [ ] Code compiles without errors or warnings
- [ ] Tests pass (`cargo test`)
- [ ] Clippy passes (`./script/clippy`)
- [ ] Documentation updated (if applicable)
- [ ] No unrelated changes in the PR

---

## Project Structure

```
editor/
├── crates/                     # All Rust crates (236+)
│   ├── agent/                  # AI agent core
│   ├── agent_settings/         # Agent config schema
│   ├── agent_skills/           # Skill system + built-in skills
│   ├── agent_ui/               # Agent panel UI
│   ├── auto_update/            # Auto-update system
│   ├── gpui/                   # GPU UI framework
│   ├── language/               # Language support
│   ├── project/                # Project management
│   ├── terminal/               # Terminal emulator
│   ├── vim/                    # Vim mode
│   ├── anthropic/              # Anthropic provider
│   ├── openai/                 # OpenAI provider
│   ├── ollama/                 # Ollama provider
│   ├── google_ai/              # Google AI provider
│   └── ...                     # 220+ more crates
├── docs/                       # Documentation
│   ├── guide/                  # User guides
│   └── dev/                    # Developer docs
└── assets/                     # Bundled assets
```

---

## Getting Help

- **Issues:** [github.com/IkramRamadhan08/TAU-theArtificialUltimate/issues](https://github.com/IkramRamadhan08/TAU-theArtificialUltimate/issues)
- **Documentation:** [editor/docs/](editor/docs/)

---

<div align="center">
Every contribution counts. Thank you! ❤️
</div>
