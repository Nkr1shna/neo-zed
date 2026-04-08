---
name: action-system-worker
description: Add plugin action metadata, proxy dispatch, command-palette discovery, and keybinding integration.
---

# Action System Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use for features that add plugin action metadata, runtime handler registration, host proxy actions, command-palette integration, keymap parsing/validation, keybinding editor behavior, lazy startup on invocation, or session-coherence across action entry paths.

## Required Skills

- `agent-browser` — Use for manual desktop verification of action discovery, invocation, startup failure handling, and keybinding behavior.

## Work Procedure

1. Read `.factory/library/architecture.md`, `.factory/library/plugin-runtime.md`, `.factory/library/user-testing.md`, and the action/cross-area assertions the feature fulfills.
2. Confirm the design keeps command-palette and keymap integration compatible with GPUI’s static action infrastructure. If the feature needs per-plugin dynamic Rust action types, return to the orchestrator.
3. Write failing tests first in the narrowest relevant targets (`plugin_host`, `command_palette`, `keymap_editor`, `settings`, or manifest parsing) before implementing behavior.
4. Implement metadata discovery, host proxy dispatch, and runtime handler wiring in incremental steps so discovery-before-startup and invoke-after-startup remain separable.
5. For any feature affecting user keymaps, run `./script/check-keymaps` and verify add/edit/unbind/delete flows rather than only the happy path.
6. Use `agent-browser` with the dev plugin fixture to verify discovery before startup, lazy-start invocation, repeated invocation behavior, startup failure handling, and keybinding dispatch.
7. Collect host-log or fixture-counter evidence for assertions about same session, single startup path, or exactly-once action execution.
8. Before finishing, run `cargo check --workspace --all-targets` and `./script/clippy`.

## Example Handoff

```json
{
  "salientSummary": "Added manifest-visible plugin actions with host proxy dispatch and keybinding integration, and verified cold-start invocation through both command palette and keyboard. Command and keymap flows remain compatible with existing native behavior.",
  "whatWasImplemented": "Extended plugin metadata and host dispatch so plugin actions are discoverable before startup, lazily start the plugin on invocation, register runtime handlers, and participate in command-palette and keymap workflows without breaking native actions or keymap validation.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test -p command_palette --lib --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Command palette tests pass with plugin action entries and native action regressions intact."
      },
      {
        "command": "cargo test -p plugin_host --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Plugin host dispatch and lazy-start tests pass, including startup failure and duplicate invocation handling."
      },
      {
        "command": "cargo test -p keymap_editor --lib --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Keybinding editor tests pass for plugin action search and edit flows."
      },
      {
        "command": "./script/check-keymaps",
        "exitCode": 0,
        "observation": "Keymap validation still succeeds with plugin action bindings and after plugin action removal/update scenarios."
      },
      {
        "command": "cargo check --workspace --all-targets",
        "exitCode": 0,
        "observation": "Workspace compiles cleanly after action/keymap changes."
      },
      {
        "command": "./script/clippy",
        "exitCode": 0,
        "observation": "No new lint failures introduced by action-system changes."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Discovered the fixture plugin action before startup, invoked it from the command palette, then bound and invoked it via a custom keybinding.",
        "observed": "Discovery did not start the plugin, the first invocation started it exactly once, repeated invocation stayed deterministic, and the keybinding reused the same live plugin session."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/command_palette/src/command_palette.rs",
        "cases": [
          {
            "name": "plugin_actions_are_discoverable_before_plugin_startup",
            "verifies": "Manifest-declared plugin actions appear in command search before the plugin process starts."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The design requires dynamically mutating the GPUI action registry per plugin action type.
- Keymap validation/editor surfaces cannot be kept compatible with existing native action behavior without a broader architecture change.
- A feature needs external services or global environment changes not captured in mission boundaries.
