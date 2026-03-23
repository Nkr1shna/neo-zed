# Workflow Node Inspector & Workflow CRUD

**Date:** 2026-03-23
**Workstream:** 3 of 5 — Node inspector panel + workflow save/publish
**New crate:** `crates/workflow_ui` (inspector module)
**Depends on:** Workstream 2 (WorkflowCanvas selection events), Workstream 5 (HTTP client)

---

## Goal

A right-side panel (`NodeInspectorPanel`) that appears when a node is selected in the `WorkflowCanvas`. Shows and allows editing of node details and policies. Also provides "Save Draft" (local) and "Publish" (API) actions for the workflow.

---

## `NodeInspectorPanel` struct

```rust
pub struct NodeInspectorPanel {
    /// The workflow currently being edited
    workflow: Option<WorkflowDefinition>,
    /// The id of the currently selected node
    selected_node_id: Option<String>,
    focus_handle: FocusHandle,
    /// Input editors for node fields
    label_editor: Entity<Editor>,
    required_reviews_editor: Entity<Editor>,
    required_checks_editor: Entity<Editor>,
    max_attempts_editor: Entity<Editor>,
    backoff_ms_editor: Entity<Editor>,
    /// Dirty flag: unsaved local changes
    is_dirty: bool,
    /// Save/publish state
    publish_state: PublishState,
    client: Arc<WorkflowClient>,
    _save_task: Option<Task<()>>,
}

#[derive(Clone, Debug)]
enum PublishState {
    Idle,
    Publishing,
    Success,
    Error(String),
}
```

---

## Panel trait implementation

`NodeInspectorPanel` implements `workspace::Panel`:

```rust
impl Panel for NodeInspectorPanel {
    fn persistent_name() -> &'static str { "Node Inspector" }
    fn panel_key() -> &'static str { "NodeInspector" }
    fn position(&self, ..) -> DockPosition { DockPosition::Right }
    fn position_is_valid(&self, pos: DockPosition) -> bool { pos == DockPosition::Right }
    fn icon(&self, ..) -> Option<IconName> { Some(IconName::Sliders) }
    fn icon_tooltip(&self, ..) -> Option<&'static str> { Some("Node Inspector") }
    fn toggle_action(&self) -> Box<dyn Action> { Box::new(ToggleNodeInspector) }
    fn activation_priority(&self) -> u32 { 200 }
}
```

The panel is registered in `workflow_ui::register(workspace, window, cx)` and opens automatically when a node is selected.

---

## Opening / updating the inspector

`WorkflowCanvas` holds a `WeakEntity<NodeInspectorPanel>` reference (or communicates via events). When a node is selected:

```rust
// In WorkflowCanvas, on node selection:
cx.emit(WorkflowCanvasEvent::NodeSelected {
    workflow: self.workflow.clone(),
    node_id: selected_node_id.clone(),
});
```

`NodeInspectorPanel` subscribes to the active `WorkflowCanvas` item and updates its fields when `NodeSelected` fires.

Alternatively, the workspace mediates: `WorkflowCanvas` fires an event; the workspace handler calls `inspector.update(cx, |inspector, cx| inspector.set_node(..., cx))`.

---

## Render layout

```
┌─────────────────────────────┐
│ Node Inspector         [×]  │  ← panel header
├─────────────────────────────┤
│ Kind:  [task ▼]             │  ← kind selector (dropdown)
│ Label: [________________]   │  ← text editor
├─────────────────────────────┤
│ POLICIES                    │
│ Required reviews: [__]      │
│ Required checks:            │
│ [______________________]    │  ← comma-separated
│ Retry max attempts: [__]    │
│ Retry backoff (ms): [_____] │
├─────────────────────────────┤
│ [Save Draft]  [Publish →]   │  ← action buttons
│                             │
│ ✓ Published                 │  ← publish status
└─────────────────────────────┘
```

When no node is selected:
```
┌─────────────────────────────┐
│ Node Inspector         [×]  │
├─────────────────────────────┤
│  Select a node to edit      │  ← muted placeholder
└─────────────────────────────┘
```

---

## Kind selector

A custom inline picker (not a modal) showing the four node kinds as radio buttons or a simple `div` list with click handlers. Current kind highlighted.

---

## Field binding

When the user edits the label editor:
```rust
cx.subscribe(&self.label_editor, |this, _, event, cx| {
    if let EditorEvent::BufferEdited = event {
        this.sync_label_to_workflow(cx);
        this.is_dirty = true;
        cx.notify();
    }
})
```

`sync_label_to_workflow` updates the `WorkflowDefinition` in the canvas via a shared `Arc<RwLock<WorkflowDefinition>>` or by firing a `WorkflowCanvasEvent::NodeLabelChanged`.

---

## Save Draft

"Save Draft" stores the current `WorkflowDefinition` to a JSON file:
```
~/.config/zed/workflow-drafts/{workflow_id}.json
```
Uses `cx.background_spawn` for the file write. No API call. Clears `is_dirty`.

---

## Publish

"Publish" button:
1. Sets `publish_state = Publishing`, `cx.notify()`
2. Calls `client.update_workflow(workflow_id, &workflow_request).await`
   - For new workflows (no id yet): `client.create_workflow(&workflow_request).await` → stores returned `id`
3. On success: `publish_state = Success`, show "✓ Published" for 3s then revert to `Idle`
4. On error: `publish_state = Error(message)`, show error inline

```rust
fn publish(&mut self, _: &PublishWorkflow, window: &mut Window, cx: &mut Context<Self>) {
    let Some(workflow) = self.workflow.clone() else { return };
    let client = self.client.clone();
    self.publish_state = PublishState::Publishing;
    cx.notify();
    self._save_task = Some(cx.spawn(async move |this, mut cx| {
        let result = if workflow.id.is_nil() {
            client.create_workflow(&workflow.into()).await
                .map(|r| r.id)
        } else {
            client.update_workflow(workflow.id, &workflow.into()).await
                .map(|_| workflow.id)
        };
        this.update(&mut cx, |inspector, cx| {
            match result {
                Ok(id) => {
                    inspector.publish_state = PublishState::Success;
                    cx.notify();
                    // Clear success status after 3s
                    let delay = cx.background_executor().timer(Duration::from_secs(3));
                    cx.spawn(async move |this, mut cx| {
                        delay.await;
                        this.update(&mut cx, |i, cx| {
                            i.publish_state = PublishState::Idle;
                            cx.notify();
                        }).ok();
                    }).detach();
                }
                Err(e) => {
                    inspector.publish_state = PublishState::Error(e.to_string());
                    cx.notify();
                }
            }
        }).ok();
    }));
}
```

---

## Workflow Definitions list view (`WorkflowDefsView`)

Rendered inside the sidebar when in `SidebarView::WorkflowDefs` mode.

```rust
pub struct WorkflowDefsView {
    workflows: Vec<WorkflowDefinitionRecord>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _fetch_task: Option<Task<()>>,
}
```

Renders:
- "＋ New Workflow" button at top → creates a blank `WorkflowDefinition`, opens canvas
- List of workflow names (with last-modified date)
- Click → opens `WorkflowCanvas` in edit mode for that workflow
- Refresh button in header

Fetches on creation: `GET /workflows`.

---

## Actions

```rust
gpui::actions!(workflow_ui, [
    ToggleNodeInspector,
    PublishWorkflow,
    SaveWorkflowDraft,
    NewWorkflow,
]);
```

---

## Testing checklist

- Selecting a node in the canvas populates inspector fields
- Editing label in inspector updates node label in canvas in real time
- Changing kind updates node color in canvas
- Save Draft writes file, clears dirty flag
- Publish calls PUT /workflows/{id} with correct body
- Publish error shows inline message
- Workflow list loads on first show, displays names + dates
- New Workflow opens blank canvas with empty node list
- Inspector shows placeholder when no node is selected
