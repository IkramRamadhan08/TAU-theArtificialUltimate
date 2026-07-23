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

# ---------- Confirmation ----------
if [[ "$1" != "-y" ]] && [[ "$1" != "--yes" ]]; then
  read -rp "Install TAU to ~/.local? [Y/n] " CONFIRM
  case "$CONFIRM" in
    [nN][oO]|[nN]) echo "Installation cancelled."; exit 0 ;;
    *) ;;
  esac
fi

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
  MSG_DEPS_FAIL="Warning: failed to install some runtime dependencies. TAU may not launch correctly."
  if command -v apt &>/dev/null; then
    sudo apt install -y libxkbcommon-x11-0 libxcb-cursor0 xz-utils 2>/dev/null || echo "  $MSG_DEPS_FAIL"
  elif command -v pacman &>/dev/null; then
    sudo pacman -S --noconfirm libxkbcommon libxcb wayland fontconfig libva mesa alsa-lib xz 2>/dev/null || echo "  $MSG_DEPS_FAIL"
  elif command -v dnf &>/dev/null; then
    sudo dnf install -y libxkbcommon libxcb wayland fontconfig libva mesa-libGL alsa-lib xz 2>/dev/null || echo "  $MSG_DEPS_FAIL"
  fi
fi

# ---------- Download / Build ----------
TAU_APP_DIR="${INSTALL_DIR}/../tau.app"
RELEASE_VERSION=""

# ---------- Download with progress ----------
download_with_progress() {
  local url="$1"
  local output="$2"
  local label="$3"

  # Get file size first
  local total_size
  total_size=$(curl -4 -fsSL --connect-timeout 10 --max-time 10 -I "$url" 2>/dev/null | grep -i content-length | awk '{print $2}' | tr -d '\r' | head -1 || echo "")

  if [[ -z "$total_size" || "$total_size" -eq 0 ]]; then
    # Can't get size, download with simple progress
    echo "  $label (size unknown, downloading...)"
    curl $CURL_OPTS -fsSL --progress-bar -o "$output" "$url"
    return $?
  fi

  # Download with accurate progress bar using #
  local width=40

  echo "  $label"

  # Use curl with progress output to stderr, parse it
  local last_percent=0
  local bar=""

  curl $CURL_OPTS -fsSL -# -o "$output" "$url" 2>&1 | while IFS= read -r line; do
    # curl -# outputs progress like: "\r  ##  12.3M/45.6M  27%"
    if [[ "$line" =~ ([0-9]+)% ]]; then
      local percent="${BASH_REMATCH[1]}"
      if [[ "$percent" -ne "$last_percent" ]]; then
        last_percent=$percent
        local filled=$((percent * width / 100))
        bar=""
        for (( i=0; i<filled; i++ )); do
          bar="${bar}#"
        done
        for (( i=filled; i<width; i++ )); do
          bar="${bar}-"
        done
        printf "\r  [$bar] %3d%%" "$percent"
      fi
    fi
  done

  local exit_code=${PIPESTATUS[0]}

  # Final state
  if [[ $exit_code -eq 0 ]] && [[ -f "$output" ]]; then
    local final_size
    final_size=$(stat -c%s "$output" 2>/dev/null || echo 0)
    local bar=""
    for (( i=0; i<width; i++ )); do
      bar="${bar}#"
    done
    printf "\r  [$bar] 100%% ✓ (%s)\n" "$(numfmt --to=iec $final_size)"
  else
    printf "\r  Download failed ✗\n"
  fi

  return $exit_code
}

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
    download_with_progress "$DOWNLOAD_URL" "/tmp/tau-download.tar.xz" "Downloading TAU for Linux ($ARCH)"
    tar xJ -f /tmp/tau-download.tar.xz -C "$(dirname "$TAU_APP_DIR")"
    rm -f /tmp/tau-download.tar.xz

    # Find the actual binary in the extracted bundle
    # Prefer bin/tau (CLI launcher) over libexec/tau-editor (editor binary).
    # The CLI launcher handles IPC, single-instance detection, and proper
    # terminal behavior. Linking directly to the editor binary bypasses all
    # of that and causes the terminal to be killed on launch.
    TAU_BINARY=""
    if [[ -f "$TAU_APP_DIR/bin/tau" ]]; then
      TAU_BINARY="$TAU_APP_DIR/bin/tau"
    elif [[ -f "$TAU_APP_DIR/libexec/tau-editor" ]]; then
      TAU_BINARY="$TAU_APP_DIR/libexec/tau-editor"
    elif [[ -f "$TAU_APP_DIR/tau-editor" ]]; then
      TAU_BINARY="$TAU_APP_DIR/tau-editor"
    elif [[ -f "$TAU_APP_DIR/tau" ]]; then
      TAU_BINARY="$TAU_APP_DIR/tau"
    fi

    if [[ -z "$TAU_BINARY" ]]; then
      echo "  Error: could not find TAU binary in extracted bundle"
      echo "  Contents of $TAU_APP_DIR:"
      ls -la "$TAU_APP_DIR" 2>/dev/null || true
      exit 1
    fi

    chmod +x "$TAU_BINARY"
    ln -sf "$TAU_BINARY" "$INSTALL_DIR/tau"
    echo "  Linked $TAU_BINARY -> $INSTALL_DIR/tau"
  elif [[ "$OS" == "darwin" ]]; then
    download_with_progress "$DOWNLOAD_URL" "/tmp/tau.tar.gz" "Downloading TAU for macOS ($ARCH)"
    tar xzf /tmp/tau.tar.gz -C /tmp
    BINARY=$(ls /tmp/tau-*-macos 2>/dev/null | head -1)
    if [[ -n "$BINARY" ]]; then
      cp "$BINARY" "$INSTALL_DIR/tau"
      chmod +x "$INSTALL_DIR/tau"
    else
      echo "  Error: could not find binary in archive"
      echo "  Contents of /tmp after extraction:"
      ls -la /tmp/tau-* 2>/dev/null || true
      exit 1
    fi
    rm -f /tmp/tau.tar.gz /tmp/tau-*-macos
  elif [[ "$OS" == "windows" ]]; then
    download_with_progress "$DOWNLOAD_URL" "/tmp/tau.zip" "Downloading TAU for Windows"
    unzip -o /tmp/tau.zip -d /tmp/tau-install
    BINARY=$(ls /tmp/tau-install/*.exe 2>/dev/null | head -1)
    if [[ -n "$BINARY" ]]; then
      mv "$BINARY" "$INSTALL_DIR/tau.exe"
      chmod +x "$INSTALL_DIR/tau.exe"
    else
      echo "  Error: could not find binary in archive"
      echo "  Contents of /tmp/tau-install:"
      ls -la /tmp/tau-install/ 2>/dev/null || true
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
if [[ "$OS" == "windows" ]]; then
  TAU_BIN="$INSTALL_DIR/tau.exe"
else
  TAU_BIN="$INSTALL_DIR/tau"
fi

if [[ -f "$TAU_BIN" ]]; then
  if [[ "$OS" != "windows" ]] && [[ ! -x "$TAU_BIN" ]]; then
    chmod +x "$TAU_BIN"
  fi
  # Test that binary actually runs
  if "$TAU_BIN" --version >/dev/null 2>&1; then
    VERSION_OUTPUT=$("$TAU_BIN" --version 2>/dev/null || echo "")
    if [[ -n "$VERSION_OUTPUT" ]]; then
      echo "  $MSG_VERIFY_OK ($VERSION_OUTPUT)"
    else
      echo "  $MSG_VERIFY_OK"
    fi
  else
    # Binary exists but --version fails (maybe it needs display or deps)
    echo "  $MSG_VERIFY_OK"
    echo "  (Note: '$TAU_BIN --version' failed, but binary exists)"
  fi
else
  echo "  $MSG_VERIFY_FAIL"
  echo "  $TAU_BIN"
  exit 1
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
Exec=$INSTALL_DIR/tau --foreground %F
Icon=tau
Categories=Utility;TextEditor;Development;IDE;
Keywords=tau;agent;code;ide;
MimeType=text/plain;application/x-zerosize;x-scheme-handler/tau;
StartupNotify=false
Actions=NewWorkspace;

[Desktop Action NewWorkspace]
Exec=$INSTALL_DIR/tau --foreground --new %F
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
    if [[ "$SHELL" == */fish ]]; then
      echo "set -gx PATH \$PATH $INSTALL_DIR" >> "$SHELL_CONFIG"
    else
      echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$SHELL_CONFIG"
    fi
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
