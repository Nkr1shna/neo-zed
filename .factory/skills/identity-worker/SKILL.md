---
name: identity-worker
description: Implement shared product-identity, domain, path, and documentation rebrand changes for the Neo Zed fork.
---

# Identity Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the work procedure.

## When to Use This Skill

Use this skill for features that change:

- shared release identity sources
- CLI command or URL scheme surfaces
- local app-data/config/cache/log path identity
- public domain/repo/email mappings
- user-facing docs, legal/support docs, or the fork-release checklist
- final consistency sweeps across release-facing surfaces

## Required Skills

None.

## Work Procedure

1. Read `.factory/library/release-identity.md`, `.factory/library/architecture.md`, and mission `AGENTS.md` before editing anything.
2. Identify the smallest set of release-facing files that actually own the surface you are changing. Prefer editing shared sources of truth before patching downstream copies.
3. Add or update characterization coverage first when the surface is backed by Rust logic:
   - examples: `release_channel`, `paths`, URL builders, feedback/support URL helpers
   - if no existing automated test location fits, add a narrowly scoped assertion test before implementation
4. Make the implementation changes, keeping internal Rust component/type/module names unchanged unless the old name leaks into the public release identity.
5. Run targeted verification for the changed scope:
   - relevant `cargo test ...` for any new or updated assertions
   - `cargo fmt --all -- --check` if Rust files changed
   - `./script/clippy -p <package>` or the narrowest practical lint scope when Rust files changed
   - `rg` checks proving the new release-facing identity is present and the replaced upstream identity is absent in the touched scope
6. Perform manual inspection of the touched metadata/docs and record the exact identity tuple you verified (`Neo Zed`, `neozed`, `neozed://`, `dev.neozed*`, `neozed.dev`, fork repo URL).
7. In the handoff, explicitly call out any upstream `Zed` references intentionally left in place for compatibility and why they were preserved.

## Example Handoff

```json
{
  "salientSummary": "Updated the shared release identity and public URL surfaces to Neo Zed. Added targeted tests for release-channel IDs and URL builders, then verified docs and support links now use neozed.dev and the fork repo URL.",
  "whatWasImplemented": "Changed the shared release-channel mapping to Neo Zed display names and dev.neozed-based IDs, updated the public CLI/scheme/domain helpers, and rewrote release-facing docs/support links to use neozed.dev, neozed://, neozed, and the fork repository URL.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test -p release_channel",
        "exitCode": 0,
        "observation": "Release-channel tests passed with the new Neo Zed display names and app IDs."
      },
      {
        "command": "cargo test -p http_client",
        "exitCode": 0,
        "observation": "URL-builder tests passed using neozed.dev, api.neozed.dev, and cloud.neozed.dev."
      },
      {
        "command": "cargo fmt --all -- --check",
        "exitCode": 0,
        "observation": "Formatting remained clean after the Rust edits."
      },
      {
        "command": "rg -n \"neozed|neozed://|dev\\.neozed|neozed\\.dev|Nkr1shna/neo-zed\" crates docs legal assets",
        "exitCode": 0,
        "observation": "Release-facing surfaces now expose the new fork identity tuple."
      }
    ],
    "interactiveChecks": [
      {
        "action": "Inspected updated release identity sources and docs for the public identity tuple",
        "observed": "Observed consistent Neo Zed naming, neozed command examples, neozed:// links, and neozed.dev URLs in the touched files."
      }
    ]
  },
  "tests": {
    "added": [
      {
        "file": "crates/release_channel/tests/release_identity.rs",
        "cases": [
          {
            "name": "channel_display_names_use_neo_zed_branding",
            "verifies": "Stable/preview/nightly/dev display names and app IDs derive from the Neo Zed identity map."
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- The feature needs a public URL, repo URL, mailbox, or ID mapping that is not defined in `.factory/library/release-identity.md`
- A release-facing surface appears compatibility-sensitive and you cannot determine whether it should remain `Zed`
- The only safe validation path would require unavailable publishing/signing credentials
