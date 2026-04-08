# Plugin Runtime

Plugin-platform facts and constraints workers should keep in mind.

## What belongs here

- Plugin manifest and host/runtime boundary facts
- Constraints on discoverability versus execution
- Mission-specific facts about fixture design and observability

## Current platform shape

- Plugin manifests currently describe panels and titlebar widgets.
- Plugin UI is mirrored through `gpui_api` / `plugin_protocol` and reconstructed by `plugin_host`.
- `plugin_host` owns process startup, surface binding, and remote session lifecycle.
- Command palette and keymap discovery today are built around statically registered GPUI actions.

## Mission constraints

- Canvas work should extend the mirrored runtime instead of bypassing it.
- Zoom work should integrate with workspace-owned zoom state instead of inventing a plugin-only fullscreen mode.
- Action discovery must come from metadata that exists before plugin startup.
- Action execution must route through runtime handler registration and lazy plugin startup.
- The mission needs a dev plugin fixture that exposes:
  - a visible canvas surface,
  - a titlebar widget opening a panel,
  - a clearly labeled nested zoom target,
  - at least one manifest-declared action with runtime handler,
  - observable counters or logs for startup/session/action execution.

## Useful verification targets

- `crates/gpui_plugin/tests/interactive_surface.rs`
- `crates/gpui_plugin/tests/mirror_runtime.rs`
- `crates/ui_plugin/tests/ui_surface.rs`
- `crates/plugin_host/tests/plugin_host.rs`
- `crates/command_palette/src/command_palette.rs`
- `crates/workspace/src/workspace.rs`
- `crates/keymap_editor/src/keymap_editor.rs`
- `script/check-keymaps`

## Validation limitation

- Current automation cannot attach to the native GPUI surface in-session. Workers must keep the dev plugin fixture and host-log/session/action counters usable for later manual user validation.
