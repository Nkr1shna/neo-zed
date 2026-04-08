# User Testing

Validation surface findings and execution guidance for the plugin canvas / zoom / actions mission.

## Validation Surface

### 1. Manual native desktop plugin validation
- Tool: later manual user validation in the local Neo Zed desktop app
- Surface: the native GPUI app with a purpose-built development plugin fixture
- Primary user-facing checks:
  - plugin canvas rendering and interaction
  - titlebar widget rendering and panel opening
  - `Shift-Escape` plugin zoom, including nested child-target zoom
  - command-palette discovery and invocation of plugin actions
  - user keybinding assignment and invocation of plugin actions
  - restored-workspace and multi-entry coherence

### 2. Focused Rust validation
- Tools: `cargo test`, `./script/check-keymaps`, `./script/clippy`, `cargo check`
- Supporting checks:
  - mirror runtime and plugin-host event/rerender behavior
  - workspace zoom regressions
  - command palette regressions
  - keymap validation and editor regressions

## Validation Concurrency

### Surface: desktop plugin flow validation
- Max concurrent validators: **1**
- Rationale: plugin install state, keymap edits, workspace restore state, zoom state, and desktop window focus are all shared mutable resources. Parallel desktop validators would interfere with each other and make evidence unreliable.

### Surface: focused Rust validation
- Max concurrent validators: **2**
- Rationale: the machine has 12 CPU cores and ample RAM, but the workspace shares a heavy target directory and cold compiles for `plugin_host`, `command_palette`, and `workspace` are expensive. Two concurrent validators keep contention manageable.

## Dry-run findings

The validation path is executable in this environment. Representative commands that passed during planning:
- `cargo test -p gpui_plugin --test interactive_surface -- --exact gpui_plugin_exposes_interactive_surface`
- `cargo test -p gpui_plugin --test mirror_runtime -- --exact mirror_runtime_dispatches_non_click_events`
- `cargo test -p ui_plugin --test ui_surface`
- `cargo test -p plugin_host --test plugin_host`
- `cargo test -p command_palette tests::test_command_palette -- --exact`
- `cargo test -p workspace tests::test_pane_zoom_in_out -- --exact`
- `./script/check-keymaps`

Resource notes from the dry run:
- `plugin_host` cold-path testing peaked around 3.5 GB RSS and ~98s real time.
- `command_palette` cold-path testing peaked around 3.6 GB RSS.
- `workspace` cold-path testing peaked around 4.1 GB RSS.
- Warm reruns were fast and low overhead.

## Validator expectations

- Treat `validation-contract.md` as the source of truth.
- For any assertion about “same plugin session”, “single startup path”, or “no duplicate panel instance”, collect host-log or fixture-counter evidence rather than relying on screenshots alone.
- For nested child-target zoom assertions, use the designated dev plugin fixture that labels the zoom target and sibling content explicitly.
- For pre-start action discovery assertions, collect evidence that discovery did not start the plugin process.
- For removed/updated plugin actions, rerun `./script/check-keymaps` after the change as part of evidence collection.

## Setup Notes

- No long-running services, databases, or credentials are required.
- Validation should use a dedicated dev plugin fixture created within this mission so the canvas, zoom-target, and action/session counters are observable.
- Prefer a single later manual validation session per milestone so workspace/plugin state remains coherent.
- Current in-session automation cannot attach to the native GPUI panel surface; workers should rely on targeted Rust validation plus fixture/log/counter preparation for later user validation.
