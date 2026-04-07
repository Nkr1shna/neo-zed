# Windowing

Mission-specific notes for detached agent windows and always-on-top behavior.

## What belongs here

- Detached-window lifecycle expectations
- Platform-conditioned always-on-top rules
- Constraints workers should preserve when touching GPUI window APIs

## Detached window rules

- Detached agent windows are native floating windows, not separate workspaces and not full app-window replacements.
- A detached window reuses the owning workspace’s existing `AgentPanel` state.
- Each workspace may have at most one detached agent window.
- Closing the detached window restores the docked panel for the same workspace.

## Always-on-top rules

- Runtime toggling is required for the lock control.
- Supported lock behavior:
  - macOS
  - Windows
  - Linux/X11
- Unsupported lock behavior:
  - Linux/Wayland

## Wayland degradation contract

- Detach itself must still work on Wayland.
- The lock control must be visibly disabled.
- The UI must never present a selected/success state for always-on-top on Wayland.

## Worker guidance

- Keep GPUI backend changes generic and reusable; agent UI code should call a high-level window API rather than platform-specific code.
- Preserve existing floating-window behavior for unrelated windows.
- Prefer additive APIs over special cases wired only for agent UI.
