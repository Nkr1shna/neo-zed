---
name: plugin-surface-worker
description: Extend the plugin mirror runtime, host rendering, and validation fixture for remote plugin surfaces.
---

# Plugin Surface Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use for features that change plugin metadata/protocol/runtime/host rendering for panels, titlebar widgets, canvas parity, remote element identity, fixture plugin surfaces, or plugin-surface lifecycle behavior.

## Required Skills

- `agent-browser` — Use for manual desktop verification against the dev plugin fixture whenever the feature changes a user-visible plugin surface or lifecycle.

## Work Procedure

1. Read `.factory/library/architecture.md`, `.factory/library/plugin-runtime.md`, `.factory/library/user-testing.md`, and the mission `validation-contract.md` assertions the feature fulfills.
2. Identify the exact protocol/runtime seam being changed before editing any code. If the feature would require bypassing `plugin_protocol` / `plugin_host`, return to the orchestrator.
3. Write failing tests first in the narrowest relevant surface (`gpui_plugin`, `ui_plugin`, `plugin_host`, or fixture tests). In the handoff, name the failing tests you added before implementation.
4. Implement the feature by extending existing plugin/runtime crates. Preserve existing mirrored widget behavior and existing panel/titlebar bindings.
5. If the feature affects manual validation, update or add the dev plugin fixture so validators can observe labeled canvas regions, nested zoom targets, and startup/session/action counters.
6. Run targeted validators relevant to the feature, then run `cargo check --workspace --all-targets` and `./script/clippy` before finishing.
7. If the feature is user-visible, use `agent-browser` to verify the fixture flow and capture evidence-worthy observations. For same-session or no-duplicate claims, include host-log or fixture-counter evidence.
8. Leave the tree clean of temporary logs or fixture hacks before returning.

## Example Handoff

```json
{
  "salientSummary": "Added mirrored canvas support to plugin surfaces, preserved existing panel/widget rendering, and extended the dev fixture with labeled canvas regions and session counters. Targeted plugin runtime and host tests pass, and the fixture renders correctly in the desktop app.",
  "whatWasImplemented": "Extended the plugin protocol/runtime/host path to support the new remote surface capability, added failing tests first in gpui_plugin and plugin_host, and updated the dev plugin fixture so canvas interactions and session identity are observable during manual validation.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test -p gpui_plugin --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Mirror runtime and interactive surface tests pass with the new surface payloads."
      },
      {
        "command": "cargo test -p plugin_host --tests -- --test-threads=6",
        "exitCode": 0,
        "observation": "Plugin host rerender, lifecycle, and action-routing regressions pass."
      },
      {
        "command": "cargo check --workspace --all-targets",
        "exitCode": 0,
        "observation": "Workspace compiles cleanly after protocol/runtime changes."
      },
      {
        "command": "./script/clippy",
        "exitCode": 0,
        "observation": "No new lint failures introduced by plugin-surface changes."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Opened the dev plugin panel and exercised the labeled canvas region with agent-browser.",
        "observed": "Canvas rendered in the panel chrome, input updated only the targeted surface, and fixture session counters showed no duplicate session."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/plugin_host/tests/plugin_host.rs",
        "cases": [
          {
            "name": "remote_canvas_rerender_keeps_targeted_surface_isolated",
            "verifies": "Two plugin surfaces do not cross-talk when only one receives input and rerenders."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature appears to require a new service, background process, or port.
- The only viable design would bypass the plugin host/protocol boundary instead of extending it.
- Required fixture observability (startup/session/action counters or labeled zoom target) cannot be added without changing mission scope.
- Backward compatibility for existing plugin panels/titlebar widgets would be broken without a broader migration plan.
