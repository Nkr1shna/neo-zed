---
name: agent-panel-worker
description: Implement detached agent-panel behavior in agent_ui and workspace while preserving one workspace-owned agent experience.
---

# Agent Panel Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that change:

- `crates/agent_ui` detached-window behavior
- `workspace` routing, dock suppression, and restore behavior for the agent panel
- per-workspace detach lifecycle, focus routing, and cleanup
- detached-window shell UI and agent-specific controls that are not GPUI backend work

## Required Skills

None.

## Work Procedure

1. Read `.factory/library/architecture.md`, `.factory/library/windowing.md`, `.factory/library/user-testing.md`, and mission `AGENTS.md` before editing.
2. Identify the smallest set of owning files in `agent_ui` and `workspace` before changing code. Do not create a second independent agent state model.
3. Add or update focused tests first:
   - `agent_ui` tests for detach/restore state continuity, one-window-per-workspace behavior, and agent-surface routing
   - `workspace` tests for dock suppression, focus/reveal routing, and cleanup when windows/workspaces close
4. Implement the feature using the existing workspace-owned `AgentPanel` as the source of truth. Treat detach as a presentation switch between docked and detached shells.
5. For dock behavior, ensure the docked presentation is fully suppressed while detached and restored on close. Do not leave duplicate surfaces or empty dock shells.
6. Verify behavior with the narrowest practical commands first:
   - `cargo test -p agent_ui --lib --tests -- --test-threads=6`
   - `cargo test -p workspace --lib --tests -- --test-threads=6`
   - `cargo fmt --all -- --check`
   - `./script/clippy`
   - `cargo check --workspace --all-targets`
7. Perform manual desktop validation for user-visible native-window behavior relevant to the feature, and record the exact actions and observations.
8. In the handoff, explicitly call out any lifecycle edge cases that were not fully validated manually.

## Example Handoff

```json
{
  "salientSummary": "Implemented the detached agent-panel shell and workspace routing so each workspace can pop out one floating agent window without duplicating agent state. Added targeted `agent_ui` and `workspace` coverage, then manually verified detach, restore, and focus routing.",
  "whatWasImplemented": "Added a workspace-owned detached-agent controller and a floating detached shell that reuses the existing AgentPanel state. While detached, the docked presentation is fully suppressed; closing the detached window restores the docked panel, and repeated focus/reveal entry points reuse the same detached window for that workspace.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test -p agent_ui --lib --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Agent UI tests passed, including new detach/restore and one-window-per-workspace cases."
      },
      {
        "command": "cargo test -p workspace --lib --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Workspace tests passed, covering dock suppression and focus/reveal routing."
      },
      {
        "command": "cargo fmt --all -- --check",
        "exitCode": 0,
        "observation": "Formatting remained clean after the Rust edits."
      },
      {
        "command": "./script/clippy",
        "exitCode": 0,
        "observation": "Repo-standard linting passed."
      },
      {
        "command": "cargo check --workspace --all-targets",
        "exitCode": 0,
        "observation": "Workspace-wide type checking completed successfully."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Opened the docked agent panel, clicked pop out, then closed the detached window",
        "observed": "The docked panel disappeared while detached, the floating window reused the same thread state, and closing it restored the docked panel without creating a duplicate surface."
      },
      {
        "action": "Used agent focus/reveal commands while detached",
        "observed": "Focus routed to the existing detached window for the owning workspace instead of reopening a docked duplicate."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/agent_ui/src/agent_panel.rs",
        "cases": [
          {
            "name": "detached_panel_reuses_existing_thread_state",
            "verifies": "Detaching does not create a second independent agent conversation surface."
          }
        ]
      },
      {
        "file": "crates/workspace/src/workspace.rs",
        "cases": [
          {
            "name": "agent_panel_focus_routes_to_detached_window",
            "verifies": "Workspace focus/reveal entry points target the detached window while detached."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature requires GPUI/window backend APIs that do not exist yet
- A change would require duplicating or migrating agent conversation ownership out of the workspace-owned panel model
- Focus/reveal behavior is ambiguous for a user-visible workflow not covered by the mission contract
