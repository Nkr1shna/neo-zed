# User Testing

Validation surface findings and execution guidance for the Neo Zed fork identity mission.

## Validation Surface

This mission does not depend on a browser or long-running application flow. The primary validation surfaces are:

1. **File and metadata inspection**
   - Tools: `rg`, `python3`, `plutil`
   - Targets: bundle metadata, installer metadata, manifests, scripts, docs, legal text, workflow files

2. **Targeted Rust/build validation**
   - Tools: `cargo`, `cargo fmt`, `./script/clippy`
   - Targets: shared release identity sources (`release_channel`, `paths`, URL builders) and any touched Rust code

3. **Rendered packaging sanity checks**
   - Tools: `python3`, `plutil`, shell commands
   - Targets: desktop-entry templates, bundle metadata blocks, workflow artifact names, package IDs

## Validation Concurrency

### Surface: file/metadata inspection
- Max concurrent validators: **2**
- Rationale: inspection itself is cheap, but many checks will be paired with workspace-wide `rg`, Python extraction, or targeted cargo verification. Keeping concurrency at 2 avoids noisy overlap and conflicting conclusions while still providing throughput.

### Surface: targeted Rust/build validation
- Max concurrent validators: **2**
- Rationale: the machine has 12 CPU cores and ~48 GB memory, but Rust lint/build work in this repo can be heavy and competes for the same target directory. Using 70% of practical headroom still favors a conservative cap of 2 concurrent validator sessions for this metadata-heavy mission.

## Dry-run findings

- `cargo fmt --all -- --check` succeeds locally.
- `./script/clippy` is the repo-standard lint entrypoint.
- `cargo nextest` is not installed locally, so validators should not depend on it unless they install it explicitly.
- `plutil`, `python3`, and `rg` are available for metadata inspection.
- Task-based validator droids can currently fail to launch with `Invalid model: custom:droidproxy:gpt-5.4`; when that happens, fall back to manual flow reports in the current validator session and record the workaround in synthesis.

## Validator expectations

- Use scoped absence checks instead of trying to eliminate every internal `Zed` string in the repository.
- Treat these as release-facing scopes by default:
  - packaging metadata
  - install/uninstall scripts
  - release workflows/artifact names
  - user/admin/release docs
  - public URLs, emails, and repo links
- Treat these as potentially intentional/internal unless the feature says otherwise:
  - Rust component/type/module names
  - compatibility-sensitive internal protocol names
  - historical comments or non-release development references outside the touched scope

## Setup Notes

- The `core-identity` milestone does not require long-running local services, seeded data, or browser automation.
- Validation should run against the checked-out repository state with read-only inspection commands plus report/evidence writes under `.factory/validation/` and the mission `evidence/` directory.

## Flow Validator Guidance: file/metadata inspection

- Isolation boundary: treat the repository checkout as shared read-only state. Do not edit product code or docs while validating; only write the assigned flow report and evidence files.
- Stay within the assertion-specific scopes provided by the validator. Use scoped absence checks in touched release-facing files instead of broad repo-wide claims about every `Zed` string.
- Capture exact evidence: file paths, extracted values, and the commands used to confirm presence/absence of `Neo Zed`, `neozed`, `neozed://`, `dev.neozed*`, and install-path identities.
- If an assertion depends on intentionally preserved internal names, mark the assertion `pass` only when the remaining legacy values are clearly outside the assigned release-facing scope; otherwise mark it `fail` and explain the conflicting file paths.
