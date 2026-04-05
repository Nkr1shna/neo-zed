---
name: packaging-worker
description: Implement OS packaging, installer, manifest, and release-workflow identity changes for the Neo Zed fork.
---

# Packaging Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the work procedure.

## When to Use This Skill

Use this skill for features that change:

- macOS bundle metadata, DMG naming, install/uninstall resources
- Windows installer/AppX/AppUserModelID/winget/release metadata
- Linux desktop, Flatpak, Snap, install/uninstall, bundle, and artifact metadata
- release workflows and artifact naming

## Required Skills

None.

## Work Procedure

1. Read `.factory/library/release-identity.md`, `.factory/library/architecture.md`, `.factory/library/user-testing.md`, and mission `AGENTS.md`.
2. Determine the owning packaging files for the target platform before editing; avoid one-off replacements when a template or shared script is the real source of truth.
3. Add or update narrow validation coverage first where practical:
   - Rust/resource tests if the packaging surface is generated from Rust metadata
   - lightweight script or metadata assertions where an existing test harness exists
   - otherwise prepare deterministic inspection commands before editing so you can prove the output changed as intended
4. Implement the platform-specific packaging edits, preserving the agreed channel mapping and public identity tuple from `.factory/library/release-identity.md`.
5. Run the narrowest practical validation for the changed platform:
   - `cargo fmt --all -- --check` and targeted `./script/clippy -p <package>` if Rust files changed
   - metadata inspection (`plutil`, `python3`, `rg`) for bundle manifests, AppX manifests, desktop entries, snap/flatpak metadata, and release workflows
   - targeted build or packaging commands only when they are feasible on the current host and do not require unavailable credentials
6. Compare all channel variants for the touched platform and record any mismatches found/fixed.
7. In the handoff, explicitly note any platform validation that could not be executed locally because of host OS or missing signing/publishing credentials.

## Example Handoff

```json
{
  "salientSummary": "Reworked the Linux and workflow packaging identity to Neo Zed. The desktop templates, bundle scripts, Snap/Flatpak metadata, and release workflow artifact names now use neozed/dev.neozed values instead of the upstream Zed identity.",
  "whatWasImplemented": "Updated the Linux desktop integration templates and bundle/install scripts to emit neozed commands and dev.neozed-based desktop IDs, then aligned release workflow artifact names and package metadata with the Neo Zed naming scheme.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "python3 - <<'PY'\nfrom pathlib import Path\nprint(Path('crates/zed/resources/zed.desktop.in').read_text())\nPY",
        "exitCode": 0,
        "observation": "Desktop template inspection showed the new command, scheme handler, and app naming surfaces."
      },
      {
        "command": "rg -n \"Neo Zed|neozed|dev\\.neozed|neozed\\.dev\" crates/zed/resources script .github/workflows",
        "exitCode": 0,
        "observation": "Touched packaging/workflow files now expose the Neo Zed identity tuple."
      },
      {
        "command": "cargo fmt --all -- --check",
        "exitCode": 0,
        "observation": "Formatting remained clean after packaging-related Rust or metadata generator edits."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Inspected per-channel packaging metadata for stable, preview, nightly, and dev",
        "observed": "Observed consistent channel naming and dev.neozed-derived IDs without channel cross-contamination in the touched platform files."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/release_channel/tests/package_identity.rs",
        "cases": [
          {
            "name": "channel_app_ids_match_packaging_identity_contract",
            "verifies": "Packaging-facing app IDs remain aligned with the shared Neo Zed channel map."
          }
        ]
      }
    ]
  },
  "discoveredIssues": [
    {
      "severity": "medium",
      "description": "Publishing credentials for signed release artifacts are still external prerequisites and must be tracked in the release checklist."
    }
  ]
}
```

## When to Return to Orchestrator

- The platform-specific identity mapping is ambiguous or contradicts `.factory/library/release-identity.md`
- A packaging surface cannot be safely updated without deciding whether to preserve an upstream compatibility identifier
- Validation would require unavailable platform tooling, signing credentials, or publication credentials and no reliable inspection fallback exists
