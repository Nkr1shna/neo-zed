# Architecture

How the detached agent panel PiP feature should fit into Neo Zed.

## What belongs here

- Workspace, dock, and detached-window ownership rules
- High-level relationships between `Workspace`, `AgentPanel`, detached presentation, and GPUI windowing
- Platform invariants for always-on-top support

## Core model

### 1. One agent experience per workspace
- `AgentPanel` remains the single source of truth for agent UI state for a workspace.
- The feature must not create a second independent agent session or duplicate conversation model when detaching.
- Existing per-workspace thread and panel state should continue to flow through the same workspace-owned panel entity.

### 2. Two presentations of the same workspace-owned panel
- **Docked presentation:** the existing panel rendered inside the workspace dock.
- **Detached presentation:** a floating native window with a window-local shell that wraps the same workspace-owned agent experience.
- Only one presentation is visible at a time for a given workspace.
- Preferred pattern: keep `AgentPanel` as the state owner and add a detached shell/root view around it rather than creating a second agent view model.

### 3. Workspace-owned detach controller
- Detached-window lifecycle state should be owned by the workspace layer, not by a second agent view model.
- The controller should track:
  - whether the workspace is currently detached
  - the detached window handle/identity
  - most recently used detached bounds
  - requested always-on-top state for the current app session
- Closing the detached window must restore the docked presentation for that same workspace.

## Dock and routing rules

### Dock suppression
- While detached, the workspace must continue to own `AgentPanel`, but the docked presentation must be suppressed.
- Suppression means:
  - no visible agent panel body in the dock
  - no duplicate docked agent content
  - no empty reserved dock space or ghost shell attributable to the agent panel
- Detach is a presentation-mode switch, not panel deletion.

### Focus and reveal routing
- Normal agent focus/reveal/toggle flows should resolve to the detached window when that workspace is detached.
- After reattachment, those same flows must resolve back to the docked panel.
- Routing must be keyed to the owning workspace, not to the most recently active app window or to project-name similarity.

### Multi-workspace behavior
- Each workspace may have at most one detached agent window.
- Multiple workspaces may each detach independently.
- Actions for workspace A must never focus, mutate, or steal workspace B’s detached agent surface.

## Detached window behavior

### Floating shell
- The detached presentation should be a native floating window that can coexist visibly with the owning workspace window.
- The detached shell should reuse the same visible thread, transcript, and draft state the user had before detaching.
- Once detached, the shell should swap the toolbar affordance from pop-out to pop-in so the user can restore directly to the owning workspace without exposing another duplicate-detach path.
- Pop-in must close the detached window and restore exactly one docked panel in the owning workspace rather than leaving both presentations visible.
- All restore paths, including pop-in and standard detached-window close shortcuts, must route through the same restore logic so they restore the current workspace-owned panel state rather than an older stale presentation.
- Detached fullscreen should apply to the same detached shell instead of recreating a mirrored docked panel in the main window, and pop-in should remain available while fullscreen.
- The detached shell should use the same titlebar style/treatment as the main Neo Zed window for the current platform, while still surfacing detached-specific controls.
- Titlebar parity requires the detached window to use the same custom titlebar treatment/component path as the main workspace window rather than only copying window options or padding under native floating chrome.
- The detached panel header/content must render below the native titlebar with no overlap or conflicting chrome.
- Detached bounds are remembered per workspace within the current app session only.
- Relaunch returns to docked mode; detached presentation itself is not restored across app restart.

## Likely code ownership / entry points

- `crates/agent_ui/src/agent_panel.rs`
  - existing panel actions, toolbar/menu affordances, active-thread handling, panel serialization
- `crates/agent_ui/src/conversation_view.rs`
  - existing focus/reveal and notification-driven panel routing
- `crates/workspace/src/workspace.rs`
  - dock/panel ownership, focus routing, panel lifecycle
- `crates/workspace/src/multi_workspace.rs`
  - workspace/window lifecycle edge cases
- `crates/gpui/src/platform.rs` and `crates/gpui/src/window.rs`
  - shared runtime always-on-top API surface
- `crates/gpui_macos/src/window.rs`, `crates/gpui_windows/src/window.rs`, `crates/gpui_linux/src/linux/x11/window.rs`, `crates/gpui_linux/src/linux/wayland/window.rs`
  - backend behavior for runtime always-on-top and Wayland degradation

### Cleanup
- Closing the detached window restores the docked presentation.
- Closing the owning workspace removes its detached window cleanly.
- No stale focus target, orphaned floating window, or dead reveal path should remain after teardown.

## Always-on-top architecture

### GPUI abstraction
- GPUI needs an explicit runtime always-on-top capability exposed on `Window` / `PlatformWindow`.
- Creation-time floating behavior is not sufficient because the lock must toggle at runtime.
- The detached window should use that API rather than backend-specific logic from agent UI code.

### Platform expectations
- Supported for the lock toggle:
  - macOS
  - Windows
  - Linux/X11
- Graceful degradation:
  - Linux/Wayland supports detach itself, but the lock control should be absent or visibly disabled and must never imply success.

## Required invariants

1. A workspace has one logical agent panel state, regardless of whether it is docked or detached.
2. A workspace has at most one detached agent window at a time.
3. Detaching must never create duplicate visible agent surfaces for the same workspace.
4. Reattaching must preserve visible thread and draft state.
5. Relaunch always returns to docked mode.
6. Always-on-top is runtime-toggleable on supported platforms and explicitly unavailable on Wayland.
