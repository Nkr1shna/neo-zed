---
title: Installing Plugins
description: "Install and manage plugins from the Plugins page."
---

# Installing Plugins {#installing-plugins}

Plugins add functionality to Zed, including dock panels and title bar widgets.

Open the Plugins page with {#kb zed::Plugins}, or select "Zed > Plugins" from the menu bar.

On macOS, Zed launches plugins inside a host-managed sandbox that limits writable paths while still allowing network access, localhost callbacks, and the plugin runtime itself. On other platforms, plugins currently run with the same user, filesystem, network, and environment access as Zed. Only install plugins you trust.

To install a plugin you are developing locally, click the `Install Dev Plugin` button (or the {#action zed::InstallDevPlugin} action) and select the directory containing your plugin.

Use the search field to filter the list, or switch between the `All`, `Installed`, and `Development` filters.

## Installation Location

- On macOS, plugins are installed in `~/Library/Application Support/Zed/plugins`.
- On Linux, they are installed in either `$XDG_DATA_HOME/zed/plugins` or `~/.local/share/zed/plugins`.
- On Windows, the directory is `%LOCALAPPDATA%\\Zed\\plugins`.

This directory contains two subdirectories:

- `installed`, which contains the managed copy of each installed plugin.
- `development`, which contains registrations for development plugins installed from local directories.
