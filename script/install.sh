#!/usr/bin/env sh
set -eu

# Downloads a tarball from https://zed.dev/releases and unpacks it
# into ~/.local/. If you'd prefer to do this manually, instructions are at
# https://zed.dev/docs/linux.

main() {
    platform="$(uname -s)"
    arch="$(uname -m)"
    channel="${ZED_CHANNEL:-stable}"
    ZED_VERSION="${ZED_VERSION:-latest}"
    # Use TMPDIR if available (for environments with non-standard temp directories)
    if [ -n "${TMPDIR:-}" ] && [ -d "${TMPDIR}" ]; then
        temp="$(mktemp -d "$TMPDIR/neozed-XXXXXX")"
    else
        temp="$(mktemp -d "/tmp/neozed-XXXXXX")"
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

    if [ "$(command -v neozed)" = "$HOME/.local/bin/neozed" ]; then
        echo "Neo Zed has been installed. Run with 'neozed'"
    else
        echo "To run Neo Zed from your terminal, you must add ~/.local/bin to your PATH"
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

        echo "To run Neo Zed now, '~/.local/bin/neozed'"
    fi
}

linux() {
    archive_path="$temp/neozed-linux-$arch.tar.gz"
    if [ -n "${ZED_BUNDLE_PATH:-}" ]; then
        cp "$ZED_BUNDLE_PATH" "$archive_path"
    else
        echo "Downloading Neo Zed version: $ZED_VERSION"
        curl "https://cloud.zed.dev/releases/$channel/$ZED_VERSION/download?asset=zed&arch=$arch&os=linux&source=install.sh" > "$archive_path"
    fi

    suffix=""
    if [ "$channel" != "stable" ]; then
        suffix="-$channel"
    fi

    appid=""
    case "$channel" in
      stable)
        appid="dev.neozed"
        ;;
      nightly)
        appid="dev.neozed.Nightly"
        ;;
      preview)
        appid="dev.neozed.Preview"
        ;;
      dev)
        appid="dev.neozed.Dev"
        ;;
      *)
        echo "Unknown release channel: ${channel}. Using stable app ID."
        appid="dev.neozed"
        ;;
    esac

    app_dir="$HOME/.local/neozed$suffix.app"
    bundle_root="$(tar -tzf "$archive_path" | awk -F/ 'NF && $1 != "." { print $1; exit }')"
    if [ -z "$bundle_root" ]; then
        echo "Could not determine Neo Zed bundle directory"
        exit 1
    fi

    # Unpack
    rm -rf "$app_dir"
    tar -xzf "$archive_path" -C "$HOME/.local/"
    if [ "$HOME/.local/$bundle_root" != "$app_dir" ]; then
        mv "$HOME/.local/$bundle_root" "$app_dir"
    fi

    # Setup ~/.local directories
    mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications"

    bundled_cli_path=""
    if [ -x "$app_dir/bin/neozed" ]; then
        bundled_cli_path="$app_dir/bin/neozed"
    elif [ -x "$app_dir/bin/cli" ]; then
        bundled_cli_path="$app_dir/bin/cli"
    else
        for candidate in "$app_dir"/bin/*; do
            [ -x "$candidate" ] || continue
            bundled_cli_path="$candidate"
            break
        done
    fi
    if [ -z "$bundled_cli_path" ]; then
        echo "Could not determine Neo Zed CLI path"
        exit 1
    fi

    if [ "$bundled_cli_path" != "$app_dir/bin/neozed" ]; then
        ln -sf "$bundled_cli_path" "$app_dir/bin/neozed"
    fi
    ln -sf "$app_dir/bin/neozed" "$HOME/.local/bin/neozed"

    # Copy .desktop file
    desktop_file_path="$HOME/.local/share/applications/${appid}.desktop"
    src_dir="$app_dir/share/applications"
    desktop_source=""
    if [ -f "$src_dir/${appid}.desktop" ]; then
        desktop_source="$src_dir/${appid}.desktop"
    else
        for candidate in "$src_dir"/*.desktop; do
            [ -f "$candidate" ] || continue
            desktop_source="$candidate"
            break
        done
    fi
    if [ -z "$desktop_source" ]; then
        echo "Could not determine Neo Zed desktop file"
        exit 1
    fi
    cp "$desktop_source" "${desktop_file_path}"

    icon_dir="$app_dir/share/icons/hicolor/512x512/apps"
    icon_path="$icon_dir/neozed.png"
    if [ ! -f "$icon_path" ]; then
        for candidate in "$icon_dir"/*; do
            [ -f "$candidate" ] || continue
            ln -sf "$candidate" "$icon_path"
            break
        done
    fi
    if [ -f "$icon_path" ]; then
        sed -i "s|^Icon=.*$|Icon=$icon_path|g" "${desktop_file_path}"
    fi
    sed -i "s|^Exec=.*$|Exec=$app_dir/bin/neozed|g" "${desktop_file_path}"
    sed -i "s|^TryExec=.*$|TryExec=$app_dir/bin/neozed|g" "${desktop_file_path}"
}

macos() {
    dmg_path="$temp/Neo-Zed-$arch.dmg"
    echo "Downloading Neo Zed version: $ZED_VERSION"
    curl "https://cloud.zed.dev/releases/$channel/$ZED_VERSION/download?asset=zed&os=macos&arch=$arch&source=install.sh" > "$dmg_path"
    hdiutil attach -quiet "$dmg_path" -mountpoint "$temp/mount"
    app="$(cd "$temp/mount/"; echo *.app)"
    echo "Installing Neo Zed"
    if [ -d "/Applications/$app" ]; then
        echo "Removing existing Neo Zed app bundle"
        rm -rf "/Applications/$app"
    fi
    ditto "$temp/mount/$app" "/Applications/$app"
    hdiutil detach -quiet "$temp/mount"

    mkdir -p "$HOME/.local/bin"
    cli_path="/Applications/$app/Contents/MacOS/neozed"
    if [ ! -f "$cli_path" ]; then
        cli_path="/Applications/$app/Contents/MacOS/cli"
    fi
    ln -sf "$cli_path" "$HOME/.local/bin/neozed"
}

main "$@"
