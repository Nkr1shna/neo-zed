# Environment

Environment and setup notes for the detached agent panel PiP mission.

## External dependencies

- No new credentials, accounts, databases, or external APIs are required for this mission.
- No long-running local services need to be started for implementation or validation.

## Local tooling findings

- `cargo`, `cargo fmt`, `python3`, and `rg` are available locally.
- `cargo nextest` is not installed locally; workers should use `cargo test` for local verification unless a feature explicitly installs or requires something else.
- The repo-standard lint command is `./script/clippy`.

## Machine/resource notes

- The planning dry run observed a 12-core machine with ample memory.
- Repo-wide Rust test commands should still use conservative test parallelism because this workspace is large and shares one target directory.
- `.factory/services.yaml` therefore caps test threads at 6 for the baseline `cargo test` commands.

## Platform scope

- In scope:
  - macOS
  - Windows
  - Linux/X11
  - Linux/Wayland
- Expected degradation:
  - Wayland supports detach/restore
  - Wayland does not promise always-on-top; the lock control must be disabled or unavailable there

## What does not belong here

- Service ports or start/stop commands (use `.factory/services.yaml`)
- Architectural behavior or invariants (use `architecture.md`)
