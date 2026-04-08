# Environment

Environment variables, external dependencies, and setup notes for the plugin platform mission.

## External dependencies

- No new credentials, accounts, databases, or external APIs are required.
- No long-running local services or ports are needed for implementation or validation.

## Local tooling findings

- `cargo`, `cargo fmt`, `python3`, `rg`, and repo scripts are available locally.
- `cargo nextest` is not installed locally; workers should use `cargo test` for local verification.
- Repo-standard linting is `./script/clippy`.
- Repo-standard keymap validation is `./script/check-keymaps`.

## Local repo constraints

- Preserve unrelated local edits in `assets/settings/default.json` and `crates/agent_ui/src/agent_panel.rs` unless a mission feature explicitly requires overlap.
- Do not introduce new local services, background daemons, or port allocations.

## Machine/resource notes

- Planning dry run observed a 12-core machine with roughly 51.5 GB RAM.
- This Rust workspace is large and shares one target directory; use conservative test parallelism.
- Baseline `cargo test` commands in `.factory/services.yaml` should stay capped at 6 test threads.

## What does not belong here

- Service start/stop commands or ports (use `.factory/services.yaml`)
- Behavioral invariants or crate ownership (use `architecture.md` or topic files)
