---
name: workspace-plugin-worker
description: Extend workspace-owned zoom behavior for plugin surfaces while preserving native pane and dock semantics.
---

# Workspace Plugin Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use for features that integrate plugin surfaces with workspace zoom, child-target zoom identity, dock movement/restore behavior, or native pane/dock zoom regressions.

## Required Skills

- `agent-browser` — Use for manual desktop verification of `Shift-Escape`, dock placement, restore flows, and child-target zoom behavior.

## Work Procedure

1. Read `.factory/library/architecture.md`, `.factory/library/plugin-runtime.md`, `.factory/library/user-testing.md`, and the zoom/cross-area assertions the feature fulfills.
2. Confirm the feature keeps zoom workspace-owned. If the implementation drifts toward a plugin-local fullscreen mode, stop and return to the orchestrator.
3. Write failing tests first in `workspace` and/or `plugin_host` for the exact zoom transitions being changed.
4. Implement the minimal host-visible identity and zoom plumbing needed for plugin surfaces or child targets.
5. Re-run native pane and dock zoom regressions in addition to new plugin zoom tests.
6. Use `agent-browser` with the dev plugin fixture to verify the exact user flow: focused panel zoom, child-target zoom, restore, dock move/reopen, and any session-coherence requirement tied to the feature.
7. Before finishing, run `cargo check --workspace --all-targets` and `./script/clippy`.

## Example Handoff

```json
{
  "salientSummary": "Integrated plugin surfaces with workspace zoom, added child-target zoom identity, and preserved native pane/dock zoom behavior. Manual Shift-Escape checks passed against the labeled fixture target.",
  "whatWasImplemented": "Added workspace/plugin-host plumbing so focused plugin panels and explicit nested plugin targets participate in the existing workspace zoom model, including restore and dock-move behavior, while keeping native pane and dock zoom semantics intact.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test -p workspace --lib --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Workspace zoom regressions and new plugin zoom tests pass."
      },
      {
        "command": "cargo test -p plugin_host --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Plugin host restore/dedup behavior remains stable with plugin zoom support."
      },
      {
        "command": "cargo check --workspace --all-targets",
        "exitCode": 0,
        "observation": "No type errors after zoom integration changes."
      },
      {
        "command": "./script/clippy",
        "exitCode": 0,
        "observation": "No new lint failures introduced by zoom changes."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Focused the fixture plugin panel, pressed Shift-Escape, then repeated with the nested labeled zoom target and a dock move.",
        "observed": "Zoom toggled only the intended surface/target, restore returned focus correctly, and the host log showed no duplicate panel instance."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/workspace/src/workspace.rs",
        "cases": [
          {
            "name": "plugin_zoom_clears_pane_zoom_and_restores_on_toggle",
            "verifies": "Pane zoom and plugin zoom stay mutually exclusive under workspace-owned zoom state."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature needs a broader change to workspace zoom semantics than the mission planned.
- Child-target zoom cannot be made host-addressable without a protocol change that belongs in an earlier pending feature.
- Native pane or dock zoom regressions appear unrelated but blocking and cannot be fixed inside the feature scope.
