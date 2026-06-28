#!/usr/bin/env bash
set -e

REPO="IkramRamadhan08/TAU-theArtificialUltimate"
VERSION="latest"

echo "=== TAU Editor Installer ==="

# Detect architecture and OS
ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

# Map to asset name
case "$OS" in
  linux)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-linux.tar.gz" ;;
      aarch64|arm64) ASSET="tau-aarch64-linux.tar.gz" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-macos.tar.gz" ;;
      arm64|aarch64) ASSET="tau-aarch64-macos.tar.gz" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    ;;
  mingw*|msys*|cygwin*)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-windows.zip" ;;
      *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    OS="windows"
    ;;
  *)
    echo "Unsupported OS: $OS"
    echo "See https://github.com/$REPO for build instructions."
    exit 1
    ;;
esac

DOWNLOAD_URL="https://github.com/$REPO/releases/$VERSION/download/$ASSET"

# Install runtime deps (Linux)
if [[ "$OS" == "linux" ]]; then
  if command -v apt &>/dev/null; then
    sudo apt install -y libxkbcommon-x11-0 libxcb-cursor0 2>/dev/null || true
  elif command -v pacman &>/dev/null; then
    sudo pacman -S --noconfirm libxkbcommon libxcb wayland fontconfig libva mesa alsa-lib 2>/dev/null || true
  elif command -v dnf &>/dev/null; then
    sudo dnf install -y libxkbcommon libxcb wayland fontconfig libva mesa-libGL alsa-lib 2>/dev/null || true
  fi
fi

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"

# Try pre-built binary first, fall back to building from source
if curl -fsSL -o /dev/null --max-time 10 "$DOWNLOAD_URL" 2>/dev/null; then
  echo "Downloading TAU for $OS ($ARCH)..."
  if [[ "$OS" == "windows" ]]; then
    curl -fsSL "$DOWNLOAD_URL" -o /tmp/tau.zip
    unzip -o /tmp/tau.zip -d "$INSTALL_DIR"
    rm /tmp/tau.zip
  else
    curl -fsSL "$DOWNLOAD_URL" | tar xz -C "$INSTALL_DIR"
  fi
  chmod +x "$INSTALL_DIR/tau" 2>/dev/null || true
else
  echo "No pre-built binary for $OS ($ARCH). Building from source..."
  if ! command -v cargo &>/dev/null; then
    echo "Rust is required to build TAU from source."
    echo "Install it from: https://rustup.rs"
    echo ""
    echo "Then run this script again."
    exit 1
  fi

  echo "Installing system dependencies..."
  if [[ "$OS" == "darwin" ]]; then
    if command -v brew &>/dev/null; then
      brew install fontconfig 2>/dev/null || true
    fi
  fi

  TMP_DIR="$(mktemp -d)"
  git clone --depth 1 "https://github.com/$REPO.git" "$TMP_DIR"
  cd "$TMP_DIR/editor"

  echo "Building TAU (this will take a while)..."
  cargo build --release --bin tau --jobs "$(nproc 2>/dev/null || echo 4)"

  cp "target/release/tau" "$INSTALL_DIR/tau"
  rm -rf "$TMP_DIR"
fi

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
