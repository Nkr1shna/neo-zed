---
title: Uninstall
description: "This guide covers how to uninstall Zed on different operating systems."
---

# Uninstall

This guide covers how to uninstall Zed on different operating systems.

## macOS

### Standard Installation

If you installed Zed by downloading it from the website:

1. Quit Zed if it's running
2. Open Finder and go to your Applications folder
3. Drag Zed to the Trash (or right-click and select "Move to Trash")
4. Empty the Trash

### Homebrew Installation

If you installed Zed using Homebrew, use the following command:

```sh
brew uninstall --cask zed
```

Or for the preview version:

```sh
brew uninstall --cask zed@preview
```

### Removing User Data (Optional)

To completely remove all Neo Zed configuration files and data:

1. Open Finder
2. Press `Cmd + Shift + G` to open "Go to Folder"
3. Delete the following directories if they exist:
   - `~/Library/Application Support/Neo Zed`
   - `~/Library/Saved Application State/dev.neozed.savedState`
   - `~/Library/Logs/Neo Zed`
   - `~/Library/Caches/dev.neozed`
   - `~/Library/Caches/Neo Zed`
   - `~/.config/neozed`
   - `~/.local/state/Neo Zed`

## Linux

### Standard Uninstall

If Neo Zed was installed using the default installation script, run:

```sh
neozed --uninstall
```

You'll be prompted whether to keep or delete your preferences. After making a choice, you should see a message that Zed was successfully uninstalled.

If the `neozed` command is not found in your PATH, try:

```sh
$HOME/.local/bin/neozed --uninstall
```

or:

```sh
$HOME/.local/neozed.app/bin/neozed --uninstall
```

### Package Manager

If you installed Zed using a package manager (such as Flatpak, Snap, or a distribution-specific package manager), consult that package manager's documentation for uninstallation instructions.

### Manual Removal

If the uninstall command fails or Zed was installed to a custom location, you can manually remove:

- Installation directory: `~/.local/neozed.app` (or your custom installation path)
- Binary symlink: `~/.local/bin/neozed`
- Configuration and data: `~/.config/neozed`

## Windows

### Standard Installation

1. Quit Zed if it's running
2. Open Settings (Windows key + I)
3. Go to "Apps" > "Installed apps" (or "Apps & features" on Windows 10)
4. Search for "Zed"
5. Click the three dots menu next to Zed and select "Uninstall"
6. Follow the prompts to complete the uninstallation

Alternatively, you can:

1. Open the Start menu
2. Right-click on Zed
3. Select "Uninstall"

### Removing User Data (Optional)

To completely remove all Neo Zed configuration files and data:

1. Press `Windows key + R` to open Run
2. Type `%APPDATA%` and press Enter
3. Delete the `Neo Zed` folder if it exists
4. Press `Windows key + R` again, type `%LOCALAPPDATA%` and press Enter
5. Delete the `Neo Zed` folder if it exists

## Troubleshooting

If you encounter issues during uninstallation:

- **macOS/Windows**: Ensure Neo Zed is completely quit before attempting to uninstall. Check Activity Monitor (macOS) or Task Manager (Windows) for any running Neo Zed processes.
- **Linux**: If the uninstall script fails, check the error message and consider manual removal of the directories listed above.
- **All platforms**: If you want to start fresh while keeping Neo Zed installed, you can delete the configuration directories instead of uninstalling the application entirely.

For additional help, see our [Linux-specific documentation](./linux.md) or visit the [Zed community](https://zed.dev/community-links).
