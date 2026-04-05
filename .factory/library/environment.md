# Environment

Environment variables, external dependencies, and release-identity setup notes for the Neo Zed fork mission.

## Canonical public identity

- Product name: `Neo Zed`
- CLI command: `neozed`
- URL scheme: `neozed://`
- Base bundle/app ID: `dev.neozed`
- Domain: `https://neozed.dev`
- API domain: `https://api.neozed.dev`
- Cloud/update domain: `https://cloud.neozed.dev`
- Repo URL: `https://github.com/Nkr1shna/neo-zed/`

## Email convention

Preserve the existing mailbox local parts on the new domain where release-facing addresses are needed:

- `hi@neozed.dev`
- `legal@neozed.dev`
- `privacy@neozed.dev`
- `billing-support@neozed.dev`
- `arbitration-opt-out@neozed.dev`

## Release credentials not required for this mission

Workers should not block implementation on missing distribution credentials unless a feature explicitly requires a live signed build:

- macOS signing / notarization certificates
- Windows signing credentials
- Sentry / crash-reporting credentials
- registry publication credentials (winget / stores)

These missing credentials must instead be captured in the release checklist document if still required for first release.

## Tooling findings

- `cargo`, `cargo fmt`, `python3`, `plutil`, and `rg` are available locally.
- `cargo nextest` is not installed locally; do not assume it for baseline validation.
- The repo-standard lint command is `./script/clippy`.

## What does not belong here

- Service ports or start/stop commands (use `.factory/services.yaml`)
- Architecture/source-of-truth explanations (use `architecture.md`)
