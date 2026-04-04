---
title: Developing Plugins
description: "Create Zed plugins with out-of-process GPUI panels and title bar widgets."
---

# Developing Plugins {#developing-plugins}

Zed plugins are directories containing a `plugin.toml` manifest. They can provide dock panels and title bar widgets.

Plugins run out of process.

- On macOS, Zed launches plugins through a host-managed sandbox with plugin-scoped writable paths. Development plugins still receive the writable paths they need for Cargo builds, plugin state, temp files, and localhost callbacks.
- On Linux, Zed launches plugins with `PR_SET_NO_NEW_PRIVS` and a seccomp filter that blocks mount, namespace, tracing, and kernel-instrumentation syscalls.
- On Windows, Zed launches plugins in dedicated Job Objects so the host can contain and tear down the full plugin process tree.

## Plugin Features {#plugin-features}

Plugins can provide:

- Panels
- Title Bar Widgets

## Developing a Plugin Locally

Before starting to develop a plugin for Zed, be sure to install Rust and Cargo.

When developing a plugin, you can use it in Zed without needing to publish it by installing it as a _dev plugin_.

From the Plugins page, click the `Install Dev Plugin` button (or the {#action zed::InstallDevPlugin} action) and select the directory containing your plugin.

If you need to troubleshoot, check Zed.log ({#action zed::OpenLog}) for additional output. For debug output, close and relaunch Zed from the command line with `zed --foreground`, which shows more verbose INFO-level logs.

## Directory Structure of a Zed Plugin

A Zed plugin is a directory that contains a `plugin.toml`. This file must contain some basic information about the plugin:

```toml
id = "my-plugin"
name = "My Plugin"
version = "0.0.1"
schema_version = 1
authors = ["Your Name <you@example.com>"]
description = "Example plugin"
repository = "https://github.com/your-name/my-zed-plugin"
homepage = "https://github.com/your-name/my-zed-plugin"
entry = "my-plugin"

[[panels]]
id = "my-plugin-panel"
title = "My Plugin"
dock = "right"
tooltip = "Open the My Plugin panel."
activation = "on_demand"

[[titlebar_widgets]]
id = "my-plugin-widget"
title = "My Plugin"
tooltip = "Open the My Plugin panel."
side = "right"
priority = 100
opens_panel_id = "my-plugin-panel"
```

Panels can be placed in the `left`, `right`, or `bottom` dock, and can use either `on_demand` or `on_startup` activation. Title bar widgets can be placed on the `left` or `right` side. If `opens_panel_id` is set, clicking the widget opens the referenced plugin panel.

An example directory structure of a plugin is as follows:

```
my-plugin/
  plugin.toml
  Cargo.toml
  src/
    main.rs
```

## Building Plugin UI

Plugin UI is written in Rust and runs in a separate process. To use the GPUI mirror runtime, add `gpui_plugin` and `ui_plugin` as dependencies and alias them to `gpui` and `ui`:

```toml
[package]
name = "my-plugin"
version = "0.0.1"
edition = "2021"

[features]
default = []
mirror = ["dep:gpui", "dep:ui"]

[[bin]]
name = "my-plugin"
path = "src/main.rs"

[dependencies]
anyhow = "1"
gpui = { package = "gpui_plugin", path = "../../crates/gpui_plugin", optional = true }
ui = { package = "ui_plugin", path = "../../crates/ui_plugin", optional = true }
```

This example assumes the plugin lives under the Zed repository's `plugins/` directory. If your plugin lives elsewhere, update the dependency paths accordingly.

When Zed launches a Cargo-backed plugin, it runs `cargo run --features mirror --bin <entry>` from the plugin directory. If your plugin is not a Cargo project, `entry` can point to a relative executable path instead.

Within plugin code, `use gpui::*;` and `use ui::*;` work the same way as in-process UI code. The plugin runtime mirrors those APIs and sends the resulting element tree back to Zed.

In `src/main.rs`, register each panel and title bar widget declared in `plugin.toml`:

```rs
use anyhow::Result;
use gpui::*;
use ui::*;

struct MyPluginPanel;

impl Render for MyPluginPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_4()
            .gap_2()
            .child(Label::new("Hello from a plugin"))
    }
}

struct MyPluginWidget;

impl Render for MyPluginWidget {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().gap_1().child(Label::new("My Plugin"))
    }
}

fn main() -> Result<()> {
    gpui::run(|plugin| {
        plugin.register_panel("my-plugin-panel", |_cx| MyPluginPanel);
        plugin.register_titlebar_widget("my-plugin-widget", |_cx| MyPluginWidget);
    })
}
```

Each `[[panels]]` and `[[titlebar_widgets]]` entry in `plugin.toml` must also be registered in code.
