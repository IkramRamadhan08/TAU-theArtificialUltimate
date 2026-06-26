#!/usr/bin/env sh
set -eu

# Installs a TAU tarball and unpacks it
# into ~/.local/. If you'd prefer to do this manually, instructions are at
# the TAU release notes.

main() {
    platform="$(uname -s)"
    arch="$(uname -m)"
    channel="${ZED_CHANNEL:-stable}"
    ZED_VERSION="${ZED_VERSION:-latest}"
    # Use TMPDIR if available (for environments with non-standard temp directories)
    if [ -n "${TMPDIR:-}" ] && [ -d "${TMPDIR}" ]; then
        temp="$(mktemp -d "$TMPDIR/tau-XXXXXX")"
    else
        temp="$(mktemp -d "/tmp/tau-XXXXXX")"
    fi

    if [ "$platform" = "Darwin" ]; then
        platform="macos"
    elif [ "$platform" = "Linux" ]; then
        platform="linux"
    else
        echo "Unsupported platform $platform"
        exit 1
    fi

    case "$platform-$arch" in
        macos-arm64* | linux-arm64* | linux-armhf | linux-aarch64)
            arch="aarch64"
            ;;
        macos-x86* | linux-x86* | linux-i686*)
            arch="x86_64"
            ;;
        *)
            echo "Unsupported platform or architecture"
            exit 1
            ;;
    esac

    if command -v curl >/dev/null 2>&1; then
        curl () {
            command curl -fL "$@"
        }
    elif command -v wget >/dev/null 2>&1; then
        curl () {
            wget -O- "$@"
        }
    else
        echo "Could not find 'curl' or 'wget' in your path"
        exit 1
    fi

    "$platform" "$@"

    tau_path="$(command -v tau || true)"
    if [ "$tau_path" = "$HOME/.local/bin/tau" ]; then
        echo "TAU has been installed. Run with 'TAU'"
    else
        echo "To run TAU from your terminal, you must add ~/.local/bin to your PATH"
        echo "Run:"

        case "$SHELL" in
            *zsh)
                echo "   echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.zshrc"
                echo "   source ~/.zshrc"
                ;;
            *fish)
                echo "   fish_add_path -U $HOME/.local/bin"
                ;;
            *)
                echo "   echo 'export PATH=\$HOME/.local/bin:\$PATH' >> ~/.bashrc"
                echo "   source ~/.bashrc"
                ;;
        esac

        echo "To run TAU now, '~/.local/bin/TAU'"
    fi
}

linux() {
    if [ -n "${ZED_BUNDLE_PATH:-}" ]; then
        cp "$ZED_BUNDLE_PATH" "$temp/tau-linux-$arch.tar.gz"
    else
        echo "Set ZED_BUNDLE_PATH to a TAU Linux tarball before running this installer."
        exit 1
    fi

    suffix=""
    if [ "$channel" != "stable" ]; then
        suffix="-$channel"
    fi

    appid=""
    case "$channel" in
      stable)
        appid="ai.tau.TAU"
        ;;
      nightly)
        appid="ai.tau.TAU-Nightly"
        ;;
      preview)
        appid="ai.tau.TAU-Preview"
        ;;
      dev)
        appid="ai.tau.TAU-Dev"
        ;;
      *)
        echo "Unknown release channel: ${channel}. Using stable app ID."
        appid="ai.tau.TAU"
        ;;
    esac

    # Unpack
    rm -rf "$HOME/.local/tau$suffix.app"
    mkdir -p "$HOME/.local/tau$suffix.app"
    tar -xzf "$temp/tau-linux-$arch.tar.gz" -C "$HOME/.local/"

    # Setup ~/.local directories
    mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"

    # Link the binary
    if [ -f "$HOME/.local/tau$suffix.app/bin/tau" ]; then
        ln -sf "$HOME/.local/tau$suffix.app/bin/tau" "$HOME/.local/bin/tau"
        ln -sf "$HOME/.local/tau$suffix.app/bin/tau" "$HOME/.local/bin/TAU"
    else
        echo "TAU binary missing from tarball"
        exit 1
    fi

    # Copy .desktop file
    desktop_file_path="$HOME/.local/share/applications/${appid}.desktop"
    src_dir="$HOME/.local/tau$suffix.app/share/applications"
    if [ -f "$src_dir/${appid}.desktop" ]; then
        cp "$src_dir/${appid}.desktop" "${desktop_file_path}"
    else
        echo "TAU desktop file missing from tarball"
        exit 1
    fi
    sed -i "s|Icon=tau|Icon=$HOME/.local/tau$suffix.app/share/icons/hicolor/512x512/apps/tau.png|g" "${desktop_file_path}"
    sed -i "s|Exec=tau|Exec=$HOME/.local/tau$suffix.app/bin/tau|g" "${desktop_file_path}"
}

macos() {
    echo "macOS install is not wired for TAU public releases yet."
    exit 1
}

main "$@"
