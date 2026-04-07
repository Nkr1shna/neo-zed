# User Testing

Validation surface findings and execution guidance for the detached agent panel PiP mission.

## Validation Surface

This mission is a native desktop UI feature. The primary validation surfaces are:

1. **Manual desktop validation**
   - Tools: native app UI, screenshots, window counts, visible focus/stacking observations
   - Targets:
     - docked agent panel pop-out
     - detached floating window behavior
     - close-to-restore flow
     - focus/reveal routing
     - per-workspace detached ownership
     - always-on-top behavior on supported platforms
     - Wayland degradation behavior

2. **Focused Rust/GPUI automated validation**
   - Tools: `cargo test`, `cargo fmt`, `./script/clippy`, `cargo check`
   - Targets:
     - `agent_ui` tests for agent-panel state and routing
     - `workspace` tests for dock/workspace lifecycle behavior
     - GPUI/window abstraction tests for always-on-top API

3. **Visual-runner sanity coverage**
   - Tools: `cargo build -p zed --bin zed_visual_test_runner --features visual-tests`
   - Targets:
     - detached-window-adjacent presentation states when deterministic screenshot coverage is practical
   - Note: visual tests complement, but do not replace, real desktop manual validation for native window manager behavior.

## Validation Concurrency

### Surface: manual desktop validation
- Max concurrent validators: **1**
- Rationale: detached windows and always-on-top behavior interact with global window-manager state, so parallel validators would interfere with each other and make evidence unreliable.

### Surface: focused Rust/GPUI automated validation
- Max concurrent validators: **2**
- Rationale: the machine has 12 CPU cores and ample memory, but this repo’s Rust builds share a heavy target directory. A cap of 2 keeps validation useful without causing too much contention.

## Dry-run findings

- `cargo test -p workspace ... --no-run` succeeded during planning on a relevant GPUI/window-backed target.
- `cargo fmt --all -- --check` and `./script/clippy` are the repo-standard validation entrypoints.
- `cargo nextest` is not installed locally; validators should prefer `cargo test`.
- The native validation path is executable locally without extra services or credentials.
- Baseline repo validators currently have unrelated pre-existing failures in `gpui_plugin`, `plugin_host`, `plugin_protocol`, and `plugins/codex-usage-plugin`; validators should record them as pre-existing unless a mission feature directly touches those areas.

## Validator expectations

- Treat the contract as the source of truth for user-visible behavior.
- Prefer real desktop interaction for assertions about:
  - floating window existence
  - dock suppression
  - focus routing
  - close/restore lifecycle
  - always-on-top stacking
- Use automated tests to support behavior and regression coverage, not as the sole proof of native window-manager behavior.
- For platform-conditioned assertions:
  - macOS / Windows / Linux-X11: lock control must work
  - Linux-Wayland: detach must work, lock must be visibly disabled without a false success signal

## Setup Notes

- No long-running services, seeded databases, or browser automation are required.
- Validation should run against the checked-out repo state plus writes under `.factory/validation/` and mission evidence paths only.
