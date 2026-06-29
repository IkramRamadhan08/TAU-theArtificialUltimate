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
    MSG_BUILD="Tidak ada binary siap pakai. Membangun dari sumber..."
    MSG_RUST_REQUIRED="Rust diperlukan untuk membangun TAU dari sumber."
    MSG_RUST_INSTALL="Pasang dari: https://rustup.rs"
    MSG_RUN_AGAIN="Jalankan script ini lagi setelah Rust terpasang."
    MSG_BUILD_WAIT="Membangun TAU (ini akan memakan waktu)..."
    MSG_DEPS_INSTALL="Memasang dependensi sistem..."
    MSG_DESKTOP_ASK="Ingin menampilkan TAU di desktop?"
    MSG_DESKTOP_YES="y"
    MSG_DESKTOP_NO="n"
    MSG_DESKTOP_INSTALL="Memasang ikon dan pintasan desktop..."
    MSG_DESKTOP_DONE="Ikon dan pintasan desktop terpasang"
    MSG_ICON_INSTALL="Memasang ikon..."
    MSG_PATH_ADD="Menambahkan ke PATH di"
    MSG_SUCCESS="=== TAU v0.62 terpasang! ==="
    MSG_LAUNCH_DESKTOP="Klik dua kali ikon TAU di desktop untuk menjalankan."
    MSG_LAUNCH_TERMINAL="Ketik 'tau' di terminal untuk menjalankan."
    MSG_LAUNCH_WINDOWS="Jalankan 'tau' dari Command Prompt atau PowerShell."
    MSG_DESKTOP_NOTE="Terminal akan tertutup otomatis dan TAU akan muncul."
    MSG_CHOICE="Pilihan"
    MSG_INVALID="Pilihan tidak valid. Gunakan"
    MSG_ICON_FAIL="Peringatan: gagal mengunduh ikon"
    ;;
  *)
    MSG_TITLE="=== TAU Editor Installer ==="
    MSG_ARCH="Unsupported architecture"
    MSG_OS="Unsupported OS"
    MSG_DOWNLOAD="Downloading TAU for"
    MSG_BUILD="No pre-built binary. Building from source..."
    MSG_RUST_REQUIRED="Rust is required to build TAU from source."
    MSG_RUST_INSTALL="Install from: https://rustup.rs"
    MSG_RUN_AGAIN="Run this script again after Rust is installed."
    MSG_BUILD_WAIT="Building TAU (this will take a while)..."
    MSG_DEPS_INSTALL="Installing system dependencies..."
    MSG_DESKTOP_ASK="Show TAU on your desktop?"
    MSG_DESKTOP_YES="y"
    MSG_DESKTOP_NO="n"
    MSG_DESKTOP_INSTALL="Installing desktop icon and shortcut..."
    MSG_DESKTOP_DONE="Desktop icon and shortcut installed"
    MSG_ICON_INSTALL="Installing icon..."
    MSG_PATH_ADD="Added to PATH in"
    MSG_SUCCESS="=== TAU v0.62 installed! ==="
    MSG_LAUNCH_DESKTOP="Double-click the TAU icon on your desktop to launch."
    MSG_LAUNCH_TERMINAL="Type 'tau' in a terminal to launch."
    MSG_LAUNCH_WINDOWS="Run 'tau' from Command Prompt or PowerShell."
    MSG_DESKTOP_NOTE="The terminal will close automatically and TAU will appear."
    MSG_CHOICE="Choice"
    MSG_INVALID="Invalid choice. Use"
    MSG_ICON_FAIL="Warning: could not download icon"
    ;;
esac

echo "$MSG_TITLE"

ARCH="$(uname -m)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

case "$OS" in
  linux)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-linux.tar.gz" ;;
      aarch64|arm64) ASSET="tau-aarch64-linux.tar.gz" ;;
      *) echo "$MSG_ARCH: $ARCH"; exit 1 ;;
    esac
    ;;
  darwin)
    case "$ARCH" in
      x86_64) ASSET="tau-x86_64-macos.tar.gz" ;;
      arm64|aarch64) ASSET="tau-aarch64-macos.tar.gz" ;;
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
    sudo apt install -y libxkbcommon-x11-0 libxcb-cursor0 2>/dev/null || true
  elif command -v pacman &>/dev/null; then
    sudo pacman -S --noconfirm libxkbcommon libxcb wayland fontconfig libva mesa alsa-lib 2>/dev/null || true
  elif command -v dnf &>/dev/null; then
    sudo dnf install -y libxkbcommon libxcb wayland fontconfig libva mesa-libGL alsa-lib 2>/dev/null || true
  fi
fi

# ---------- Download / Build ----------
TAU_APP_DIR="${INSTALL_DIR}/../tau.app"

if curl -fsSL --connect-timeout 15 --max-time 60 -o /dev/null "$DOWNLOAD_URL" 2>/dev/null; then
  echo "$MSG_DOWNLOAD $OS ($ARCH)..."

  if [[ "$OS" == "linux" ]]; then
    mkdir -p "$TAU_APP_DIR"
    curl -fsSL --max-time 600 "$DOWNLOAD_URL" | tar xz -C "$(dirname "$TAU_APP_DIR")"
    chmod +x "$TAU_APP_DIR/libexec/tau-editor" 2>/dev/null || true
    ln -sf "$TAU_APP_DIR/libexec/tau-editor" "$INSTALL_DIR/tau"
  elif [[ "$OS" == "darwin" ]]; then
    curl -fsSL --max-time 600 -o /tmp/tau.tar.gz "$DOWNLOAD_URL"
    tar xzf /tmp/tau.tar.gz -C /tmp
    cp /tmp/tau-x86_64-macos /tmp/tau-aarch64-macos "$INSTALL_DIR/tau" 2>/dev/null
    chmod +x "$INSTALL_DIR/tau"
    rm -f /tmp/tau.tar.gz /tmp/tau-x86_64-macos /tmp/tau-aarch64-macos
  elif [[ "$OS" == "windows" ]]; then
    curl -fsSL --max-time 600 "$DOWNLOAD_URL" -o /tmp/tau.zip
    unzip -o /tmp/tau.zip -d "$INSTALL_DIR"
    rm /tmp/tau.zip
    chmod +x "$INSTALL_DIR/tau" 2>/dev/null || true
  fi
else
  echo "$MSG_BUILD"
  if ! command -v cargo &>/dev/null; then
    echo "$MSG_RUST_REQUIRED"
    echo "$MSG_RUST_INSTALL"
    echo ""
    echo "$MSG_RUN_AGAIN"
    exit 1
  fi

  echo "$MSG_DEPS_INSTALL"
  if [[ "$OS" == "darwin" ]]; then
    if command -v brew &>/dev/null; then
      brew install fontconfig 2>/dev/null || true
    fi
  fi

  TMP_DIR="$(mktemp -d)"
  git clone --depth 1 "https://github.com/$REPO.git" "$TMP_DIR"
  cd "$TMP_DIR/editor"

  echo "$MSG_BUILD_WAIT"
  cargo build --release --bin tau --jobs "$(nproc 2>/dev/null || echo 4)"

  if [[ "$OS" == "linux" ]]; then
    mkdir -p "$TAU_APP_DIR/libexec"
    cp "target/release/tau" "$TAU_APP_DIR/libexec/tau-editor"
    ln -sf "$TAU_APP_DIR/libexec/tau-editor" "$INSTALL_DIR/tau"
  else
    cp "target/release/tau" "$INSTALL_DIR/tau"
  fi
  rm -rf "$TMP_DIR"
fi

# ---------- Ask desktop shortcut ----------
DESKTOP_CHOICE=""
if [ -t 0 ]; then
  case "$LANG_CODE" in
    id)
      echo -n "$MSG_DESKTOP_ASK (y/n): "
      read -r DESKTOP_CHOICE
      ;;
    *)
      echo -n "$MSG_DESKTOP_ASK (y/n): "
      read -r DESKTOP_CHOICE
      ;;
  esac
fi
# Non-interactive (piped) or empty response defaults to no

if [[ "$DESKTOP_CHOICE" == "$MSG_DESKTOP_YES" || "$DESKTOP_CHOICE" == "y" || "$DESKTOP_CHOICE" == "Y" ]]; then
  # ---------- Install icon to applications ----------
  echo "$MSG_DESKTOP_INSTALL"

  ICON_URL="$RAW_BASE/editor/crates/tau/resources/tau-icon.svg"
  if curl -fsSL --max-time 15 "$ICON_URL" -o "$ICON_DIR/tau.svg" 2>/dev/null; then
    echo "  $MSG_ICON_INSTALL"
  else
    echo "  $MSG_ICON_FAIL"
  fi

  # Applications menu entry
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

  # Desktop shortcut
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
if [[ "$OS" == "darwin" && "$DESKTOP_CHOICE" == "y" ]]; then
  ICON_URL="$RAW_BASE/editor/crates/tau/resources/tau-icon.svg"
  mkdir -p "$HOME/.local/share/icons"
  curl -fsSL --max-time 15 "$ICON_URL" -o "$HOME/.local/share/icons/tau.svg" 2>/dev/null || true
fi

# ---------- Add to PATH ----------
SHELL_CONFIG=""
case "$SHELL" in
  */zsh) SHELL_CONFIG="$HOME/.zshrc" ;;
  */bash) SHELL_CONFIG="$HOME/.bashrc" ;;
  */fish) SHELL_CONFIG="$HOME/.config/fish/config.fish" ;;
esac

if [[ -n "$SHELL_CONFIG" ]] && ! grep -q "$INSTALL_DIR" "$SHELL_CONFIG" 2>/dev/null; then
  echo "" >> "$SHELL_CONFIG"
  echo "# TAU Editor" >> "$SHELL_CONFIG"
  echo "export PATH=\"\$PATH:$INSTALL_DIR\"" >> "$SHELL_CONFIG"
  echo "$MSG_PATH_ADD $SHELL_CONFIG"
fi

echo ""
echo "$MSG_SUCCESS"
echo ""

if [[ "$DESKTOP_CHOICE" == "$MSG_DESKTOP_YES" || "$DESKTOP_CHOICE" == "y" || "$DESKTOP_CHOICE" == "Y" ]]; then
  echo "$MSG_LAUNCH_DESKTOP"
else
  echo "$MSG_LAUNCH_TERMINAL"
  echo "$MSG_DESKTOP_NOTE"
fi
echo ""
