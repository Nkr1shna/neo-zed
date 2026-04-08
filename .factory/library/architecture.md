# Architecture

How plugin canvas, plugin zoom, and plugin actions should fit into Neo Zed.

## What belongs here

- High-level relationships between plugin manifests, plugin runtime mirroring, plugin host rendering, workspace zoom, and action/keymap systems
- Behavioral invariants that workers must preserve while extending the plugin platform
- Likely ownership boundaries across crates

## System model

### 1. Plugin surfaces are host-rendered remote views
- Plugin UI is authored in plugin code through `gpui_plugin` and `ui_plugin`.
- The plugin runtime mirrors its UI into serializable data in `gpui_api` and `plugin_protocol`.
- `plugin_host` owns process lifecycle, panel/titlebar bindings, event routing, and host-side rendering.
- Today the mirrored surface is a constrained remote widget tree; this mission extends that model without regressing current mirrored controls.
- Remote surface nodes that participate in input routing, zoom targeting, or action dispatch must expose stable host-recognized identity across rerender and restore; behavior must not depend on transient tree order alone.

### 2. Manifest metadata and runtime behavior are separate concerns
- `plugin.toml` / `PluginManifest` describe discoverable user-facing metadata.
- Runtime registration in the plugin process provides executable handlers and live UI behavior.
- This separation is required for plugin actions: metadata must be visible before startup, but handlers only exist once the plugin process is ready.

### 3. Canvas parity requires extending the remote-surface protocol, not bypassing it
- Native GPUI canvas is callback-driven drawing tied to host `Window` rendering.
- Plugins cannot receive direct host `Window` access; instead they need a mirrored canvas abstraction that can round-trip rendering intent and input through the existing plugin/runtime boundary.
- The host must remain the renderer of record for plugin surfaces.
- Existing mirrored widget nodes and event semantics must continue to work beside the new canvas surface.

### 4. Zoom remains workspace-owned
- The `workspace` crate owns zoom state and the `Shift-Escape` flow.
- Native behavior today zooms whole panes or whole dock panels.
- Plugin zoom support must integrate with that same workspace-owned zoom state rather than introducing a plugin-local fullscreen mode.
- Child-target zoom therefore needs a stable protocol-visible host-understood target identity inside the plugin-rendered surface, not only a plugin-private reference.

### 5. Plugin actions should use a host proxy layer
- GPUI actions are statically registered Rust action types and drive command-palette and keymap discovery.
- Plugin-defined actions should not mutate the static action registry dynamically per action type.
- Preferred shape: host-owned proxy action plumbing plus plugin metadata for discoverability and runtime handler registration for execution.
- Plugin action execution must remain compatible with GPUI's static action registry, keymap parsing/building, and command discovery infrastructure rather than inventing a parallel dispatch path.
- Lazy startup belongs in `plugin_host`; discovery must not start the plugin, but invocation may.

## Core invariants

1. Existing plugin panels and titlebar widgets keep working while new canvas capability is added.
2. Plugin canvases remain host-rendered remote surfaces; plugin code does not take direct ownership of native windows.
3. A plugin surface may have one zoomed target at a time, coordinated by workspace zoom state.
4. Pane zoom and plugin zoom are mutually exclusive under the existing workspace zoom model.
5. Plugin actions are discoverable before plugin startup but only executable once a runtime handler is available.
6. Action discovery must not start the plugin process.
7. Titlebar, panel, command-palette, keybinding, and restore entry paths must reattach to the same live plugin session and surface bindings when possible rather than spawning duplicate processes or duplicate panel instances.
8. Workspace restore, dock reopen, and rerender paths must not duplicate remote plugin panels or leave stale handlers behind.

## Likely code ownership

### Plugin metadata and protocol
- `crates/plugin/src/plugin.rs`
- `crates/plugin_protocol/src/plugin_protocol.rs`
- `crates/gpui_api/src/gpui_api.rs`
- `crates/gpui_plugin/src/gpui_plugin.rs`
- `crates/ui_plugin/src/ui_plugin.rs`

### Host rendering and process/session lifecycle
- `crates/plugin_host/src/plugin_host.rs`
- `crates/plugin_host/tests/plugin_host.rs`

### Workspace zoom integration
- `crates/workspace/src/workspace.rs`
- `crates/workspace/src/dock.rs`
- `crates/workspace/src/pane.rs`

### Actions, command palette, and keymaps
- `crates/gpui/src/action.rs`
- `crates/gpui/src/app.rs`
- `crates/gpui/src/window.rs`
- `crates/command_palette/src/command_palette.rs`
- `crates/command_palette_hooks/src/command_palette_hooks.rs`
- `crates/settings/src/keymap_file.rs`
- `crates/keymap_editor/src/keymap_editor.rs`
- `script/check-keymaps`

## Worker guidance

- Treat the plugin protocol boundary as the hardest seam in this mission.
- Prefer small, explicit protocol and metadata changes that preserve backward compatibility for existing plugin surfaces.
- Whenever a feature claims “same plugin session” or “no duplicate panel”, verify it through host lifecycle evidence, not only visible UI.
- For manual validation, use a purpose-built dev plugin fixture with labeled nested zoom targets and observable session/action counters so cross-area assertions are actually testable.
