#!/usr/bin/env bash
set -e

echo "=== TAU Editor Uninstaller ==="

INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/tau"
AGENTS_DIR="${HOME}/.agents"

# Remove binary
if [ -f "$INSTALL_DIR/tau" ]; then
  rm "$INSTALL_DIR/tau"
  echo "  Removed $INSTALL_DIR/tau"
fi

# Remove agent skills
if [ -d "$AGENTS_DIR" ]; then
  rm -rf "$AGENTS_DIR"
  echo "  Removed $AGENTS_DIR"
fi

# Remove desktop/app entries
DESKTOP_FILE="${HOME}/.local/share/applications/tau.desktop"
if [ -f "$DESKTOP_FILE" ]; then
  rm "$DESKTOP_FILE"
  echo "  Removed $DESKTOP_FILE"
fi

# Remove icon
ICON_FILE="${HOME}/.local/share/icons/hicolor/scalable/apps/tau.svg"
if [ -f "$ICON_FILE" ]; then
  rm "$ICON_FILE"
  echo "  Removed $ICON_FILE"
fi

# Remove desktop shortcuts
if [ -f "$HOME/Desktop/tau.desktop" ]; then
  rm "$HOME/Desktop/tau.desktop"
  echo "  Removed $HOME/Desktop/tau.desktop"
fi

if [ -f "$HOME/Área de Trabalho/tau.desktop" ]; then
  rm "$HOME/Área de Trabalho/tau.desktop"
  echo "  Removed $HOME/Área de Trabalho/tau.desktop"
fi

# Clean PATH entries from shell configs
for CONFIG in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.config/fish/config.fish"; do
  if [ -f "$CONFIG" ]; then
    sed -i '/# TAU Editor/d' "$CONFIG" 2>/dev/null || true
    sed -i '\|export PATH="\$PATH:'"$INSTALL_DIR"'"|d' "$CONFIG" 2>/dev/null || true
  fi
done

# Remove config (ask first)
if [ -d "$CONFIG_DIR" ]; then
  echo ""
  echo "Remove TAU configuration files in $CONFIG_DIR?"
  echo -n "This includes settings, keymaps, themes. (y/N): "
  read -r CONFIRM
  if [[ "$CONFIRM" == "y" || "$CONFIRM" == "Y" ]]; then
    rm -rf "$CONFIG_DIR"
    echo "  Removed $CONFIG_DIR"
  else
    echo "  Kept $CONFIG_DIR"
  fi
fi

# Refresh desktop database
if command -v update-desktop-database &>/dev/null; then
  update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache &>/dev/null; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons" 2>/dev/null || true
fi

echo ""
echo "=== TAU has been uninstalled. ==="
echo "Config files kept at: $CONFIG_DIR (remove manually if needed)"
