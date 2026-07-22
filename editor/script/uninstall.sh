#!/usr/bin/env bash
set -e

# ---------- Language detection ----------
LANG_CODE="${LANG:0:2}"
case "$LANG_CODE" in
  id)
    MSG_TITLE="=== Pencopotan TAU Editor ==="
    MSG_CONFIRM="Hapus TAU dan semua file terkait? [Y/n] "
    MSG_CANCELLED="Pencopotan dibatalkan."
    ;;
  *)
    MSG_TITLE="=== TAU Editor Uninstaller ==="
    MSG_CONFIRM="Remove TAU and all related files? [Y/n] "
    MSG_CANCELLED="Uninstall cancelled."
    ;;
esac

echo "$MSG_TITLE"
echo ""

# ---------- Confirmation ----------
if [[ "$1" != "-y" ]] && [[ "$1" != "--yes" ]]; then
  read -rp "$MSG_CONFIRM" CONFIRM
  case "$CONFIRM" in
    [nN][oO]|[nN]) echo "$MSG_CANCELLED"; exit 0 ;;
    *) ;;
  esac
fi

INSTALL_DIR="${HOME}/.local/bin"
TAU_APP_DIR="${HOME}/.local/tau.app"

# Detect OS
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

# ---------- Linux/FreeBSD paths ----------
LINUX_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/tau"
LINUX_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/tau"
LINUX_STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/tau"
LINUX_CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/tau"

# ---------- macOS paths ----------
MACOS_DATA_DIR="$HOME/Library/Application Support/TAU"
MACOS_CONFIG_DIR="$HOME/.config/tau"
MACOS_STATE_DIR="$HOME/.local/state/TAU"
MACOS_CACHE_DIR="$HOME/Library/Caches/TAU"
MACOS_LOGS_DIR="$HOME/Library/Logs/TAU"
MACOS_CRASH_DIR="$HOME/Library/Logs/DiagnosticReports"

# ---------- Common paths ----------
AGENTS_DIR="$HOME/.agents"
COPILOT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/github-copilot"

# ---------- Detect platform ----------
case "$OS" in
  linux|freebsd)
    DATA_DIR="$LINUX_DATA_DIR"
    CONFIG_DIR="$LINUX_CONFIG_DIR"
    STATE_DIR="$LINUX_STATE_DIR"
    CACHE_DIR="$LINUX_CACHE_DIR"
    ;;
  darwin)
    DATA_DIR="$MACOS_DATA_DIR"
    CONFIG_DIR="$MACOS_CONFIG_DIR"
    STATE_DIR="$MACOS_STATE_DIR"
    CACHE_DIR="$MACOS_CACHE_DIR"
    ;;
  *)
    echo "  Unsupported OS: $OS"
    echo "  For Windows, use uninstall.ps1"
    exit 1
    ;;
esac

# ---------- Remove binary ----------
echo "Removing binaries..."
if [ -f "$INSTALL_DIR/tau" ]; then
  rm "$INSTALL_DIR/tau"
  echo "  Removed $INSTALL_DIR/tau"
fi
if [ -f "$INSTALL_DIR/tau.exe" ]; then
  rm "$INSTALL_DIR/tau.exe"
  echo "  Removed $INSTALL_DIR/tau.exe"
fi

# Remove symlink if it exists
if [ -L "$INSTALL_DIR/tau" ]; then
  rm "$INSTALL_DIR/tau"
  echo "  Removed symlink $INSTALL_DIR/tau"
fi

# ---------- Remove app bundle (Linux) ----------
if [ -d "$TAU_APP_DIR" ]; then
  rm -rf "$TAU_APP_DIR"
  echo "  Removed $TAU_APP_DIR"
fi

# ---------- Remove data directory ----------
if [ -d "$DATA_DIR" ]; then
  rm -rf "$DATA_DIR"
  echo "  Removed $DATA_DIR"
fi

# ---------- Remove config directory ----------
if [ -d "$CONFIG_DIR" ]; then
  echo ""
  echo "Remove TAU configuration files in $CONFIG_DIR?"
  echo "  This includes settings, keymaps, themes, AGENTS.md, snippets."
  echo -n "  (y/N): "
  read -r CONFIRM
  if [[ "$CONFIRM" == "y" || "$CONFIRM" == "Y" ]]; then
    rm -rf "$CONFIG_DIR"
    echo "  Removed $CONFIG_DIR"
  else
    echo "  Kept $CONFIG_DIR"
  fi
fi

# ---------- Remove state directory ----------
if [ -d "$STATE_DIR" ]; then
  rm -rf "$STATE_DIR"
  echo "  Removed $STATE_DIR"
fi

# ---------- Remove cache directory ----------
if [ -d "$CACHE_DIR" ]; then
  rm -rf "$CACHE_DIR"
  echo "  Removed $CACHE_DIR"
fi

# ---------- Remove macOS-specific paths ----------
if [[ "$OS" == "darwin" ]]; then
  # Logs
  if [ -d "$MACOS_LOGS_DIR" ]; then
    rm -rf "$MACOS_LOGS_DIR"
    echo "  Removed $MACOS_LOGS_DIR"
  fi

  # Crash reports
  if [ -d "$MACOS_CRASH_DIR" ]; then
    CRASH_FILES=$(find "$MACOS_CRASH_DIR" -name "tau*" -type f 2>/dev/null | head -20)
    if [ -n "$CRASH_FILES" ]; then
      echo "$CRASH_FILES" | while read -r f; do
        rm -f "$f"
        echo "  Removed $f"
      done
    fi
  fi

  # macOS icon
  if [ -f "$HOME/.local/share/icons/tau.svg" ]; then
    rm -f "$HOME/.local/share/icons/tau.svg"
    echo "  Removed $HOME/.local/share/icons/tau.svg"
  fi
fi

# ---------- Remove agent skills ----------
if [ -d "$AGENTS_DIR" ]; then
  echo ""
  echo "Remove global agent skills in $AGENTS_DIR?"
  echo "  This includes custom skills you've created."
  echo -n "  (y/N): "
  read -r CONFIRM
  if [[ "$CONFIRM" == "y" || "$CONFIRM" == "Y" ]]; then
    rm -rf "$AGENTS_DIR"
    echo "  Removed $AGENTS_DIR"
  else
    echo "  Kept $AGENTS_DIR"
  fi
fi

# ---------- Remove Copilot config ----------
if [ -d "$COPILOT_DIR" ]; then
  echo ""
  echo "Remove GitHub Copilot config in $COPILOT_DIR?"
  echo -n "  (y/N): "
  read -r CONFIRM
  if [[ "$CONFIRM" == "y" || "$CONFIRM" == "Y" ]]; then
    rm -rf "$COPILOT_DIR"
    echo "  Removed $COPILOT_DIR"
  else
    echo "  Kept $COPILOT_DIR"
  fi
fi

# ---------- Remove desktop entries ----------
echo ""
echo "Removing desktop entries..."

# Desktop file
DESKTOP_FILE="${HOME}/.local/share/applications/tau.desktop"
if [ -f "$DESKTOP_FILE" ]; then
  rm "$DESKTOP_FILE"
  echo "  Removed $DESKTOP_FILE"
fi

# Icon (hicolor)
ICON_FILE="${HOME}/.local/share/icons/hicolor/scalable/apps/tau.svg"
if [ -f "$ICON_FILE" ]; then
  rm "$ICON_FILE"
  echo "  Removed $ICON_FILE"
fi

# Remove other icon sizes
for SIZE in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
  ICON_PATH="${HOME}/.local/share/icons/hicolor/${SIZE}/apps/tau.png"
  if [ -f "$ICON_PATH" ]; then
    rm "$ICON_PATH"
    echo "  Removed $ICON_PATH"
  fi
done

# Desktop shortcuts
if [ -f "$HOME/Desktop/tau.desktop" ]; then
  rm "$HOME/Desktop/tau.desktop"
  echo "  Removed $HOME/Desktop/tau.desktop"
fi

if [ -f "$HOME/Área de Trabalho/tau.desktop" ]; then
  rm "$HOME/Área de Trabalho/tau.desktop"
  echo "  Removed $HOME/Área de Trabalho/tau.desktop"
fi

# ---------- Remove IPC sockets ----------
for SOCK in "$DATA_DIR/tau-stable.sock" "$DATA_DIR/tau-nightly.sock" "$DATA_DIR/tau-dev.sock" "$DATA_DIR/tau-preview.sock"; do
  if [ -S "$SOCK" ]; then
    rm -f "$SOCK"
    echo "  Removed socket $SOCK"
  fi
done

# ---------- Clean PATH entries from shell configs ----------
echo ""
echo "Cleaning PATH entries from shell configs..."

for CONFIG in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.profile" "$HOME/.bash_profile" "$HOME/.config/fish/config.fish"; do
  if [ -f "$CONFIG" ]; then
    # Remove TAU PATH entries
    sed -i '/# TAU Editor/d' "$CONFIG" 2>/dev/null || true
    sed -i '\|export PATH="\$PATH:'"$INSTALL_DIR"'"|d' "$CONFIG" 2>/dev/null || true

    # For fish, also remove fish-style PATH entries
    if [[ "$CONFIG" == *"fish"* ]]; then
      sed -i '\|set -gx PATH.*'"$INSTALL_DIR"'|d' "$CONFIG" 2>/dev/null || true
    fi
  fi
done

echo "  Cleaned shell configs"

# ---------- Refresh desktop database ----------
if command -v update-desktop-database &>/dev/null; then
  update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons" 2>/dev/null || true
fi

# ---------- Remove from current session PATH ----------
export PATH=$(echo "$PATH" | tr ':' '\n' | grep -v "$INSTALL_DIR" | tr '\n' ':' | sed 's/:$//')

# ---------- Summary ----------
echo ""
echo "=== TAU has been uninstalled ==="
echo ""
echo "Removed:"
echo "  - Binary: $INSTALL_DIR/tau"
echo "  - App bundle: $TAU_APP_DIR (if existed)"
echo "  - Data: $DATA_DIR"
echo "  - State: $STATE_DIR"
echo "  - Cache: $CACHE_DIR"
echo "  - Desktop entries and icons"
echo "  - PATH entries from shell configs"
echo ""
if [ -d "$CONFIG_DIR" ]; then
  echo "Kept: $CONFIG_DIR (user config)"
fi
if [ -d "$AGENTS_DIR" ]; then
  echo "Kept: $AGENTS_DIR (agent skills)"
fi
if [ -d "$COPILOT_DIR" ]; then
  echo "Kept: $COPILOT_DIR (Copilot config)"
fi
echo ""
echo "Note: Per-project .tau/ directories were NOT removed."
echo "      Delete them manually from each project if needed."
echo ""
echo "Close and reopen your terminal for PATH changes to take full effect."
