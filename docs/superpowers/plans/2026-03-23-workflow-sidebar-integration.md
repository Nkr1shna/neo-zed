# Workflow Sidebar Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Workflow Definitions and Workflow Runs view modes to the existing agent sidebar (`crates/sidebar/src/sidebar.rs`) via three icon-button navigation tabs.

**Architecture:** Extend the existing `SidebarView` enum with `WorkflowDefs` and `WorkflowRuns` variants. Store workflow view entities lazily in `Sidebar` struct fields (not inside the enum) for state persistence. Modify the sidebar header to add tab icons.

**Tech Stack:** Rust, GPUI, existing sidebar patterns

**Spec:** `docs/superpowers/specs/2026-03-23-workflow-sidebar-integration.md`

**Prerequisites:** All other workstreams (Workstream 5 for WorkflowClient, Workstreams 2/3/4 for view types). Implement this last.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `crates/sidebar/Cargo.toml` | Add `workflow_ui` dependency |
| Modify | `crates/sidebar/src/sidebar.rs` | All sidebar changes |

---

### Task 1: Add dependency and import

**Files:**
- Modify: `crates/sidebar/Cargo.toml`
- Modify: `crates/sidebar/src/sidebar.rs`

- [ ] **Step 1: Add `workflow_ui` to `crates/sidebar/Cargo.toml`**

Find `[dependencies]` in `crates/sidebar/Cargo.toml` and add:
```toml
workflow_ui = { path = "../workflow_ui" }
```

- [ ] **Step 2: Add imports to top of `sidebar.rs`**

After the existing `use` statements, add:
```rust
use workflow_ui::{WorkflowClient, WorkflowDefsView, WorkflowRunsView};
use std::sync::Arc;
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p sidebar 2>&1 | head -30
```

Expected: no errors (workflow_ui types are in scope).

---

### Task 2: Extend `SidebarView` enum and add new struct fields

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs`

- [ ] **Step 1: Extend `SidebarView` enum**

Find the enum at line ~67:
```rust
#[derive(Debug, Default)]
enum SidebarView {
    #[default]
    ThreadList,
    Archive(Entity<ThreadsArchiveView>),
}
```

Change to:
```rust
#[derive(Debug, Default)]
enum SidebarView {
    #[default]
    ThreadList,
    Archive(Entity<ThreadsArchiveView>),
    WorkflowDefs,
    WorkflowRuns,
}
```

> The new variants carry no associated `Entity` — entities live in `Sidebar` fields
> below, enabling state persistence across tab switches.

- [ ] **Step 2: Add fields to `Sidebar` struct**

Find the `pub struct Sidebar {` definition (~line 235). Add after existing fields:
```rust
    workflow_defs_view: Option<Entity<WorkflowDefsView>>,
    workflow_runs_view: Option<Entity<WorkflowRunsView>>,
    workflow_client: Arc<WorkflowClient>,
```

- [ ] **Step 3: Initialize new fields in `Sidebar::new`**

Find `Self {` in `Sidebar::new` (~line 334). Add:
```rust
    workflow_defs_view: None,
    workflow_runs_view: None,
    workflow_client: WorkflowClient::new(),
```

- [ ] **Step 4: Verify compile**

```bash
cargo check -p sidebar 2>&1 | head -30
```

---

### Task 3: Fix non-exhaustive `match` arms

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs`

- [ ] **Step 1: Find all match sites**

```bash
grep -n "match.*self\.view\|match.*view\b" crates/sidebar/src/sidebar.rs
```

- [ ] **Step 2: Update `toggle_archive`**

Find `fn toggle_archive` (~line 2722). Update the match:
```rust
fn toggle_archive(&mut self, _: &ToggleArchive, window: &mut Window, cx: &mut Context<Self>) {
    match &self.view {
        SidebarView::ThreadList
        | SidebarView::WorkflowDefs
        | SidebarView::WorkflowRuns => self.show_archive(window, cx),
        SidebarView::Archive(_) => self.show_thread_list(window, cx),
    }
}
```

- [ ] **Step 3: Update all other match sites**

For each remaining site, add:
```rust
SidebarView::WorkflowDefs | SidebarView::WorkflowRuns => { /* treat like ThreadList */ }
```

- [ ] **Step 4: Verify compile**

```bash
cargo check -p sidebar 2>&1 | head -30
```

Expected: no exhaustiveness errors.

- [ ] **Step 5: Commit progress**

```bash
git add crates/sidebar/
git commit -m "sidebar: Add WorkflowDefs/WorkflowRuns SidebarView variants and fields"
```

---

### Task 4: Add actions and view-switching methods

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs`

- [ ] **Step 1: Add new actions**

Find the `gpui::actions!` call (~line 52):
```rust
gpui::actions!(
    agents_sidebar,
    [
        NewThreadInGroup,
        ToggleArchive,
        ShowWorkflowDefs,   // add
        ShowWorkflowRuns,   // add
    ]
);
```

- [ ] **Step 2: Add `show_workflow_defs` and `show_workflow_runs` methods**

After the existing `show_archive`/`show_thread_list` methods, add:

```rust
impl Sidebar {
    fn show_workflow_defs_action(
        &mut self,
        _: &ShowWorkflowDefs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_workflow_defs(window, cx);
    }

    fn show_workflow_runs_action(
        &mut self,
        _: &ShowWorkflowRuns,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_workflow_runs(window, cx);
    }

    fn show_workflow_defs(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.workflow_defs_view.is_none() {
            let client = self.workflow_client.clone();
            // cx.new closure receives only &mut Context<WorkflowDefsView>
            // window is NOT a parameter — it's captured from outer scope if needed,
            // but WorkflowDefsView::new only takes (client, cx)
            let view = cx.new(|cx| WorkflowDefsView::new(client, cx));
            self.workflow_defs_view = Some(view);
        }
        self.view = SidebarView::WorkflowDefs;
        cx.notify();
    }

    fn show_workflow_runs(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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

- [ ] **Step 3: Register action handlers in the action dispatch setup**

Find where `on_action` calls are registered in `Sidebar` (look for existing `.on_action(cx.listener(Self::toggle_archive))` call in the render or init method). Add alongside:

```rust
.on_action(cx.listener(Self::show_workflow_defs_action))
.on_action(cx.listener(Self::show_workflow_runs_action))
```

- [ ] **Step 4: Verify compile**

```bash
cargo check -p sidebar 2>&1 | head -30
```

- [ ] **Step 5: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Add ShowWorkflowDefs/ShowWorkflowRuns actions and view switching"
```

---

### Task 5: Update render and header

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs`

- [ ] **Step 1: Add `render_view_tabs` method**

Verify which icon names exist before using them:
```bash
grep -n "ZedAgent\|Play\|PlayerPlay\|Ai\|AtSign\|Workflow" crates/ui/src/icon_name.rs | head -20
```

Then add:
```rust
fn render_view_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let is_threads = matches!(self.view, SidebarView::ThreadList | SidebarView::Archive(_));
    let is_defs = matches!(self.view, SidebarView::WorkflowDefs);
    let is_runs = matches!(self.view, SidebarView::WorkflowRuns);

    h_flex()
        .gap_0p5()
        .child(
            IconButton::new("tab-threads", IconName::ZedAgent)  // verify exists
                .icon_size(IconSize::Small)
                .icon_color(if is_threads { Color::Accent } else { Color::Muted })
                .tooltip(Tooltip::text("Agent Threads"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_thread_list(window, cx);
                })),
        )
        .child(
            IconButton::new("tab-workflow-defs", IconName::Ai)  // verify or replace
                .icon_size(IconSize::Small)
                .icon_color(if is_defs { Color::Accent } else { Color::Muted })
                .tooltip(Tooltip::text("Workflow Definitions"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_workflow_defs(window, cx);
                })),
        )
        .child(
            IconButton::new("tab-workflow-runs", IconName::Play)  // verify or replace
                .icon_size(IconSize::Small)
                .icon_color(if is_runs { Color::Accent } else { Color::Muted })
                .tooltip(Tooltip::text("Workflow Runs"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_workflow_runs(window, cx);
                })),
        )
}
```

- [ ] **Step 2: Add `render_workflow_tab_header` method**

```rust
fn render_workflow_tab_header(
    &self,
    window: &Window,
    cx: &mut Context<Self>,
) -> impl IntoElement {
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

- [ ] **Step 3: Update `render_sidebar_header` to include tabs**

Find `render_sidebar_header` (~line 2637). Add the tabs row inside it, always visible
(not gated by `no_open_projects`):

After `.child(self.render_sidebar_toggle_button(cx))`, add:
```rust
.child(Divider::vertical().color(ui::DividerColor::Border))
.child(self.render_view_tabs(cx))
```

- [ ] **Step 4: Extend the render `match` in `Render for Sidebar`**

Find the `match` block inside `Render for Sidebar::render` (~line 2870):
```rust
.map(|this| match &self.view {
    SidebarView::ThreadList => this...
    SidebarView::Archive(view) => this...
})
```

Add:
```rust
SidebarView::WorkflowDefs => {
    if let Some(view) = &self.workflow_defs_view {
        this.child(self.render_workflow_tab_header(window, cx))
            .child(view.clone())
    } else {
        this
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

- [ ] **Step 5: Update footer archive button visibility**

Find the footer area (~line 2913) where the archive toggle button is rendered.
Wrap it so it only shows for thread-list/archive views:

```rust
.when(
    matches!(self.view, SidebarView::ThreadList | SidebarView::Archive(_)),
    |this| this.child(/* existing archive button */),
)
```

- [ ] **Step 6: Verify compile**

```bash
cargo check -p sidebar 2>&1 | head -30
```

- [ ] **Step 7: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Add workflow tab icons and render workflow views"
```

---

### Task 6: Full build verification

- [ ] **Step 1: Run clippy on affected crates**

```bash
./script/clippy -p workflow_ui 2>&1 | head -60
./script/clippy -p sidebar 2>&1 | head -60
./script/clippy -p zed 2>&1 | head -60
```

Fix any warnings that are errors.

- [ ] **Step 2: Build the full application**

```bash
cargo build -p zed 2>&1 | tail -20
```

Expected: build succeeds.

- [ ] **Step 3: Manual smoke test**

1. Launch neo-zed
2. Verify three tab icons appear in the sidebar header
3. Click "Workflow Definitions" tab → list view appears (may show empty or error if runtime not running)
4. Click "Workflow Runs" tab → runs list appears
5. Click "Agent Threads" tab → thread list returns, search filter reappears
6. Start the runtime: `cd /Users/nest/Developer/neo-zed-runtime && cargo run`
7. Refresh workflow defs → list populates
8. Click "New Workflow" → blank canvas opens in center pane
9. Add a Task node → node appears on canvas
10. Select node → NodeInspector appears in right panel with editable fields
11. Click "Workflow Runs" → click "+ New Run" → picker opens with workflow list
12. Select workflow → creation form appears with source_repo pre-filled
13. Fill title, click "Start" → run canvas opens with nodes showing queued status
14. Wait 2s → status updates
15. Click a completed node → markdown buffer opens with conversation output

- [ ] **Step 4: Final commit**

```bash
git add -u
git commit -m "sidebar: Complete workflow sidebar integration smoke-tested"
```
