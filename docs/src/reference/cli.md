---
title: CLI Reference
description: "Reference for Neo Zed's command-line interface (CLI), including opening files and directories, integrating with other tools, and controlling Neo Zed from scripts."
---

# CLI Reference

Use Neo Zed's command-line interface (CLI) to open files and directories, integrate with other tools, and control Neo Zed from scripts.

## Installation

**macOS:** Run the `cli: install` command from the command palette ({#kb command_palette::Toggle}) to install the `neozed` CLI to `/usr/local/bin/neozed`.

**Linux:** The CLI is included with Neo Zed packages. The binary name is `neozed` for official releases.

**Windows:** The CLI is included with Neo Zed. Add Neo Zed's installation directory to your PATH, or use the full path to `neozed.exe`.

## Usage

```sh
neozed [OPTIONS] [PATHS]...
```

## Opening Files and Directories

Open a file:

```sh
neozed myfile.txt
```

Open a directory as a workspace:

```sh
neozed ~/projects/myproject
```

Open multiple files or directories:

```sh
neozed file1.txt file2.txt ~/projects/myproject
```

Open a file at a specific line and column:

```sh
neozed myfile.txt:42        # Open at line 42
neozed myfile.txt:42:10     # Open at line 42, column 10
```

## Options

### `-w`, `--wait`

Wait for all opened files to be closed before the CLI exits. When opening a directory, waits until the window is closed.

This is useful for integrating Neo Zed with tools that expect an editor to block until editing is complete (e.g., `git commit`):

```sh
export EDITOR="neozed --wait"
git commit  # Opens Neo Zed and waits for you to close the commit message file
```

### `-n`, `--new`

Open paths in a new workspace window, even if the paths are already open in an existing window:

```sh
neozed -n ~/projects/myproject
```

### `-a`, `--add`

Add paths to the currently focused workspace instead of opening a new window. When multiple workspace windows are open, files open in the focused window:

```sh
neozed -a newfile.txt
```

### `-r`, `--reuse`

Reuse an existing window, replacing its current workspace with the new paths:

```sh
neozed -r ~/projects/different-project
```

### `--diff <OLD_PATH> <NEW_PATH>`

Open a diff view comparing two files. Can be specified multiple times:

```sh
neozed --diff file1.txt file2.txt
neozed --diff old.rs new.rs --diff old2.rs new2.rs
```

### `--foreground`

Run Neo Zed in the foreground, keeping the terminal attached. Useful for debugging:

```sh
neozed --foreground
```

### `--user-data-dir <DIR>`

Use a custom directory for all user data (database, extensions, logs) instead of the default location:

```sh
neozed --user-data-dir ~/.neozed-custom
```

Default locations:

- **macOS:** `~/Library/Application Support/Neo Zed`
- **Linux:** `$XDG_DATA_HOME/neozed` (typically `~/.local/share/neozed`)
- **Windows:** `%LOCALAPPDATA%\Neo Zed`

### `-v`, `--version`

Print Neo Zed's version and exit:

```sh
neozed --version
```

### `--uninstall`

Uninstall Neo Zed and remove all related files (macOS and Linux only):

```sh
neozed --uninstall
```

### `--neozed <PATH>`

Specify a custom path to the Neo Zed application or binary:

```sh
neozed --neozed /path/to/Neo\ Zed.app myfile.txt
```

The legacy `--zed` alias still works for compatibility.

## Reading from Standard Input

Read content from stdin by passing `-` as the path:

```sh
echo "Hello, World!" | neozed -
cat myfile.txt | neozed -
ps aux | neozed -
```

This creates a temporary file with the stdin content and opens it in Neo Zed.

## URL Handling

The CLI can open `neozed://`, `http://`, and `https://` URLs:

```sh
neozed neozed://settings
neozed https://github.com/Nkr1shna/neo-zed
```

## Using Neo Zed as Your Default Editor

Set Neo Zed as your default editor for Git and other tools:

```sh
export EDITOR="neozed --wait"
export VISUAL="neozed --wait"
```

Add these lines to your shell configuration file (e.g., `~/.bashrc`, `~/.zshrc`).

## macOS: Switching Release Channels

On macOS, you can launch a specific release channel by passing the channel name as the first argument:

```sh
neozed --stable myfile.txt
neozed --preview myfile.txt
neozed --nightly myfile.txt
```

## WSL Integration (Windows)

On Windows, the CLI supports opening paths from WSL distributions. This is handled automatically when launching Neo Zed from within WSL.

## Exit Codes

| Code | Meaning                           |
| ---- | --------------------------------- |
| `0`  | Success                           |
| `1`  | Error (details printed to stderr) |

When using `--wait`, the exit code reflects whether the files were saved before closing.
