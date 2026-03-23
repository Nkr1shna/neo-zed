# Workflow Sidebar Integration

**Date:** 2026-03-23
**Workstream:** 1 of 5 — Sidebar navigation tabs
**Crate to modify:** `crates/sidebar`
**Depends on:** Workstream 5 (workflow_ui crate must exist first for its types)

---

## Goal

Add two new view modes to the existing `Sidebar` in `crates/sidebar/src/sidebar.rs`:
- **Workflow Definitions** — lists saved workflow definitions from the runtime
- **Workflow Runs** — lists active and past workflow runs

Three icon-button tabs in the sidebar header switch between: Threads / Workflow Defs / Workflow Runs.

**Important:** This is the left-side agent sidebar (the component that contains agent
threads), NOT a left-dock panel. Do not implement this as a `Panel` trait implementor.

---

## Step 1: Add fields to `Sidebar` struct

```rust
pub struct Sidebar {
    // ... existing fields ...

    /// Lazily-created workflow views. Stored here (not inside SidebarView)
    /// so state persists when the user switches away and back.
    workflow_defs_view: Option<Entity<WorkflowDefsView>>,
    workflow_runs_view: Option<Entity<WorkflowRunsView>>,

    /// Shared HTTP client for workflow views
    workflow_client: Arc<WorkflowClient>,
}
```

`WorkflowDefsView` and `WorkflowRunsView` are from the `workflow_ui` crate.
`workflow_client` is created once (`WorkflowClient::new()`) during `Sidebar::new`.

---

## Step 2: Extend `SidebarView` enum

The existing enum in `sidebar.rs` is:
```rust
enum SidebarView {
    #[default]
    ThreadList,
    Archive(Entity<ThreadsArchiveView>),
}
```

Extend it:
```rust
enum SidebarView {
    #[default]
    ThreadList,
    Archive(Entity<ThreadsArchiveView>),
    WorkflowDefs,
    WorkflowRuns,
}
```

> **Note:** `WorkflowDefs` and `WorkflowRuns` carry no `Entity` inline — the entities
> live in the `Sidebar` struct fields above. This avoids recreating entities on every
> tab switch (achieving state persistence) while keeping the enum simple.

> **Debug requirement:** `SidebarView` derives `Debug`. Since the new variants carry
> no associated data, the derive continues to work without changes. Verify the existing
> derive at line 67 before proceeding.

---

## Step 3: Fix non-exhaustive match arms

After adding new variants, update **every** `match self.view` and `match &self.view`
in `sidebar.rs`. Key sites:

### `toggle_archive` (line ~2722)

```rust
fn toggle_archive(&mut self, _: &ToggleArchive, window: &mut Window, cx: &mut Context<Self>) {
    match &self.view {
        SidebarView::ThreadList | SidebarView::WorkflowDefs | SidebarView::WorkflowRuns => {
            self.show_archive(window, cx)
        }
        SidebarView::Archive(_) => self.show_thread_list(window, cx),
    }
}
```

Any other `match self.view` sites — handle new variants by either:
- Treating them like `ThreadList` (most cases), or
- Adding an explicit no-op arm `SidebarView::WorkflowDefs | SidebarView::WorkflowRuns => {}`

Search the file for `match.*self\.view` and `match.*view` before implementing
to find all sites.

---

## Step 4: Add actions

```rust
gpui::actions!(
    agents_sidebar,
    [
        NewThreadInGroup,
        ToggleArchive,
        ShowWorkflowDefs,   // new
        ShowWorkflowRuns,   // new
    ]
);
```

Register handlers in `Sidebar::new` (alongside existing `on_action` registrations):
```rust
cx.on_action(cx.listener(Self::show_workflow_defs_action));
cx.on_action(cx.listener(Self::show_workflow_runs_action));
```

---

## Step 5: View switching methods

```rust
impl Sidebar {
    fn show_workflow_defs_action(&mut self, _: &ShowWorkflowDefs, window: &mut Window, cx: &mut Context<Self>) {
        self.show_workflow_defs(window, cx);
    }

    fn show_workflow_runs_action(&mut self, _: &ShowWorkflowRuns, window: &mut Window, cx: &mut Context<Self>) {
        self.show_workflow_runs(window, cx);
    }

    fn show_workflow_defs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Lazy init: only create entity once
        if self.workflow_defs_view.is_none() {
            let client = self.workflow_client.clone();
            // Note: window is captured from outer scope, NOT passed as closure param.
            // cx.new closure receives only &mut Context<WorkflowDefsView>.
            let view = cx.new(|cx| WorkflowDefsView::new(client, cx));
            self.workflow_defs_view = Some(view);
        }
        self.view = SidebarView::WorkflowDefs;
        cx.notify();
    }

    fn show_workflow_runs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workflow_runs_view.is_none() {
            let client = self.workflow_client.clone();
            let view = cx.new(|cx| WorkflowRunsView::new(client, cx));
            self.workflow_runs_view = Some(view);
        }
        self.view = SidebarView::WorkflowRuns;
        cx.notify();
    }
}
```

> **Constructor signatures for `WorkflowDefsView::new` and `WorkflowRunsView::new`:**
> Both take `(client: Arc<WorkflowClient>, cx: &mut Context<Self>)`.
> They do NOT take a `window` argument (window is not needed at construction time;
> it is received in `Render::render`).

---

## Step 6: Render — extend the main `match` in `Render`

In `Render for Sidebar` (around line 2870), the existing map:
```rust
.map(|this| match &self.view {
    SidebarView::ThreadList => this
        .child(self.render_sidebar_header(...))
        ...
    SidebarView::Archive(view) => this
        ...
})
```

Add:
```rust
SidebarView::WorkflowDefs => {
    if let Some(view) = &self.workflow_defs_view {
        this.child(self.render_workflow_tab_header(window, cx))
            .child(view.clone())
    } else {
        this  // fallback: entity not yet created (should not happen)
    }
}
SidebarView::WorkflowRuns => {
    if let Some(view) = &self.workflow_runs_view {
        this.child(self.render_workflow_tab_header(window, cx))
            .child(view.clone())
    } else {
        this
    }
}
```

---

## Step 7: Tab header implementation

Add `render_workflow_tab_header` — a simplified header with no search filter,
showing the three navigation tabs and a close/back button:

```rust
fn render_workflow_tab_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let header_height = platform_title_bar_height(window);
    let traffic_lights = cfg!(target_os = "macos") && !window.is_fullscreen();

    h_flex()
        .h(header_height)
        .mt_px()
        .pb_px()
        .border_b_1()
        .border_color(cx.theme().colors().border)
        .when(traffic_lights, |this| {
            this.pl(px(ui::utils::TRAFFIC_LIGHT_PADDING))
        })
        .pr_1p5()
        .gap_1()
        .child(self.render_sidebar_toggle_button(cx))
        .child(Divider::vertical().color(ui::DividerColor::Border))
        .child(self.render_view_tabs(cx))
}
```

---

## Step 8: Tab icons in `render_sidebar_header`

Modify the existing `render_sidebar_header` to add view-switching tabs.
Add the tab row **regardless of `no_open_projects`** (tabs should always be visible):

```rust
fn render_sidebar_header(&self, no_open_projects: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
    let header_height = platform_title_bar_height(window);
    let traffic_lights = cfg!(target_os = "macos") && !window.is_fullscreen();
    let has_query = self.has_filter_query(cx);

    h_flex()
        .h(header_height)
        .mt_px()
        .pb_px()
        .when(traffic_lights, |this| this.pl(px(ui::utils::TRAFFIC_LIGHT_PADDING)))
        .pr_1p5()
        .gap_1()
        // existing close-toggle button
        .child(self.render_sidebar_toggle_button(cx))
        .child(Divider::vertical().color(ui::DividerColor::Border))
        // NEW: view-switching tabs
        .child(self.render_view_tabs(cx))
        // existing search filter (only in thread-list mode when projects open)
        .when(!no_open_projects && matches!(self.view, SidebarView::ThreadList), |this| {
            this.child(Divider::vertical().color(ui::DividerColor::Border))
                .child(div().ml_1().child(
                    Icon::new(IconName::MagnifyingGlass).size(IconSize::Small).color(Color::Muted),
                ))
                .child(self.render_filter_input(cx))
                .child(/* existing clear filter button */)
        })
}
```

---

## Step 9: `render_view_tabs`

```rust
fn render_view_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let is_threads = matches!(self.view, SidebarView::ThreadList | SidebarView::Archive(_));
    let is_defs = matches!(self.view, SidebarView::WorkflowDefs);
    let is_runs = matches!(self.view, SidebarView::WorkflowRuns);

    h_flex()
        .gap_0p5()
        .child(
            IconButton::new("tab-threads", IconName::ZedAgent)
                .icon_size(IconSize::Small)
                .icon_color(if is_threads { Color::Accent } else { Color::Muted })
                .tooltip(Tooltip::text("Agent Threads"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_thread_list(window, cx);
                })),
        )
        .child(
            IconButton::new("tab-workflow-defs", IconName::Ai)   // verify name exists
                .icon_size(IconSize::Small)
                .icon_color(if is_defs { Color::Accent } else { Color::Muted })
                .tooltip(Tooltip::text("Workflow Definitions"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_workflow_defs(window, cx);
                })),
        )
        .child(
            IconButton::new("tab-workflow-runs", IconName::Play)  // verify name exists
                .icon_size(IconSize::Small)
                .icon_color(if is_runs { Color::Accent } else { Color::Muted })
                .tooltip(Tooltip::text("Workflow Runs"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_workflow_runs(window, cx);
                })),
        )
}
```

> **Icon names:** Before using `IconName::Ai` and `IconName::Play`, grep
> `crates/ui/src/icon_name.rs` to confirm they exist. Suitable fallbacks:
> `IconName::AtSign` for defs, `IconName::PlayerPlay` for runs.
> Use whatever verified names are available.

---

## Step 10: Footer — archive button

The existing footer has an archive-toggle button with:
```rust
matches!(self.view, SidebarView::Archive(..))
```
This continues to work after our changes since we only added non-Archive variants.
However, the archive button should only show when in `ThreadList` or `Archive` view
(not when in workflow views). Wrap it with:
```rust
.when(
    matches!(self.view, SidebarView::ThreadList | SidebarView::Archive(_)),
    |this| this.child(archive_toggle_button),
)
```

---

## Dependency setup

In `crates/sidebar/Cargo.toml`, add:
```toml
workflow_ui = { path = "../workflow_ui" }
```

Add to import at top of `sidebar.rs`:
```rust
use workflow_ui::{WorkflowClient, WorkflowDefsView, WorkflowRunsView};
```

---

## Testing checklist

- Clicking Workflow Defs tab shows `WorkflowDefsView`, highlights tab in accent color
- Clicking Workflow Runs tab shows `WorkflowRunsView`
- Clicking Threads tab returns to thread list (search filter reappears)
- Switching from Threads → Defs → Threads preserves thread list scroll position
- Switching from Defs → Runs → Defs preserves workflow list state (no re-fetch)
- `toggle_archive` action works from all view modes
- Archive button in footer hidden when in workflow views
- Tab icons show correctly; icon names verified to exist in `IconName`
- No compile errors from non-exhaustive match — all `SidebarView` match arms updated
- `workflow_defs_view` entity created only once (lazy init)
