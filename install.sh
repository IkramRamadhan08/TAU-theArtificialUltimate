#!/usr/bin/env bash
set -e

echo "=== TAU Editor Installer ==="

# Check Rust
if ! command -v rustc &>/dev/null; then
    echo "Error: Rust not found. Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

RUST_VERSION=$(rustc --version | cut -d' ' -f2)
echo "Rust: $RUST_VERSION"

# Install 1.95.0 toolchain if needed
if ! rustup toolchain list | grep -q "1.95.0"; then
    echo "Installing Rust 1.95.0 toolchain..."
    rustup toolchain install 1.95.0
fi

# Linux deps
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "Checking Linux dependencies..."
    if command -v apt &>/dev/null; then
        sudo apt install -y pkg-config libxkbcommon-dev libxcb-shape0-dev libxcb-xfixes0-dev \
            libwayland-dev libfontconfig-dev libva-dev mesa-common-dev libasound2-dev 2>/dev/null || true
    elif command -v pacman &>/dev/null; then
        sudo pacman -S --noconfirm pkgconf libxkbcommon libxcb wayland fontconfig libva mesa alsa-lib 2>/dev/null || true
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y pkg-config libxkbcommon-devel libxcb-devel wayland-devel \
            fontconfig-devel libva-devel mesa-libGL-devel alsa-lib-devel 2>/dev/null || true
    fi
fi

echo ""
echo "Building TAU (this may take 15-30 minutes)..."
cd "$(dirname "$0")/editor"
cargo build --release --bin tau

echo ""
echo "Installing to ~/.local/bin/tau..."
mkdir -p ~/.local/bin
cp target/release/tau ~/.local/bin/

# Add to PATH if not already
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    SHELL_CONFIG=""
    case "$SHELL" in
        */zsh) SHELL_CONFIG="$HOME/.zshrc" ;;
        */bash) SHELL_CONFIG="$HOME/.bashrc" ;;
    esac
    if [[ -n "$SHELL_CONFIG" ]]; then
        echo 'export PATH="$PATH:$HOME/.local/bin"' >> "$SHELL_CONFIG"
        echo "Added ~/.local/bin to PATH in $SHELL_CONFIG"
    fi
fi

echo ""
echo "=== TAU installed successfully! ==="
echo "Run: tau"
