#!/usr/bin/env bash
set -e

REPO="IkramRamadhan08/TAU-theArtificialUltimate"
VERSION="latest"
RAW_BASE="https://raw.githubusercontent.com/$REPO/main"

# ---------- Language detection ----------
LANG_CODE="${LANG:0:2}"
case "$LANG_CODE" in
  id)
    MSG_TITLE="=== Pemasang TAU Editor ==="
    MSG_ARCH="Arsitektur tidak didukung"
    MSG_OS="Sistem operasi tidak didukung"
    MSG_DOWNLOAD="Mengunduh TAU untuk"
    MSG_DESKTOP_INSTALL="Memasang ikon dan pintasan desktop..."
    MSG_DESKTOP_DONE="Ikon dan pintasan desktop terpasang"
    MSG_ICON_INSTALL="Memasang ikon..."
    MSG_PATH_ADD="Menambahkan ke PATH di"
    MSG_SUCCESS="TAU berhasil dipasang!"
    MSG_LAUNCH_DESKTOP="Klik ikon TAU di menu aplikasi atau desktop."
    MSG_LAUNCH_TERMINAL="Ketik 'tau' di terminal untuk menjalankan."
    MSG_DESKTOP_NOTE="Terminal akan tertutup otomatis dan TAU akan muncul."
    MSG_ICON_FAIL="Peringatan: gagal mengunduh ikon"
    MSG_VERIFY="Memverifikasi pemasangan..."
    MSG_VERIFY_OK="TAU siap digunakan!"
    MSG_VERIFY_FAIL="Peringatan: TAU tidak dapat dijalankan. Coba jalankan manual:"
    ;;
  *)
    MSG_TITLE="=== TAU Editor Installer ==="
    MSG_ARCH="Unsupported architecture"
    MSG_OS="Unsupported OS"
    MSG_DOWNLOAD="Downloading TAU for"
    MSG_DESKTOP_INSTALL="Installing desktop icon and shortcut..."
    MSG_DESKTOP_DONE="Desktop icon and shortcut installed"
    MSG_ICON_INSTALL="Installing icon..."
    MSG_PATH_ADD="Added to PATH in"
    MSG_SUCCESS="TAU installed successfully!"
    MSG_LAUNCH_DESKTOP="Click the TAU icon in your app menu or desktop."
    MSG_LAUNCH_TERMINAL="Type 'tau' in a terminal to launch."
    MSG_DESKTOP_NOTE="The terminal will close automatically and TAU will appear."
    MSG_ICON_FAIL="Warning: could not download icon"
    MSG_VERIFY="Verifying installation..."
    MSG_VERIFY_OK="TAU is ready to use!"
    MSG_VERIFY_FAIL="Warning: TAU may not work correctly. Try running manually:"
    ;;
esac

echo "$MSG_TITLE"

ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-linux.tar.xz" ;;
      aarch64|arm64) ASSET="tau-aarch64-linux.tar.xz" ;;
      *) echo "$MSG_ARCH: $ARCH"; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      arm64|aarch64) ASSET="tau-aarch64-macos.tar.gz" ;;
      x86_64)
        echo "  Intel Mac detected. Using ARM64 build via Rosetta 2."
        echo "  (For native performance, use an ARM64 Mac.)"
        ASSET="tau-aarch64-macos.tar.gz"
        ;;
      *) echo "$MSG_ARCH: $ARCH"; exit 1 ;;
    esac
    ;;
  mingw*|msys*|cygwin*)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-windows.zip" ;;
      *) echo "$MSG_ARCH: $ARCH"; exit 1 ;;
    esac
    OS="windows"
    ;;
  *)
    echo "$MSG_OS: $OS"
    exit 1
    ;;
esac

DOWNLOAD_URL="https://github.com/$REPO/releases/$VERSION/download/$ASSET"

INSTALL_DIR="${HOME}/.local/bin"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"
APP_DIR="${HOME}/.local/share/applications"
DESKTOP_FILE="$APP_DIR/tau.desktop"
mkdir -p "$INSTALL_DIR" "$ICON_DIR" "$APP_DIR"

# ---------- Install runtime deps ----------
if [[ "$OS" == "linux" ]]; then
  if command -v apt &>/dev/null; then
    sudo apt install -y libxkbcommon-x11-0 libxcb-cursor0 xz-utils 2>/dev/null || true
  elif command -v pacman &>/dev/null; then
    sudo pacman -S --noconfirm libxkbcommon libxcb wayland fontconfig libva mesa alsa-lib xz 2>/dev/null || true
  elif command -v dnf &>/dev/null; then
    sudo dnf install -y libxkbcommon libxcb wayland fontconfig libva mesa-libGL alsa-lib xz 2>/dev/null || true
  fi
fi

# ---------- Download / Build ----------
TAU_APP_DIR="${INSTALL_DIR}/../tau.app"
RELEASE_VERSION=""

# Try IPv4 first, fallback to IPv6
CURL_OPTS="--progress-bar --connect-timeout 15 --max-time 600"
if curl -4 -fsSL --connect-timeout 5 --max-time 5 -o /dev/null -w '' "https://github.com" 2>/dev/null; then
  CURL_OPTS="-4 $CURL_OPTS"
elif curl -6 -fsSL --connect-timeout 5 --max-time 5 -o /dev/null -w '' "https://github.com" 2>/dev/null; then
  CURL_OPTS="-6 $CURL_OPTS"
else
  CURL_OPTS="-4 $CURL_OPTS"
fi

if curl $CURL_OPTS -fsSL -I "$DOWNLOAD_URL" 2>/dev/null; then
  echo "$MSG_DOWNLOAD $OS ($ARCH)..."

  # Get version from redirect URL
  REDIRECT_URL=$(curl -4 -fsSL --connect-timeout 15 --max-time 15 -o /dev/null -w '%{redirect_url}' "$DOWNLOAD_URL" 2>/dev/null || echo "")
  if [[ -n "$REDIRECT_URL" ]]; then
    RELEASE_VERSION=$(echo "$REDIRECT_URL" | grep -oP 'tag/v\K[0-9]+\.[0-9]+' | head -1)
  fi

  if [[ "$OS" == "linux" ]]; then
    mkdir -p "$TAU_APP_DIR"
    curl $CURL_OPTS -fsSL "$DOWNLOAD_URL" | tar xJ -C "$(dirname "$TAU_APP_DIR")"
    chmod +x "$TAU_APP_DIR/libexec/tau-editor" 2>/dev/null || true
    ln -sf "$TAU_APP_DIR/libexec/tau-editor" "$INSTALL_DIR/tau"
  elif [[ "$OS" == "darwin" ]]; then
    curl $CURL_OPTS -fsSL -o /tmp/tau.tar.gz "$DOWNLOAD_URL"
    tar xzf /tmp/tau.tar.gz -C /tmp
    BINARY=$(ls /tmp/tau-*-macos 2>/dev/null | head -1)
    if [[ -n "$BINARY" ]]; then
      cp "$BINARY" "$INSTALL_DIR/tau"
      chmod +x "$INSTALL_DIR/tau"
    else
      echo "  Error: could not find binary in archive"
      exit 1
    fi
    rm -f /tmp/tau.tar.gz /tmp/tau-*-macos
  elif [[ "$OS" == "windows" ]]; then
    curl $CURL_OPTS -fsSL -o /tmp/tau.zip "$DOWNLOAD_URL"
    unzip -o /tmp/tau.zip -d /tmp/tau-install
    BINARY=$(ls /tmp/tau-install/*.exe 2>/dev/null | head -1)
    if [[ -n "$BINARY" ]]; then
      mv "$BINARY" "$INSTALL_DIR/tau.exe"
      chmod +x "$INSTALL_DIR/tau.exe"
    else
      echo "  Error: could not find binary in archive"
      exit 1
    fi
    rm -rf /tmp/tau.zip /tmp/tau-install
  fi
else
  echo "  $MSG_VERIFY_FAIL"
  echo "  Tidak ada binary yang tersedia untuk platform ini."
  echo "  Install dari: https://github.com/$REPO/releases"
  exit 1
fi

# ---------- Verify binary ----------
echo "  $MSG_VERIFY"
if [[ -f "$INSTALL_DIR/tau" ]]; then
  echo "  $MSG_VERIFY_OK"
else
  echo "  $MSG_VERIFY_FAIL"
  echo "  $INSTALL_DIR/tau"
fi

# ---------- Desktop shortcut (always create if display available) ----------
HAS_DISPLAY=false
if [[ -n "$DISPLAY" || -n "$WAYLAND_DISPLAY" ]]; then
  HAS_DISPLAY=true
fi

if $HAS_DISPLAY; then
  echo "  $MSG_DESKTOP_INSTALL"

  ICON_URL="$RAW_BASE/editor/crates/tau/resources/tau-icon.svg"
  if curl -4 -fsSL --max-time 15 "$ICON_URL" -o "$ICON_DIR/tau.svg" 2>/dev/null; then
    echo "    $MSG_ICON_INSTALL"
  else
    echo "    $MSG_ICON_FAIL"
  fi

  cat > "$DESKTOP_FILE" << DESKTOP
[Desktop Entry]
Version=1.0
Type=Application
Name=TAU
GenericName=AI Code Editor
Comment=The Artificial Ultimate local agentic coding IDE.
TryExec=$INSTALL_DIR/tau
Exec=$INSTALL_DIR/tau %F
Icon=tau
Categories=Utility;TextEditor;Development;IDE;
Keywords=tau;agent;code;ide;
MimeType=text/plain;application/x-zerosize;x-scheme-handler/tau;
StartupNotify=false
Actions=NewWorkspace;

[Desktop Action NewWorkspace]
Exec=$INSTALL_DIR/tau --new %F
Name=Open a new workspace
DESKTOP

  DESKTOP_SCREEN="$HOME/Desktop/tau.desktop"
  if [[ -d "$HOME/Desktop" ]]; then
    cp "$DESKTOP_FILE" "$DESKTOP_SCREEN"
    chmod +x "$DESKTOP_SCREEN"
  elif [[ -d "$HOME/Área de Trabalho" ]]; then
    cp "$DESKTOP_FILE" "$HOME/Área de Trabalho/tau.desktop"
    chmod +x "$HOME/Área de Trabalho/tau.desktop"
  fi

  if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$APP_DIR" 2>/dev/null || true
  fi
  if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons" 2>/dev/null || true
  fi

  echo "  $MSG_DESKTOP_DONE"
fi

# ---------- macOS icon ----------
if [[ "$OS" == "darwin" ]] && $HAS_DISPLAY; then
  ICON_URL="$RAW_BASE/editor/crates/tau/resources/tau-icon.svg"
  mkdir -p "$HOME/.local/share/icons"
  curl -4 -fsSL --max-time 15 "$ICON_URL" -o "$HOME/.local/share/icons/tau.svg" 2>/dev/null || true
fi

# ---------- Add to PATH (shell rc) ----------
SHELL_CONFIG=""
case "$SHELL" in
  */zsh) SHELL_CONFIG="$HOME/.zshrc" ;;
  */bash) SHELL_CONFIG="$HOME/.bashrc" ;;
  */fish) SHELL_CONFIG="$HOME/.config/fish/config.fish" ;;
esac

# Check if ~/.local/bin is already in PATH (via any method)
ALREADY_IN_PATH=false
if [[ ":$PATH:" == *":$INSTALL_DIR:"* ]]; then
  ALREADY_IN_PATH=true
fi

if [[ -n "$SHELL_CONFIG" ]] && ! grep -q "$INSTALL_DIR" "$SHELL_CONFIG" 2>/dev/null; then
  if $ALREADY_IN_PATH; then
    echo "  $INSTALL_DIR already in PATH (via system config)"
  else
    echo "" >> "$SHELL_CONFIG"
    echo "# TAU Editor" >> "$SHELL_CONFIG"
    echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$SHELL_CONFIG"
    echo "  $MSG_PATH_ADD $SHELL_CONFIG"
  fi
fi

# ---------- Export PATH for CURRENT session ----------
export PATH="$PATH:$INSTALL_DIR"

# ---------- Done ----------
echo ""
echo "  $MSG_SUCCESS"
if [[ -n "$RELEASE_VERSION" ]]; then
  echo "  Version: v$RELEASE_VERSION"
fi
echo ""

if $HAS_DISPLAY; then
  echo "  $MSG_LAUNCH_DESKTOP"
else
  echo "  $MSG_LAUNCH_TERMINAL"
  echo "  $MSG_DESKTOP_NOTE"
fi
echo ""
echo "  Tip: You can immediately use 'tau' in this terminal."
echo "       New terminals will also have 'tau' available."
echo ""
