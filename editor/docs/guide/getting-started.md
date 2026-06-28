# Getting Started

## Prerequisites

- **Rust 1.95.0:** `rustup toolchain install 1.95.0`
- **System deps:** See [README > Prerequisites](../../README.md#prerequisites)

## Build

```bash
cd editor
cargo run --bin tau
```

> First build: ~236 crates, 15–30 min.

## Install to PATH

```bash
cargo build --release --bin tau
cp target/release/tau ~/.local/bin/
```

Or use the install script:
```bash
./install.sh
```

## First Launch

```bash
tau                    # Open current directory
tau ~/projects/my-app  # Open specific project
```

## Open a Project

- `Ctrl+O` — Open file
- `Ctrl+Shift+O` — Open folder
- Drag & drop folder onto TAU window

## Next Steps

- [Configure TAU](configuration.md)
- [Learn keybindings](keybindings.md)
- [Try the AI agent](ai-agent.md)
