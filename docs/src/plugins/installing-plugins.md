---
title: Installing Plugins
description: "Install and manage plugins from the Plugins page."
---

# Installing Plugins {#installing-plugins}

Plugins add functionality to Zed, including dock panels and title bar widgets.

Open the Plugins page with {#kb zed::Plugins}, or select "Zed > Plugins" from the menu bar.

Zed launches plugins out of process.

- On macOS, plugins run inside a host-managed sandbox with plugin-scoped writable paths.
- On Linux, plugins run with `PR_SET_NO_NEW_PRIVS`, a Landlock filesystem sandbox, and a seccomp filter that blocks mount, namespace, tracing, and kernel-instrumentation syscalls.
- On Windows, plugins run inside AppContainers with plugin-scoped filesystem access, and Zed also places them in dedicated Job Objects so the process tree is torn down cleanly with the host.

Plugins still run with your user account and can make network requests. Only install plugins you trust.

To install a plugin you are developing locally, click the `Install Dev Plugin` button (or the {#action zed::InstallDevPlugin} action) and select the directory containing your plugin.

Use the search field to filter the list, or switch between the `All`, `Installed`, and `Development` filters.

## Installation Location

- On macOS, plugins are installed in `~/Library/Application Support/Zed/plugins`.
- On Linux, they are installed in either `$XDG_DATA_HOME/zed/plugins` or `~/.local/share/zed/plugins`.
- On Windows, the directory is `%LOCALAPPDATA%\\Zed\\plugins`.

This directory contains two subdirectories:

- `installed`, which contains the managed copy of each installed plugin.
- `development`, which contains registrations for development plugins installed from local directories.
