# Neo Zed First Release Checklist

Verified against the fork repository state on April 5, 2026. Upstream release-flow notes were cross-checked on April 3, 2026.

This document is the launch gate for the first public Neo Zed release. It combines the current in-repo consistency sweep with the remaining external work that still has to exist outside this repository before a public launch is safe.

## Repo Consistency Sweep Completed

The current repository state is aligned on the public Neo Zed identity tuple across the audited release-facing surfaces in code, packaging, workflows, scripts, and docs:

- product name: `Neo Zed`
- CLI command: `neozed`
- URL scheme: `neozed://`
- base app ID: `dev.neozed`
- public site: `https://neozed.dev`
- cloud/update host: `https://cloud.neozed.dev`
- fork repo URL: `https://github.com/Nkr1shna/neo-zed/`

The consistency sweep covered these source-of-truth or release-entry surfaces:

- `crates/release_channel/src/lib.rs`
- `crates/zed/Cargo.toml`
- `script/install.sh`, `script/uninstall.sh`, `script/bundle-linux`, `script/bundle-windows.ps1`
- `.github/workflows/release.yml`, `.github/workflows/release_nightly.yml`, `.github/workflows/run_bundling.yml`
- release-facing help links and support URLs in app code
- release-facing docs and install/reference pages touched by this mission

## Intentional Compatibility Holdovers

Some upstream-branded identifiers are still intentionally preserved because changing them in this pass would break compatibility or upgrade behavior:

- legacy internal `zed://` parsing remains accepted so older deep links can still open
- remote server artifacts and install paths still use `zed-remote-server*` and `~/.zed_server`
- Windows upgrade cleanup still references legacy `ZedIndustries.Zed*` AppX package names so users can upgrade from earlier builds
- some internal staging or provider identifiers still use upstream-style names where no fork-owned replacement has been defined yet
- historical upstream GitHub issue/PR/discussion links remain where they are part of changelog, troubleshooting, or development context rather than the fork's public release identity

## First-Release Checklist

### 1. DNS, Site, and Public Web Content

- [ ] Point `neozed.dev` at the production website
- [ ] Point `api.neozed.dev` at the production API/control-plane service
- [ ] Point `cloud.neozed.dev` at the production release/update host
- [ ] Serve all public pages the app and docs now reference: download, releases/stable, releases/preview, blog, roadmap, pricing, account, FAQ/community links, CLA, terms, privacy, cookie policy, and docs
- [ ] Publish `https://neozed.dev/oauth/client-metadata.json`
- [ ] Publish extension-listing, theme-builder, and schema endpoints that now use the Neo Zed domain
- [ ] Mirror or replace any externally hosted docs/site images before launch if broken image links are unacceptable
- [ ] Decide whether fork-owned staging hostnames are needed; if so, define them explicitly and update code/config that still assumes upstream staging names

### 2. Auth, API, Cloud, and Update Hosting

- [ ] Deploy the services behind `api.neozed.dev`
- [ ] Deploy release/update downloads behind `cloud.neozed.dev`, including the endpoints used by install scripts and auto-update
- [ ] Confirm OAuth allowlists and callback policies accept the Neo Zed domains and metadata URL
- [ ] Confirm external providers that inspect referers or product URLs accept `https://neozed.dev`
- [ ] Dry-run account, billing, upgrade, shared-link, and release-download flows against fork-owned infrastructure

### 3. Email and Support Operations

- [ ] Create and verify all public mailboxes referenced by the repo, including `hi@neozed.dev`, `billing-support@neozed.dev`, `legal@neozed.dev`, `sales@neozed.dev`, and `arbitration-opt-out@neozed.dev`
- [ ] Route support, billing, and legal mailboxes to monitored operational inboxes
- [ ] Confirm public support/community destinations are fork-owned and staffed

### 4. Signing, Notarization, and Trusted Distribution

- [ ] Provision Apple Developer certificates, notarization credentials, provisioning profiles, and DMG signing assets
- [ ] Provision Windows code-signing credentials and any publisher identity needed for AppX/MSIX or installer trust
- [ ] Decide which Linux channels are officially supported for the first launch and provision the necessary signing keys or store credentials
- [ ] Revisit packaging-script compatibility notes that currently assume temporary or placeholder fork credentials

### 5. Release Automation and Secrets

- [ ] Add the GitHub Actions secrets, GitHub App credentials, storage credentials, and release tokens required by `release.yml`, `release_nightly.yml`, and `after_release.yml`
- [ ] Configure protected tags, environments, and permissions in the fork repository to match the workflow assumptions
- [ ] Dry-run the stable/preview release workflow, nightly workflow, and after-release workflow in fork-owned infrastructure before the first public tag
- [ ] Decide whether preview patch releases should keep the inherited auto-publish behavior exactly as upstream does

### 6. Package Registries and Distribution Channels

- [ ] Publish or reserve the Homebrew package names for `neozed` and `neozed@preview`
- [ ] Publish WinGet manifests for the fork-owned identifiers and verify upgrade behavior from earlier prerelease builds
- [ ] Publish any Linux package-manager channels that are in first-release scope (for example Snap, Flatpak, or distro-specific feeds)
- [ ] Verify GitHub Releases expose the expected Neo Zed desktop artifact names and that install scripts can download them end-to-end

### 7. Crash Reporting, Telemetry, and Observability

- [ ] Provision Neo Zed Sentry org/project(s), DSNs, auth tokens, and symbol-upload credentials
- [ ] Verify crash reports, telemetry, and hosted-service dashboards identify the product as Neo Zed where users or release operators can see them
- [ ] Confirm the fork-owned privacy/telemetry controls match the published legal/support text
- [ ] Review any remaining upstream-branded internal org/project names and decide whether to rename them or document them as internal-only

### 8. Launch Rehearsals

- [ ] Produce at least one signed dry-run stable release and one dry-run preview or nightly release from fork-owned infrastructure
- [ ] Install, update, and uninstall each supported channel on macOS, Windows, and Linux from clean machines
- [ ] Verify `neozed`, `neozed://`, docs links, account links, and release downloads all work from outside the development environment
- [ ] Confirm legal/support pages and public mailboxes are live before any public announcement

## Channel Consistency Snapshot

| Surface                      | Stable                                                                         | Preview                                                        | Nightly                                                        | Dev                                                    |
| ---------------------------- | ------------------------------------------------------------------------------ | -------------------------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------ |
| Shared display name / app ID | `Neo Zed` / `dev.neozed`                                                       | `Neo Zed Preview` / `dev.neozed.Preview`                       | `Neo Zed Nightly` / `dev.neozed.Nightly`                       | `Neo Zed Dev` / `dev.neozed.Dev`                       |
| Packaging metadata           | `crates/zed/Cargo.toml` matches shared identity                                | same                                                           | same                                                           | same                                                   |
| Install / uninstall scripts  | stable bundle + stable app ID                                                  | preview suffix + preview app ID                                | nightly suffix + nightly app ID                                | dev suffix + dev app ID                                |
| Desktop packaging scripts    | shared `neozed` artifact filenames with stable metadata inside the stable lane | shared filenames with preview metadata inside the preview lane | shared filenames with nightly metadata inside the nightly lane | shared filenames with dev metadata inside the dev lane |
| Release workflows            | `neozed-*` desktop assets uploaded to the stable tag/release                   | preview tag/release chooses preview channel                    | nightly workflow forces nightly channel                        | dev remains the repo-default development lane          |

## Upstream Zed Release-Flow Reference

Neo Zed still inherits the upstream branch/tag release model:

- `main` is the next line of development
- the newest `v0.NNN.x` branch is the preview branch
- the previous `v0.NNN.x` branch is the stable branch
- stable tags use `vX.Y.Z`, preview tags use `vX.Y.Z-pre`, and nightly is driven by the moving `nightly` tag or schedule
- `release.yml` creates a draft GitHub release, uploads assets, validates them, and only auto-publishes preview patch releases that are not `.0-pre`
- `after_release.yml` runs after publication to refresh downstream release surfaces

Treat that upstream flow as the operational baseline, but do not ship Neo Zed publicly until every external prerequisite above exists on fork-owned infrastructure.
