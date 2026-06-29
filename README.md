# TAU — The Artificial Ultimate

  <div align="center">

<img src="TAU.png" alt="TAU Logo" width="120">

**A local-first, agentic coding IDE.** Forked from [Zed](https://zed.dev).

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE-GPL)
[![Rust](https://img.shields.io/badge/rust-1.95.0-orange)](rust-toolchain.toml)
[![Release](https://img.shields.io/badge/release-v0.62--experimental-yellow)]()

![TAU Editor screenshot](editor_screenshot.png)

</div>

## Status

**Experimental.** TAU is a fork of Zed with an integrated AI agent. It works but is rough around the edges.

| What | Status |
|------|--------|
| Core editor (forked from Zed) | ✅ Stable |
| AI agent with tool execution | ✅ Working |
| Mistral, Ollama, OpenAI providers | ✅ Working |
| Other LLM providers (Anthropic, Google, DeepSeek, etc.) | ⚠️ Code present, untested |
| TAU Cloud (collaboration) | ❌ Not implemented |
| Pre-built binary | ✅ Linux x86-64 only |
| macOS / Windows | ⚠️ Build from source required |
| Auto-update | ✅ Via GitHub Releases |
| Documentation | 🚧 Incomplete |

## Quick Start

### Install (Linux x86-64)

```bash
curl -fsSL https://raw.githubusercontent.com/IkramRamadhan08/TAU-theArtificialUltimate/main/install.sh | bash
```

Downloads pre-built binary from the [latest release](https://github.com/IkramRamadhan08/TAU-theArtificialUltimate/releases/latest).

### Build from Source (any platform)

Requires **Rust 1.95.0**:

```bash
git clone https://github.com/IkramRamadhan08/TAU-theArtificialUltimate.git
cd TAU_Project/editor
cargo run --release --bin tau
```

> **macOS**: Xcode Command Line Tools required (`xcode-select --install`).
> **Windows**: Visual Studio Build Tools with "Desktop development with C++" workload required.
> **Build time**: 30+ minutes on a modern machine.

### Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/IkramRamadhan08/TAU-theArtificialUltimate/main/uninstall.sh | bash
```

## LLM Providers

TAU supports multiple LLM providers. Configure them in `~/.config/tau/settings.json`:

```json
{
  "language_models": {
    "mistral": {
      "api_key": "your-mistral-api-key",
      "model": "mistral-small-latest"
    },
    "ollama": {
      "model": "codestral",
      "base_url": "http://localhost:11434"
    },
    "openai": {
      "api_key": "your-openai-api-key",
      "model": "gpt-4o"
    }
  }
}
```

**Tested and working**: Mistral, Ollama, OpenAI. Other providers (Anthropic, Google, DeepSeek, xAI, OpenRouter, etc.) have code in place but lack real-world testing. Report issues if something doesn't work.

## Agent Features

- Built-in AI agent with tool execution (terminal, file read/write, search, git, web fetch)
- 14 built-in skills (brainstorming, debugging, TDD, code review, etc.)
- Slash commands: `/permission`, `/skill`
- Custom skills in `~/.agents/skills/`
- Circuit breaker (auto-backoff on API errors)
- Configurable request timeout (default 120s)

## Configuration

| File | Purpose |
|------|---------|
| `~/.config/tau/settings.json` | User settings, LLM config |
| `~/.config/tau/keymap.json` | Custom keybindings |
| `~/.config/tau/themes/` | Custom themes |
| `~/.agents/skills/` | Custom agent skills |

## Platform Support

| OS | Binary | GPU Backend | Windowing |
|----|--------|-------------|-----------|
| Linux x86-64 | ✅ Pre-built | Vulkan / OpenGL | X11 / Wayland |
| Linux ARM | ⚠️ Build from source | Vulkan | X11 / Wayland |
| macOS | ⚠️ Build from source | Metal | Cocoa |
| Windows | ⚠️ Build from source | Vulkan (via DX12/WGPU) | Win32 |

## Limitations

- **macOS/Windows**: No pre-built binaries yet. Must compile from Rust source.
- **TAU Cloud**: Collaboration features from Zed require a cloud backend which does not exist yet.
- **Auto-update**: Checks GitHub Releases; only Linux x86-64 binary is published currently.
- **Web search tool**: Requires external API configuration.
- **Some Zed features** may be broken or missing due to the fork.

## License

Original [Zed](https://zed.dev) source code is licensed under GPL-3.0-or-later with Apache-2.0 components. TAU modifications to GPL-covered files are distributed under GPL-3.0-or-later.

See [LICENSE-GPL](LICENSE-GPL) and [LICENSE-APACHE](LICENSE-APACHE) for details.
