#!/usr/bin/env bash
set -e

REPO="IkramRamadhan08/TAU-theArtificialUltimate"
VERSION="latest"

echo "=== TAU Editor Installer ==="

# Detect architecture and OS
ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-linux.tar.gz" ;;
      aarch64|arm64) ASSET="tau-aarch64-linux.tar.gz" ;;
      *) echo "Unsupported architecture: $ARCH on Linux"; exit 1 ;;
    esac

    # Install system deps if package manager available
    if command -v apt &>/dev/null; then
      sudo apt install -y libxkbcommon-x11-0 libxcb-cursor0 2>/dev/null || true
    elif command -v pacman &>/dev/null; then
      sudo pacman -S --noconfirm libxkbcommon libxcb wayland fontconfig libva mesa alsa-lib 2>/dev/null || true
    elif command -v dnf &>/dev/null; then
      sudo dnf install -y libxkbcommon libxcb wayland fontconfig libva mesa-libGL alsa-lib 2>/dev/null || true
    fi
    ;;
  darwin)
    ASSET="tau-universal-apple-darwin.tar.gz"
    ;;
  *)
    echo "Unsupported OS: $OS"
    exit 1
    ;;
esac

DOWNLOAD_URL="https://github.com/$REPO/releases/$VERSION/download/$ASSET"
INSTALL_DIR="${HOME}/.local/bin"

echo "Downloading TAU for $OS ($ARCH)..."
mkdir -p "$INSTALL_DIR"
curl -fsSL "$DOWNLOAD_URL" | tar xz -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/tau"

# Add to PATH if not already
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  SHELL_CONFIG=""
  case "$SHELL" in
    */zsh) SHELL_CONFIG="$HOME/.zshrc" ;;
    */bash) SHELL_CONFIG="$HOME/.bashrc" ;;
    */fish) SHELL_CONFIG="$HOME/.config/fish/config.fish" ;;
  esac
  if [[ -n "$SHELL_CONFIG" ]]; then
    echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$SHELL_CONFIG"
    echo "Added $INSTALL_DIR to PATH in $SHELL_CONFIG"
  fi
fi

echo ""
echo "=== TAU installed successfully! ==="
echo "Run: tau"
