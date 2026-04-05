# Architecture

High-level map of the release-identity surfaces for the Neo Zed fork mission.

## What belongs here

- Sources of truth for public product identity
- How identity flows into packaging, scripts, release workflows, docs, and networked surfaces
- Invariants workers must preserve while rebranding

## Identity sources of truth

### 1. Shared application identity
- `crates/release_channel/src/lib.rs` is the primary source of truth for:
  - channel display names
  - public app IDs / bundle IDs
  - Windows app identifiers
  - release-channel derived identity
- `crates/paths/src/paths.rs` is the primary source of truth for:
  - config/data/cache/log/support paths
  - local storage naming visible to users and uninstall flows

### 2. Public command and scheme identity
- `crates/cli`, `crates/install_cli`, and related scripts define the public CLI command surface.
- `crates/zed/Cargo.toml`, desktop templates, and docs define the public deep-link scheme and bundle registration.

### 3. Desktop packaging surfaces
- macOS:
  - `crates/zed/Cargo.toml`
  - `crates/zed/resources/info/*`
  - `script/bundle-mac`
  - `script/install.sh`
  - `script/uninstall.sh`
- Windows:
  - `crates/zed/resources/windows/zed.iss`
  - `crates/explorer_command_injector/AppxManifest*.xml`
  - `script/bundle-windows.ps1`
  - release workflows / winget publication
- Linux:
  - `crates/zed/resources/zed.desktop.in`
  - `crates/zed/resources/flatpak/zed.metainfo.xml.in`
  - `crates/zed/resources/snap/snapcraft.yaml.in`
  - `script/bundle-linux`
  - `script/install.sh`
  - `script/uninstall.sh`

### 4. Networked/public surfaces
- `assets/settings/default.json` establishes the default site/server URL.
- `crates/http_client` maps the primary domain to API/cloud endpoints.
- `crates/client`, `crates/auto_update`, `crates/feedback`, telemetry/crash scripts, and docs expose public URLs, repo links, support links, and email surfaces.

### 5. Release and documentation surfaces
- `.github/workflows/*` publish artifact names and registry/package metadata.
- `docs/`, `legal/`, and release-oriented scripts/docs expose the public brand and support surfaces.

## Required invariants

1. The public fork identity tuple must stay consistent:
   - product name: `Neo Zed`
   - CLI: `neozed`
   - scheme: `neozed://`
   - base bundle/app ID: `dev.neozed`
   - site: `https://neozed.dev`
   - API: `https://api.neozed.dev`
   - cloud/update host: `https://cloud.neozed.dev`
   - repo URL: `https://github.com/Nkr1shna/neo-zed/`

2. Channel identities derive from the same base:
   - stable: `Neo Zed` / `dev.neozed`
   - preview: `Neo Zed Preview` / `dev.neozed.Preview`
   - nightly: `Neo Zed Nightly` / `dev.neozed.Nightly`
   - dev: `Neo Zed Dev` / `dev.neozed.Dev`

3. Preserve upstream pullability:
   - do not rename Rust component/type/module names unless the old name leaks directly into release identity
   - leave intentionally compatibility-sensitive internal protocol names alone unless they are part of a public release-facing registration surface

4. Prefer updating existing source-of-truth files over scattered one-off replacements.

5. Packaging/workflow identities and docs must agree; do not leave mismatched names, IDs, commands, or URLs between surfaces.
