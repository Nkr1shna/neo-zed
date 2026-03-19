# AI Workspace And Dock Sizing Merge Notes

This note documents the migration that removed the docked `AgentPanel` model, moved AI chat to a center-tab `AiWorkspace`, and shifted dock sizing ownership to the host workspace/dock system.

## Scope

- `AgentPanel` was renamed and reworked into `AiWorkspace`.
- The primary AI surface is now a center `Item` via `AgentWorkspaceItem`.
- Workspace-level AI actions route through `agent_workspace_surface` instead of the old dock panel entry points.
- Dock sizing is host-owned. Panels and remote UI manifests no longer control runtime dock width.
- Remote UI `default_size` is deprecated and ignored at runtime.
- New orchestration crates were introduced:
  - `crates/orchestration`
  - `crates/orchestration_ui`

## Intentional Compatibility Leftovers

These are expected and should not be “cleaned up” during merges unless persistence migration is handled explicitly:

- `crates/agent_ui/src/ai_workspace.rs`
  - `const LEGACY_AI_WORKSPACE_KEY: &str = "agent_panel";`
  - the legacy comment describing the old `"agent_panel"` key
  - `const LEGACY_LAST_USED_EXTERNAL_AGENT_KEY: &str = "agent_panel__last_used_external_agent";`

These are kept so older saved state can still be restored.

## Key Architectural Changes

### AI Surface

- `crates/agent_ui/src/ai_workspace.rs`
  - backing AI workspace view/controller
- `crates/agent_ui/src/agent_workspace_item.rs`
  - center-tab wrapper for the AI surface
- `crates/agent_ui/src/agent_workspace_surface.rs`
  - workspace-level open/focus/load/toggle API
- `crates/zed/src/zed.rs`
  - workspace actions now register the center-first AI handlers

### Dock Sizing

- `crates/workspace/src/dock.rs`
- `crates/workspace/src/workspace.rs`
- `crates/workspace/src/persistence.rs`
- `crates/workspace/src/persistence/model.rs`
- `crates/zed/src/zed/remote_ui_extension.rs`
- `crates/extension/src/types/remote_ui.rs`

The host now persists dock size per dock position. Remote UI and built-in panels no longer own right-dock width.

### Orchestration

- `crates/orchestration`
- `crates/orchestration_ui`

This adds a workspace-owned orchestration model plus a center item shell.

## Merge Hotspots

These files are likely to conflict with upstream changes because they sit on common action, workspace, or settings paths:

- `crates/agent_ui/src/agent_ui.rs`
- `crates/agent_ui/src/ai_workspace.rs`
- `crates/agent_ui/src/agent_workspace_surface.rs`
- `crates/agent_ui/src/conversation_view.rs`
- `crates/agent_ui/src/conversation_view/thread_view.rs`
- `crates/sidebar/src/sidebar.rs`
- `crates/workspace/src/workspace.rs`
- `crates/workspace/src/dock.rs`
- `crates/workspace/src/persistence.rs`
- `crates/zed/src/zed.rs`
- `crates/zed/src/main.rs`
- `crates/zed/src/zed/remote_ui_extension.rs`
- `crates/settings_ui/src/page_data.rs`
- `crates/zed_actions/src/lib.rs`

## What To Preserve During Upstream Merges

- Keep the center-first AI action routing.
- Keep `AiWorkspace` naming in runtime code, keymaps, telemetry labels, and onboarding/settings copy.
- Keep dock size persistence attached to dock positions rather than panel state.
- Keep remote UI `default_size` ignored at runtime unless the deprecation policy changes deliberately.
- Keep the legacy persistence keys in `ai_workspace.rs` unless a full migration is added.

## Quick Sanity Checks After A Merge

Run these after rebasing or merging upstream:

```sh
rg -n '\bAgentPanel\b|agent_panel' .
cargo check -p agent_ui -p workspace -p zed
cargo test -p agent_ui toggle_preserves_workspace_controller_after_attach -- --nocapture
```

Expected search result:

- Only the intentional legacy persistence strings in `crates/agent_ui/src/ai_workspace.rs`
