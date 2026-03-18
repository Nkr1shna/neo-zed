---
title: Remote View UI Extensions (Proposed)
description: "A host-rendered extension UI contract based on remote view trees, structured events, and bounded workspace RPC."
---

# Remote View UI Extensions (Proposed)

This document specifies a host-rendered extension UI system for Zed/neo-zed.
It is a design contract, not a product memo.

> **Note:** This is a proposed contract. It is not a released extension API.

> **Current tree state:** manifest parsing, local indexing, local/dev loading,
> extension-host registration, remote panel mounting, `titlebar_widgets`,
> `footer_widgets`, command-palette registration, and the bounded context-menu
> locations (`editor-context`, `project-panel-context`, `panel-overflow`,
> `item-tab-context`) are implemented in the local runtime. UI-only extensions
> can be exercised locally from installed/dev extension directories, but
> `zed-extension` packaging and catalog publishing still reject them until
> `ExtensionProvides` grows a remote-UI capability. That limitation should
> remain documented until the catalog path lands.

The contract assumes:

- Extensions do not own GPUI entities.
- Extensions return bounded remote view trees.
- The host renders those trees, owns placement and lifecycle, and sends
  structured events back to the extension.
- Extensions can dispatch existing workspace actions and call explicit host APIs
  for bounded mutations.

## Problem Statement

The current extension system is feature-oriented. Extensions can contribute
languages, language servers, slash commands, themes, debuggers, and similar
capabilities, but there is no retained extension UI contract.

The host already owns the relevant UI surfaces:

- `Workspace` owns the title bar mount point.
- `StatusBar` owns footer/status mounts.
- `Dock` and `Panel` own docking, resizing, focus, and activation.
- The command palette, menus, and keybinding system are host-owned.

That ownership is correct and should remain in the host. What is missing is a
bounded way for extensions to declare UI and respond to user interaction without
crossing the GPUI boundary.

## Design Principles

1. Host-rendered, not guest-rendered

The extension returns data. The host turns that data into GPUI.

2. Declarative, bounded, and semantic

The extension can compose from a limited element set. It cannot emit arbitrary
GPUI or arbitrary host object references.

3. Host authority over integration

The host owns window placement, docking, focus, scheduling, theming,
virtualization, and security decisions.

4. Stable identity across rerenders

View and node identity must be explicit and stable so the host can diff,
preserve focus when appropriate, and recover from invalid updates.

5. Structured RPC, not callback leaks

Events, action dispatch, and host mutations cross the boundary as explicit
messages with validated payloads.

6. First-class workspace actions

Extension commands must be visible to the command palette, keybindings, menus,
and context actions as workspace-level actions, even though they are ultimately
dispatched through the extension host.

7. No background churn

Remote views rerender because of a host event, a host context change, or an
explicit host-mediated completion. They do not rerender from free-running guest
timers.

## Proposed Contracts

### 1. Remote View Architecture

The host adds a new extension contribution subsystem with three layers:

1. A static contribution registry loaded from `extension.toml`
2. A runtime remote-view host in the workspace process
3. A WIT/RPC surface implemented by the extension guest

At load time, the host reads manifest contributions and registers:

- `commands`
- `panels`
- `titlebar_widgets`
- `footer_widgets`
- `menus`
- `context_actions`

No view instance is created at load time. View instances are created lazily when
the host mounts a visible surface.

Each mounted surface produces a `RemoteViewInstance`:

- `extension_id`
- `contribution_id`
- `instance_id`
- `mount_kind`
- `workspace_id`
- `revision`

The host owns instance allocation and teardown. The extension only receives the
opaque `instance_id`.

### 2. View Composition Contract

#### Tree shape

Each mounted surface has exactly one root node.

Each node has:

- `node_id`: stable within the instance
- `kind`: bounded element kind
- `props`: bounded semantic props
- `children`: ordered child list

The root tree is small and semantic. Large collections use a dedicated virtual
list node instead of materializing arbitrary child counts.

#### Bounded element set

The initial element set is:

- `row`
- `column`
- `stack`
- `text`
- `icon`
- `button`
- `toggle`
- `checkbox`
- `text_input`
- `badge`
- `divider`
- `spacer`
- `scroll_view`
- `virtual_list`

Deliberate exclusions in v1:

- arbitrary canvas drawing
- raw HTML or webviews
- arbitrary image decoding
- custom focus rings
- custom layout primitives outside the bounded set
- arbitrary host view embedding

#### Props and styling

Styling is semantic, not CSS-like. Props are expressed through host-defined
tokens and enums.

Supported prop groups:

- layout: `width`, `height`, `min_width`, `max_width`, `padding`, `margin`,
  `gap`, `align`, `justify`, `grow`, `shrink`
- appearance: `tone`, `emphasis`, `surface`, `border`, `rounded`
- text: `text`, `style`, `weight`, `truncate`, `tooltip`
- interaction: `enabled`, `visible`, `on_click`, `on_change`, `on_submit`
- accessibility: `label`, `description`
- collection: `item_count`, `estimated_row_height`, `selection_mode`

The extension cannot set raw colors, fonts, z-order rules, or theme values.
Those remain host-owned and map onto the current theme system.

#### Stable identity

Identity works at three levels:

- contribution identity: manifest `id`
- instance identity: host-generated `instance_id`
- node identity: extension-provided `node_id`

Rules:

- `node_id` must be stable across rerenders for semantically identical nodes.
- `node_id` must be unique within the instance tree.
- Reusing a `node_id` for a different `kind` is invalid and forces subtree
  replacement.
- `virtual_list` items use a separate stable `item_key`.

#### Event flow

Only nodes that declare an event handler receive events.

The host emits structured events:

- `click`
- `toggle_changed`
- `text_changed`
- `submit`
- `focus_requested`
- `list_item_activated`
- `list_selection_changed`
- `context_menu_requested`

Each event includes:

- `instance_id`
- `node_id`
- `kind`
- `payload`
- `event_context`

`event_context` is bounded host state such as:

- `workspace_id`
- `trusted`
- `active_item_kind`
- `appearance`
- `modifiers`

The extension processes the event, may call host APIs, updates its own internal
state, and returns one of:

- `noop`
- `rerender`
- `rerender_virtual_range(node_id)`
- `show_error(message)`

The host coalesces rerenders per instance.

#### Mutation requests and action dispatch

The extension cannot mutate host state directly. It can only call explicit
imports:

- `dispatch_workspace_action`
- `dispatch_registered_command`
- `request_host_mutation`

`request_host_mutation` is bounded to a small set:

- `show_toast`
- `open_panel`
- `close_panel`
- `copy_to_clipboard`
- `open_external_url`

Current expectation for runtime bring-up:

- `open_panel` and `close_panel` are wired for registered remote-ui panels in
  the local runtime.
- Unknown panel identifiers, unknown workspaces, or policy-rejected requests
  still return host errors.
- `set_panel_badge` and `set_panel_title` are future host mutations and are not
  part of the current wire type.

The host may reject any request based on workspace state, trust, platform, or
policy.

### 3. Workspace Integration Contract

#### Commands

Extensions declare commands in `extension.toml`.

Each command has:

- `id`
- `title`
- `description`
- `palette`
- `input_schema` (optional JSON schema path)
- `when` (optional host expression)

Commands are first-class workspace actions, but they do not require generating
new GPUI action types per extension.

Instead, the host provides one static transport action:

`extensions::RunRegisteredCommand { command_id, input_json }`

The command palette, menus, and keymaps resolve command metadata from an
`ExtensionCommandRegistry`, then dispatch the transport action with the command
identifier.

This avoids changing GPUI's static action inventory model for each extension
command while still making extension commands first-class in workspace surfaces.

#### Menus and context actions

Extensions can register menu items and context actions only in host-defined
locations.

Supported locations in v1:

- command palette
- editor context menu
- project panel context menu
- panel overflow menu
- item tab context menu

Current runtime status:

- All listed locations are wired through host-owned registries in the local app
  runtime.
- `when` and `group` are accepted and validated in manifest parsing, but they
  are not yet interpreted by the runtime.

Explicit non-goals in v1:

- arbitrary top-level application menus
- arbitrary menu sections created by extensions

Each menu or context action references a registered command by `command_id`.
The host evaluates the `when` clause against host-owned context keys.

#### Panels

Extensions can declare dockable panels:

- `id`
- `title`
- `icon`
- `default_dock`
- `default_size`
- `root_view`
- `toggle_command`

The panel itself is host-owned. The extension only supplies the remote root
view for the panel body.

The host controls:

- whether the panel mounts at all
- actual dock side
- title normalization
- icon fallback
- focus transfer
- visibility persistence
- resizing and zoom behavior

#### `titlebar_widgets` and `footer_widgets`

Extensions can contribute lightweight remote views to the title bar and footer.

`titlebar_widgets` and `footer_widgets` are manifest fields and remain
host-controlled surfaces.

Each descriptor includes:

- `id`
- `root_view`
- `size`
- `priority`
- `when`

Titlebar widgets also include:

- `side`

Footer widgets include:

- `zone`

The current local runtime standardizes widget slots:

- Titlebar sizes:
  - `s`: one compact affordance, usually one icon or badge
  - `m`: two compact units, such as `icon + text` or `icon + badge`
  - `l`: a richer compact cluster, such as `icon + text + badge`
- Footer zones:
  - `left` and `right`: icon-only status widgets, `size = "s"` only
  - `center`: richer status widgets with `size = "s" | "m" | "l"`

The host clamps width budgets by surface and size:

- titlebar `s`: 24 px
- titlebar `m`: 64 px
- titlebar `l`: 112 px
- footer edge slots: 24 px
- footer center `s`: 24 px
- footer center `m`: 96 px
- footer center `l`: 180 px

The current local runtime also validates content-unit budgets:

- titlebar `s`: 1 content unit
- titlebar `m`: 2 content units
- titlebar `l`: 4 content units
- footer edge slots: exactly 1 icon
- footer center `s`: 1 content unit
- footer center `m`: 2 content units
- footer center `l`: 4 content units

The host may:

- accept the widget
- reject the widget
- hide it due to lack of space
- remap it into an overflow affordance
- defer mounting until the surface is visible

The host is never required to honor the requested side or exact order.
The current runtime preserves side and priority ordering for mounted widgets,
but overflow/remap behavior is still future work.

### 4. Clear Boundaries and Non-Goals

The following are out of scope by design:

- direct GPUI entity ownership by extensions
- direct access to `Window`, `Context<T>`, `App`, `AsyncApp`, or focus handles
- arbitrary host object references crossing the boundary
- arbitrary background rerender loops
- arbitrary host callbacks or borrowed references surviving across calls
- raw OS handles, process handles, or thread access
- webview escape hatches
- arbitrary filesystem or network expansion beyond existing extension
  capabilities

The host remains authoritative for:

- GPUI entity lifecycle
- focus and window integration
- foreground-thread scheduling
- virtualization
- panel docking
- theming
- security policy

## Example API Sketch

### Manifest additions

```toml
[commands.sample.open-panel]
title = "Sample: Open Panel"
description = "Open the Sample panel"
palette = true

[commands.sample.refresh]
title = "Sample: Refresh"
description = "Refresh sample data"
palette = true
when = "workspace.trusted"

[panels.sample]
title = "Sample"
icon = "bolt"
default_dock = "right"
default_size = 320
root_view = "sample.panel"
toggle_command = "sample.open-panel"

[[titlebar_widgets]]
id = "sample.sync"
root_view = "sample.titlebar"
side = "right"
size = "m"
priority = 300
when = "workspace.has_project"

[[footer_widgets]]
id = "sample.status"
root_view = "sample.footer"
zone = "center"
size = "l"
priority = 200
when = "workspace.has_project"

[[context_actions]]
id = "sample.explain-selection"
title = "Sample: Explain Selection"
target = "editor"
command = "sample.explain-selection"
when = "editor.has_selection && workspace.trusted"
```

### WIT sketch

```wit
interface remote-ui {
    enum mount-kind {
        titlebar-widget,
        footer-widget,
        panel,
    }

    enum render-reason {
        initial,
        event,
        host-context-changed,
        virtual-range-changed,
        explicit-refresh,
    }

    record mount-context {
        workspace-id: u64,
        contribution-id: string,
        mount-kind: mount-kind,
        appearance: string,
        trusted: bool,
        active-item-kind: option<string>,
    }

    record length {
        kind: string,
        value: option<u32>,
    }

    record view-style {
        padding: option<u32>,
        gap: option<u32>,
        grow: bool,
        shrink: bool,
        min-width: option<length>,
        max-width: option<length>,
        tone: option<string>,
        text-style: option<string>,
        align: option<string>,
        justify: option<string>,
    }

    record virtual-list-props {
        item-count: u32,
        estimated-row-height: u32,
        selection-mode: option<string>,
    }

    variant node-kind {
        row,
        column,
        stack,
        text(string),
        icon(string),
        button(string),
        toggle(bool),
        checkbox(bool),
        text-input(string),
        badge(string),
        divider,
        spacer,
        scroll-view,
        virtual-list(virtual-list-props),
    }

    record view-node {
        node-id: string,
        kind: node-kind,
        style: option<view-style>,
        tooltip: option<string>,
        children: list<view-node>,
    }

    record render-result {
        revision: u64,
        root: view-node,
    }

    variant event-payload {
        none,
        click,
        toggle(bool),
        text(string),
        submit(string),
        list-activate(string),
        list-selection(list<string>),
    }

    record view-event {
        node-id: string,
        kind: string,
        payload: event-payload,
    }

    variant event-outcome {
        noop,
        rerender,
        rerender-virtual-range(string),
        show-error(string),
    }

    variant host-mutation {
        show-toast(string),
        open-panel(string),
        close-panel(string),
        copy-to-clipboard(string),
        open-external-url(string),
    }

    import dispatch-workspace-action:
        func(workspace-id: u64, action-id: string, payload-json: option<string>)
            -> result<_, string>;

    import request-host-mutation:
        func(workspace-id: u64, mutation: host-mutation)
            -> result<_, string>;

    export open-view:
        func(contribution-id: string, context: mount-context)
            -> result<u64, string>;

    export render-view:
        func(instance-id: u64, context: mount-context, reason: render-reason)
            -> result<render-result, string>;

    export handle-view-event:
        func(instance-id: u64, context: mount-context, event: view-event)
            -> result<event-outcome, string>;

    export render-virtual-list-range:
        func(instance-id: u64, node-id: string, start: u32, end: u32,
             context: mount-context)
            -> result<list<view-node>, string>;

    export close-view: func(instance-id: u64);
}
```

### Host-side Rust sketch

```rust
pub type ExtensionCommandId = Arc<str>;
pub type RemoteViewInstanceId = u64;
pub type RemoteNodeId = Arc<str>;

pub struct ExtensionCommandContribution {
    pub extension_id: Arc<str>,
    pub command_id: ExtensionCommandId,
    pub title: String,
    pub description: Option<String>,
    pub palette: bool,
    pub when: Option<String>,
    pub input_schema_path: Option<PathBuf>,
}

pub enum RemoteMountKind {
    TitlebarWidget,
    FooterWidget,
    Panel,
}

pub struct RemoteMountContext {
    pub workspace_id: u64,
    pub contribution_id: Arc<str>,
    pub mount_kind: RemoteMountKind,
    pub trusted: bool,
    pub active_item_kind: Option<Arc<str>>,
}

pub trait ExtensionUiProxy: Send + Sync + 'static {
    fn register_commands(
        &self,
        commands: Vec<ExtensionCommandContribution>,
    );

    fn register_titlebar_widgets(
        &self,
        widgets: Vec<TitlebarWidgetContribution>,
    );

    fn register_footer_widgets(
        &self,
        widgets: Vec<FooterWidgetContribution>,
    );

    fn register_panels(
        &self,
        panels: Vec<ExtensionPanelContribution>,
    );

    fn register_context_actions(
        &self,
        actions: Vec<ExtensionContextActionContribution>,
    );

    fn remove_extension_ui(&self, extension_id: &Arc<str>);
}

pub trait RemoteViewRuntime: Send + Sync {
    fn mount(
        &self,
        extension: Arc<dyn extension::Extension>,
        context: RemoteMountContext,
    ) -> Task<anyhow::Result<RemoteViewInstanceId>>;

    fn render(
        &self,
        instance_id: RemoteViewInstanceId,
        reason: RenderReason,
    ) -> Task<anyhow::Result<RemoteViewTree>>;

    fn handle_event(
        &self,
        instance_id: RemoteViewInstanceId,
        event: RemoteViewEvent,
    ) -> Task<anyhow::Result<EventOutcome>>;

    fn unmount(&self, instance_id: RemoteViewInstanceId);
}
```

### Extension-side Rust sketch

```rust
pub trait Extension: Send + Sync {
    fn new() -> Self
    where
        Self: Sized;

    fn run_command(
        &mut self,
        command_id: &str,
        context: CommandContext,
        input: Option<serde_json::Value>,
    ) -> Result<(), String> {
        Err(format!("command `{command_id}` not implemented"))
    }

    fn open_remote_view(
        &mut self,
        contribution_id: &str,
        context: &RemoteMountContext,
    ) -> Result<RemoteViewInstanceId, String> {
        Err(format!("view `{contribution_id}` not implemented"))
    }

    fn render_remote_view(
        &mut self,
        instance_id: RemoteViewInstanceId,
        context: &RemoteMountContext,
        reason: RenderReason,
    ) -> Result<RemoteViewTree, String> {
        Err(format!("view instance `{instance_id}` not implemented"))
    }

    fn handle_remote_view_event(
        &mut self,
        instance_id: RemoteViewInstanceId,
        context: &RemoteMountContext,
        event: RemoteViewEvent,
    ) -> Result<EventOutcome, String> {
        Ok(EventOutcome::Noop)
    }

    fn close_remote_view(
        &mut self,
        instance_id: RemoteViewInstanceId,
    ) {}
}
```

### Example extension

```rust
use std::collections::HashMap;

use zed_extension_api as zed;

struct SampleExtension {
    views: HashMap<u64, SampleViewState>,
    next_id: u64,
}

struct SampleViewState {
    clicks: u32,
}

impl zed::Extension for SampleExtension {
    fn new() -> Self {
        Self {
            views: HashMap::new(),
            next_id: 1,
        }
    }

    fn open_remote_view(
        &mut self,
        contribution_id: &str,
        _context: &zed::RemoteMountContext,
    ) -> zed::Result<u64> {
        let instance_id = self.next_id;
        self.next_id += 1;
        self.views
            .insert(instance_id, SampleViewState { clicks: 0 });
        match contribution_id {
            "sample.footer" | "sample.panel" | "sample.titlebar" => Ok(instance_id),
            _ => Err(format!("unknown contribution `{contribution_id}`")),
        }
    }

    fn render_remote_view(
        &mut self,
        instance_id: u64,
        _context: &zed::RemoteMountContext,
        _reason: zed::RenderReason,
    ) -> zed::Result<zed::RemoteViewTree> {
        let state = self
            .views
            .get(&instance_id)
            .ok_or_else(|| "unknown instance".to_string())?;

        Ok(zed::RemoteViewTree::column(
            "root",
            vec![
                zed::RemoteNode::text("label", format!("Clicks: {}", state.clicks)),
                zed::RemoteNode::button("refresh", "Refresh"),
            ],
        ))
    }

    fn handle_remote_view_event(
        &mut self,
        instance_id: u64,
        context: &zed::RemoteMountContext,
        event: zed::RemoteViewEvent,
    ) -> zed::Result<zed::EventOutcome> {
        let state = self
            .views
            .get_mut(&instance_id)
            .ok_or_else(|| "unknown instance".to_string())?;

        if event.node_id == "refresh" && event.kind == zed::RemoteEventKind::Click {
            state.clicks += 1;
            zed::dispatch_workspace_action(
                context.workspace_id,
                "command_palette::Toggle",
                None,
            )?;
            return Ok(zed::EventOutcome::Rerender);
        }

        Ok(zed::EventOutcome::Noop)
    }

    fn close_remote_view(&mut self, instance_id: u64) {
        self.views.remove(&instance_id);
    }
}

zed::register_extension!(SampleExtension);
```

## Example Event Flow

Opening a panel:

1. The host resolves `sample.open-panel` from the extension command registry.
2. The command dispatches the host transport action
   `extensions::RunRegisteredCommand`.
3. The host decides that the command opens panel `sample`.
4. The host mounts panel `sample`, allocates `instance_id = 42`, and calls
   `open-view("sample.panel", context)`.
5. The extension creates instance state and returns `42`.
6. The host calls `render-view(42, context, initial)`.
7. The extension returns a bounded tree.
8. The host validates the tree, renders GPUI nodes, and wires event handlers.

Handling a click:

1. The user clicks the node with `node_id = "refresh"`.
2. The host emits `view-event { instance_id: 42, node_id: "refresh", kind:
"click" }`.
3. The extension updates its instance state.
4. The extension optionally calls `dispatch-workspace-action` or
   `request-host-mutation`.
5. The extension returns `rerender`.
6. The host coalesces the rerender and calls `render-view(42, context, event)`.
7. The host diffs the new tree against revision `42:n` and updates only changed
   nodes.

Virtual list updates:

1. The host scrolls a `virtual_list`.
2. The host computes the visible range.
3. The host calls `render-virtual-list-range(instance_id, node_id, start, end)`.
4. The extension returns rows for that range only.
5. The host owns recycling, measurement, and scroll state.

## Lifecycle Semantics

### Initial mount

- The host validates manifest contributions at load time.
- The host creates a view instance only when a surface becomes visible or active.
- `open-view` is called exactly once per mounted instance.
- `render-view(..., initial)` follows immediately after a successful open.

### Rerender and diff

- The extension never mutates host UI directly.
- After `render-view`, the host validates the tree and diffs it against the
  previous revision.
- `node_id` and `item_key` are the reconciliation keys.
- In v1 the wire format is full-tree replacement for the root and range-based
  replacement for virtual lists. Diffing happens in the host.
- Patch-based RPC can be added later, but it is not required for the first
  implementation.

### Event delivery

- Events are serialized per instance.
- The host never delivers two concurrent events to the same instance.
- If the extension returns `rerender`, the host schedules exactly one follow-up
  render for that instance even if multiple events arrive in the same turn.

### Teardown

- `close-view` is called when the host unmounts the surface, unloads the
  extension, or recovers from a terminal instance error.
- After `close-view`, the host must discard any queued events for the instance.
- The extension must treat unknown `instance_id` as recoverable error input.

### Error handling and recovery

Host-side validation errors:

- unknown element kind
- duplicate `node_id`
- unsupported prop on node kind
- invalid `virtual_list` child shape
- type change for stable `node_id`

Recovery rules:

- Reject the new tree.
- Keep the last known-good tree if one exists.
- Show a host error placeholder for first render failures.
- Log the extension ID, contribution ID, instance ID, and validation message.
- Tear down the instance after repeated failures in the same mount.

Guest-side runtime errors:

- An event error does not crash the workspace.
- The host keeps the previous tree and surfaces the error in logs.
- If an extension repeatedly errors, the host may disable UI contributions from
  that extension until reload.

## Security Model

The security boundary is the core reason to use host-rendered remote views.

Rules:

- Extensions do not receive GPUI entities, raw window handles, or arbitrary host
  references.
- The host never executes extension-provided layout code on the foreground UI
  thread.
- All cross-boundary messages are validated against explicit schemas.
- Host mutations are allowlisted RPCs, not arbitrary method calls.
- Commands, menus, and context actions are declared statically in the manifest
  and validated before registration.
- `when` expressions are host-evaluated against host-owned context keys. The
  extension does not run policy code inside the host.
- Trust-gated surfaces can be suppressed entirely in untrusted workspaces.
- URL opening, clipboard, and panel mutations remain explicit audited host APIs.

This model intentionally provides no escape hatch for:

- arbitrary JS or HTML embedding
- arbitrary host callback registration
- raw event taps
- foreground-thread guest execution

## Performance and Correctness Constraints

1. Stable IDs are mandatory

`node_id` and `item_key` must be stable. The host should reject unstable trees
in debug builds and log them in release builds.

2. No unconditional timer-driven rerenders

The host only rerenders due to:

- user events
- host context changes
- explicit command execution
- completion of a host-mediated operation

3. Host owns virtualization

Large lists use `virtual_list`. The extension renders visible ranges. The host
owns measurement, scroll position, recycling, and range requests.

4. Small-tree bias for title bar and footer

`titlebar_widgets` and `footer_widgets` must remain lightweight. The host may
enforce node count, depth, and render-time budgets for these surfaces.

5. Coalesced rerenders

The host should coalesce repeated `rerender` outcomes within the same event
turn.

6. No direct mutation during render

Imports that mutate host state are allowed during command or event handling, not
while `render-view` is running.

7. Tree validation before mount

The host validates the tree before creating GPUI nodes. Invalid trees never
partially mount.

## Migration Plan

### Phase 0: registry and transport action

- Add manifest parsing for `commands`, `panels`, `titlebar_widgets`,
  `footer_widgets`, `menus`, and `context_actions`.
- Add `ExtensionCommandRegistry`.
- Add the generic transport action `extensions::RunRegisteredCommand`.
- Teach the command palette, keybinding layer, and menu builders to resolve
  extension commands through the registry.

### Phase 1: remote view runtime for widgets

- Add WIT exports/imports for remote views.
- Add a workspace-owned `RemoteViewHost`.
- Mount `titlebar_widgets` and `footer_widgets` through the remote view runtime.
- Keep title bar and status bar placement host-owned.

This is the migration from the narrowed widget contract toward the full
remote-view-tree model. In this repository there is no committed extension
widget contract yet, so this phase is additive rather than destructive.

### Phase 2: panel support

- Add host-owned extension panels backed by remote root views.
- Register panel toggle/open actions through the command registry.
- Persist dock placement and visibility in the same host mechanisms used by
  other panels.

### Phase 3: menus and context actions

- Allow extension commands to appear in bounded host menu locations.
- Evaluate `when` expressions entirely in the host.
- Add context payload shaping for supported targets.

### Phase 4: virtual lists and optimizations

- Add `virtual_list`.
- Add range-based render calls.
- Add optional patch-based RPC only if host-side diffing is insufficient.

## Documentation Requirements for Extension Authors

When this lands, extension author docs must include:

- manifest reference for all new contribution fields
- supported element kinds and props
- stable ID rules with examples of correct and incorrect usage
- event types and payload schemas
- command registration and keybinding examples
- panel, `titlebar_widgets`, and `footer_widgets` examples
- `when` expression reference
- trust and security limitations
- performance limits and validation rules
- error recovery behavior

The documentation set should include one complete example extension that shows:

- one command palette command
- one panel
- one title bar widget
- one footer widget
- one context action
- one structured event that dispatches an existing workspace action

## Testing and Validation Plan

### Acceptance criteria

Implemented now:

- An extension can declare a panel, a title bar widget, and a footer widget
  without owning GPUI.
- The host can mount and unmount each surface lazily.
- Structured events reach the extension and can trigger rerender.
- The extension can dispatch an existing workspace action through the host.
- The extension can request `show_toast`, `open_panel`, `close_panel`,
  `copy_to_clipboard`, and `open_external_url`, and the host can reject them.
- Invalid trees fail safely without crashing the workspace.
- The command palette can list and run registered extension commands.
- Keybindings can invoke registered extension commands through the transport
  action.
- The bounded context-menu locations are wired:
  `editor-context`, `project-panel-context`, `panel-overflow`, and
  `item-tab-context`.
- Panels remain host-dockable and host-virtualized.
- No free-running timer loop can force continuous rerender.

Still pending:

- host evaluation of `when`
- host use of `group`
- titlebar/footer overflow and remap behavior
- explicit panel badge/title mutation support

### Host tests

- manifest parsing for each new contribution type
- registration and removal on extension reload
- command registry resolution and transport action dispatch
- view instance lifecycle: mount, render, event, unmount
- tree validation failures and recovery
- host rejection of unauthorized mutations
- title bar and footer overflow/remap behavior
- panel persistence and dock remapping
- virtual list range requests

### Wasm host tests

- WIT version negotiation for the new interface
- guest errors during `open-view`, `render-view`, and `handle-view-event`
- event serialization per instance
- unload behavior with live mounted views

### UI tests

- command palette visibility for extension commands
- keybinding dispatch for extension commands
- panel mount/unmount and docking
- editor context menu registration and command dispatch
- project panel context menu registration and command dispatch
- panel overflow registration and panel-scoped filtering
- item tab context menu registration and command dispatch
- `titlebar_widgets` and `footer_widgets` ordering and overflow
- focus retention on rerender where the host allows it
- fallback placeholder rendering after invalid tree output

### Performance tests

- rerender latency for small title bar and footer trees
- panel rerender with moderate tree depth
- virtual list scrolling under host range requests
- reload behavior with many registered extension commands

## Open Questions

1. Should extension commands support user-provided JSON arguments directly from
   the command palette, or only from keymaps and host menus?

2. Do we want a separate host API for panel-local state such as loading badges
   and empty-state messages, or is `request-host-mutation` sufficient?

3. Should remote-view UI be allowed for remotely loaded extensions, or should
   v1 require all UI-capable extensions to execute on the local UI host?

4. Do text inputs need full IME and composition event support in v1, or can the
   initial contract limit text entry to simpler fields?

5. Is host-side full-tree diffing sufficient for panel-sized trees, or do we
   need patch-based RPC immediately for acceptable performance?
