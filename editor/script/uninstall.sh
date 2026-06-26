#!/usr/bin/env sh
set -eu

# Uninstalls TAU that was installed using the install.sh script

check_remaining_installations() {
    platform="$(uname -s)"
    if [ "$platform" = "Darwin" ]; then
        # Check for any TAU variants in /Applications
        remaining=$(ls -d /Applications/TAU*.app 2>/dev/null | wc -l)
        [ "$remaining" -eq 0 ]
    else
        # Check for any TAU variants in ~/.local
        remaining=$(ls -d "$HOME/.local/tau"*.app 2>/dev/null | wc -l)
        [ "$remaining" -eq 0 ]
    fi
}

prompt_remove_preferences() {
    printf "Do you want to keep your TAU preferences? [Y/n] "
    read -r response
    case "$response" in
        [nN]|[nN][oO])
            rm -rf "$HOME/.config/tau"
            echo "Preferences removed."
            ;;
        *)
            echo "Preferences kept."
            ;;
    esac
}

main() {
    platform="$(uname -s)"
    channel="${ZED_CHANNEL:-stable}"

    if [ "$platform" = "Darwin" ]; then
        platform="macos"
    elif [ "$platform" = "Linux" ]; then
        platform="linux"
    else
        echo "Unsupported platform $platform"
        exit 1
    fi

    "$platform"

    echo "TAU has been uninstalled"
}

linux() {
    suffix=""
    if [ "$channel" != "stable" ]; then
        suffix="-$channel"
    fi

    appid=""
    db_suffix="stable"
    case "$channel" in
      stable)
        appid="ai.tau.TAU"
        db_suffix="stable"
        ;;
      nightly)
        appid="ai.tau.TAU-Nightly"
        db_suffix="nightly"
        ;;
      preview)
        appid="ai.tau.TAU-Preview"
        db_suffix="preview"
        ;;
      dev)
        appid="ai.tau.TAU-Dev"
        db_suffix="dev"
        ;;
      *)
        echo "Unknown release channel: ${channel}. Using stable app ID."
        appid="ai.tau.TAU"
        db_suffix="stable"
        ;;
    esac

    # Remove the app directory
    rm -rf "$HOME/.local/tau$suffix.app"

    # Remove the binary symlink
    rm -f "$HOME/.local/bin/tau"
    rm -f "$HOME/.local/bin/TAU"

    # Remove the .desktop file
    rm -f "$HOME/.local/share/applications/${appid}.desktop"

    # Remove the database directory for this channel
    rm -rf "$HOME/.local/share/tau/db/0-$db_suffix"

    # Remove socket file
    rm -f "$HOME/.local/share/tau/tau-$db_suffix.sock"

    # Remove the entire TAU directory if no installations remain
    if check_remaining_installations; then
        rm -rf "$HOME/.local/share/tau"
        prompt_remove_preferences
    fi

    rm -rf "$HOME/.tau_server"
}

macos() {
    app="TAU.app"
    db_suffix="stable"
    app_id="ai.tau.TAU"
    case "$channel" in
      nightly)
        app="TAU Nightly.app"
        db_suffix="nightly"
        app_id="ai.tau.TAU-Nightly"
        ;;
      preview)
        app="TAU Preview.app"
        db_suffix="preview"
        app_id="ai.tau.TAU-Preview"
        ;;
      dev)
        app="TAU Dev.app"
        db_suffix="dev"
        app_id="ai.tau.TAU-Dev"
        ;;
    esac

    # Remove the app bundle
    if [ -d "/Applications/$app" ]; then
        rm -rf "/Applications/$app"
    fi

    # Remove the binary symlink
    rm -f "$HOME/.local/bin/tau"
    rm -f "$HOME/.local/bin/TAU"

    # Remove the database directory for this channel
    rm -rf "$HOME/Library/Application Support/TAU/db/0-$db_suffix"

    # Remove app-specific files and directories
    rm -rf "$HOME/Library/Application Support/com.apple.sharedfilelist/com.apple.LSSharedFileList.ApplicationRecentDocuments/$app_id.sfl"*
    rm -rf "$HOME/Library/Caches/$app_id"
    rm -rf "$HOME/Library/HTTPStorages/$app_id"
    rm -rf "$HOME/Library/Preferences/$app_id.plist"
    rm -rf "$HOME/Library/Saved Application State/$app_id.savedState"

    # Remove the entire TAU directory if no installations remain
    if check_remaining_installations; then
        rm -rf "$HOME/Library/Application Support/TAU"
        rm -rf "$HOME/Library/Logs/TAU"

        prompt_remove_preferences
    fi

    rm -rf "$HOME/.tau_server"
}

main "$@"
