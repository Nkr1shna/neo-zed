---
name: gpui-window-worker
description: Implement GPUI window abstraction and backend changes needed for detached-agent always-on-top behavior.
---

# GPUI Window Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that change:

- `crates/gpui` window abstractions
- macOS, Windows, Linux/X11, or Wayland backend window behavior
- generic runtime always-on-top or related platform-window APIs
- platform-conditioned window capability plumbing used by the detached agent panel

## Required Skills

None.

## Work Procedure

1. Read `.factory/library/architecture.md`, `.factory/library/windowing.md`, `.factory/library/user-testing.md`, and mission `AGENTS.md`.
2. Identify the generic GPUI API shape before editing any backend code. Prefer additive reusable APIs over agent-specific special cases.
3. Add focused tests first where practical:
   - GPUI/window abstraction tests for API state and behavior
   - backend-targeted tests if an existing harness supports them
   - if no direct runtime test exists, add the narrowest deterministic coverage and document remaining manual validation needs
4. Implement the shared GPUI/window API changes first, then wire each backend:
   - macOS
   - Windows
   - Linux/X11
   - Wayland graceful degradation
5. Preserve unrelated floating-window behavior. Do not regress existing windows that already use `WindowKind::Floating`, `PopUp`, or `Dialog`.
6. Verify the changes with the narrowest practical commands first, then repo validators:
   - focused `cargo test -p gpui ...` or package-specific tests if available
   - `cargo fmt --all -- --check`
   - `./script/clippy`
   - `cargo check --workspace --all-targets`
7. Record platform-conditioned behavior explicitly in the handoff, especially anything unsupported on Wayland.

## Example Handoff

```json
{
  "salientSummary": "Added a runtime always-on-top API to GPUI windows and wired macOS, Windows, and Linux/X11 backend support with explicit Wayland degradation. Verified the API with focused tests and workspace-wide validation.",
  "whatWasImplemented": "Introduced a generic runtime always-on-top capability on the GPUI window abstraction, implemented backend support for macOS, Windows, and Linux/X11, and left Wayland in a clearly unsupported state so higher-level UI can disable the lock control without implying success.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test -p gpui -- --test-threads=6",
        "exitCode": 0,
        "observation": "Focused GPUI tests passed for the new window capability surface."
      },
      {
        "command": "cargo fmt --all -- --check",
        "exitCode": 0,
        "observation": "Formatting remained clean after API and backend edits."
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
        "action": "Reviewed supported-platform detached window stacking after enabling and disabling always-on-top",
        "observed": "Supported platforms changed stacking behavior with the runtime API; Wayland remained explicitly unsupported without a false success state."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/gpui/src/window.rs",
        "cases": [
          {
            "name": "always_on_top_state_round_trips_through_window_api",
            "verifies": "The shared window abstraction exposes and preserves the requested always-on-top state."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- Backend support differs from the mission contract in a way that needs product judgment
- Wayland or another platform cannot support the requested capability and the graceful degradation path is ambiguous
- A proposed API shape would break unrelated windows or require broad refactoring outside the approved mission
