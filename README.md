# TAU — The Artificial Ultimate

  <div align="center">

<img src="TAU.png" alt="TAU Logo" width="120">

**A local-first, agentic coding IDE for Linux, macOS, and Windows.**

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE-GPL)
[![Rust](https://img.shields.io/badge/rust-1.95.0-orange)](rust-toolchain.toml)
[![Build](https://img.shields.io/badge/build-passing-brightgreen)]()
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey)]()

![TAU Editor screenshot](editor_screenshot.png)

</div>

TAU is a high-performance, GPU-accelerated code editor with built-in AI agent capabilities. Forked from [Zed](https://zed.dev).

> 📖 **Full documentation:** [`editor/docs/`](editor/docs/) — User guide, configuration, keybindings, and more.

## Features

- **Agentic AI** — Built-in LLM integration for code generation, editing, analysis, and automated tasks
- **Multi-language** — First-class support for Rust, Python, TypeScript, JavaScript, Go, HTML, CSS, JSON, and more via LSP
- **Vim mode** — Full vim emulation with custom keymaps
- **Real-time collaboration** — Multi-user editing with shared workspaces (self-hosted)
- **GPU-accelerated rendering** — Built on GPUI framework using Vulkan, Metal, or DirectX
- **Terminal** — Integrated terminal with multiplexing
- **Git integration** — Inline blame, diff viewer, branch management, and commit UI
- **Debugger** — Built-in debug adapter protocol (DAP) support
- **Extensible** — WebAssembly-based extensions with custom language grammars
- **Theme support** — Customizable UI themes (Ayu, One, Gruvbox, and more)

## Quick Start

### Install Script (All Platforms)

```bash
curl -fsSL https://raw.githubusercontent.com/IkramRamadhan08/TAU-theArtificialUltimate/main/install.sh | bash
```

The script will:
- **Linux x86-64** — Download the pre-built binary (~130 MB)
- **macOS / ARM Linux** — Build from source (requires Rust; install from [rustup.rs](https://rustup.rs))
- Ask if you want a **desktop shortcut** (language matches your system locale)
- Add `~/.local/bin` to your `PATH`

Then just type `tau` in a terminal — the terminal closes automatically and TAU appears.

> Requires `~/.local/bin` to be in your `PATH`. Add `export PATH="$PATH:$HOME/.local/bin"` to your shell config if needed.

### Manual Install (Pre-built Binary)

```bash
curl -L https://github.com/IkramRamadhan08/TAU-theArtificialUltimate/releases/latest/download/tau-x86_64-linux.tar.gz | tar xz -C ~/.local/bin
```

### Build from Source (any platform)

Requires **Rust 1.95.0**:

```bash
git clone https://github.com/IkramRamadhan08/TAU-theArtificialUltimate.git
cd TAU_Project/editor
cargo run --release --bin tau
```

> **macOS**: Xcode Command Line Tools required (`xcode-select --install`).
> **Windows**: Visual Studio Build Tools with "Desktop development with C++" workload required.

## Configuration

TAU is configured via JSON files:

| File | Purpose |
|------|---------|
| `~/.config/tau/settings.json` | User settings |
| `~/.config/tau/keymap.json` | Custom keybindings |
| `~/.config/tau/themes/` | Custom themes |

Example `settings.json`:
```json
{
  "theme": "Ayu Dark",
  "font_family": "JetBrains Mono",
  "font_size": 14,
  "tab_size": 4,
  "vim_mode": true,
  "telemetry": false
}
```

## Platform Support

| OS | Status | GPU Backend | Windowing |
|----|--------|-------------|-----------|
| Linux | ✅ Stable | Vulkan / OpenGL | X11 / Wayland |
| macOS | ✅ Stable | Metal | Cocoa |
| Windows | ✅ Stable | Vulkan (via DX12/WGPU) | Win32 |

## Keybindings

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Command palette | `Ctrl+Shift+P` | `Cmd+Shift+P` |
| File finder | `Ctrl+P` | `Cmd+P` |
| Toggle terminal | `Ctrl+\`` | `Cmd+\`` |
| Save | `Ctrl+S` | `Cmd+S` |
| Search in file | `Ctrl+F` | `Cmd+F` |
| Search in project | `Ctrl+Shift+F` | `Cmd+Shift+F` |

Full keymaps: `editor/assets/keymaps/`

## Project Structure

```
editor/
├── crates/            # 236 Rust crates
│   ├── gpui/          # GPU-accelerated UI framework
│   ├── editor/        # Core editor engine
│   ├── agent/         # AI agent runtime & tool execution
│   ├── language/      # Language server protocol & parsing
│   ├── project/       # Project management & LSP store
│   ├── vim/           # Vim emulation
│   └── ...
├── assets/            # Themes, keymaps, icons, settings
│   ├── themes/        # Ayu, Gruvbox, One themes
│   ├── keymaps/       # Platform-specific keybindings
│   └── settings/      # Default configuration
├── extensions/        # WASM extension examples
├── script/            # Build, CI, and release scripts
└── Cargo.toml         # Workspace definition
```

## Troubleshooting

| Problem | Solution |
|---------|----------|
| Build fails on WebRTC | Set `TAU_NO_WEBRTC=true` to skip WebRTC download |
| GPU not detected | Ensure Vulkan drivers are installed (`mesa-vulkan-drivers` on Arch, `mesa-vulkan-drivers` on Debian) |
| Missing X11 libs | Install `libxcb`, `libxkbcommon` development packages for your distro |
| Fonts not rendering | Install `fontconfig` and ensure system fonts are available |

## Contributing

TAU is open-source and **we welcome contributions from everyone!**

| Area | How to Contribute |
|------|-------------------|
| 🐛 Bugs | [Open an issue](https://github.com/IkramRamadhan08/TAU_Project/issues) with steps to reproduce |
| 💡 Features | Suggest via issues or submit a PR |
| 📖 Docs | Improve guides, fix typos, add examples |
| 🔌 Extensions | Build WASM extensions for languages/tools |
| 🌍 Translations | Help translate the editor and docs |

```bash
# Get started
git clone https://github.com/IkramRamadhan08/TAU-theArtificialUltimate.git
cd TAU_Project/editor
cargo check
```

Read the [documentation](editor/docs/) to understand the codebase.

## License

Original [Zed](https://zed.dev) source code is licensed under GPL-3.0-or-later with Apache-2.0 components. TAU modifications to GPL-covered files are distributed under GPL-3.0-or-later.

See [LICENSE-GPL](LICENSE-GPL) and [LICENSE-APACHE](LICENSE-APACHE) for details.
