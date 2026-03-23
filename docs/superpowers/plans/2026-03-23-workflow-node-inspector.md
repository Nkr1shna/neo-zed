# Workflow Node Inspector & Definitions View — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `NodeInspectorPanel` (right-side panel for editing node details and publishing workflows) and `WorkflowDefsView` (list of workflows shown in the sidebar).

**Architecture:** `NodeInspectorPanel` implements the `workspace::Panel` trait (right dock position). It subscribes to `WorkflowCanvasEvent::NodeSelected` from the active canvas item. `WorkflowDefsView` is a simple GPUI view rendered inside the sidebar when the defs tab is active.

**Tech Stack:** Rust, GPUI, workspace::Panel, editor::Editor for text inputs

**Spec:** `docs/superpowers/specs/2026-03-23-workflow-node-inspector.md`

**Prerequisites:** Workstream 5 (HTTP client), Workstream 2 (canvas events)

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `crates/workflow_ui/inspector.rs` | `NodeInspectorPanel`, `WorkflowDefsView`, actions, `register` fn |
| Modify | `crates/workflow_ui/workflow_ui.rs` | Export inspector types |

---

### Task 1: `WorkflowDefsView` — workflow list for the sidebar

**Files:**
- Modify: `crates/workflow_ui/inspector.rs`

- [ ] **Step 1: Implement `WorkflowDefsView`**

```rust
use crate::client::{WorkflowClient, WorkflowDefinitionRecord};
use gpui::{App, Context, Entity, Task};
use std::sync::Arc;
use ui::prelude::*;

pub struct WorkflowDefsView {
    workflows: Vec<WorkflowDefinitionRecord>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _fetch_task: Option<Task<()>>,
}

impl WorkflowDefsView {
    pub fn new(client: Arc<WorkflowClient>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            workflows: vec![],
            loading: true,
            error: None,
            client,
            _fetch_task: None,
        };
        view.fetch(cx);
        view
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = client.list_workflows().await;
            this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(workflows) => view.workflows = workflows,
                    Err(e) => view.error = Some(e.to_string()),
                }
                cx.notify();
            }).ok();
        }));
    }
}

impl Render for WorkflowDefsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .gap_px()
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .child(Label::new("Workflows").size(LabelSize::Small).color(Color::Muted))
                    .child(div().flex_1())
                    .child(
                        IconButton::new("refresh-defs", IconName::ArrowCircle)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Refresh"))
                            .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
                    )
                    .child(
                        IconButton::new("new-workflow", IconName::Plus)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("New Workflow"))
                            .on_click(cx.listener(|_this, _, _window, _cx| {
                                // Dispatch NewWorkflow action — handled by workspace
                                // The workspace action handler will open a blank canvas
                            })),
                    ),
            )
            .when(self.loading, |this| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .child(Label::new("Loading…").color(Color::Muted).size(LabelSize::Small)),
                )
            })
            .when_some(self.error.clone(), |this, err| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .child(Label::new(err).color(Color::Error).size(LabelSize::Small)),
                )
            })
            .when(!self.loading && self.error.is_none(), |this| {
                this.children(self.workflows.iter().map(|wf| {
                    let wf = wf.clone();
                    ListItem::new(ElementId::Name(wf.id.to_string().into()))
                        .child(Label::new(wf.name.clone()))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(OpenWorkflowDef { id: wf.id }.boxed_clone(), cx);
                        })
                }))
            })
    }
}
```

- [ ] **Step 2: Add `OpenWorkflowDef` action**

```rust
use uuid::Uuid;

#[derive(Clone, Debug, gpui::Action, serde::Deserialize)]
pub struct OpenWorkflowDef {
    pub id: Uuid,
}

gpui::actions!(workflow_ui, [NewWorkflow, ToggleNodeInspector, PublishWorkflow, SaveWorkflowDraft]);
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/inspector.rs
git commit -m "workflow_ui: Add WorkflowDefsView and workflow list actions"
```

---

### Task 2: `NodeInspectorPanel` — right panel for node editing

**Files:**
- Modify: `crates/workflow_ui/inspector.rs`

- [ ] **Step 1: Implement `NodeInspectorPanel`**

```rust
use crate::client::{NodePolicy, RetryBehavior, WorkflowDefinitionRecord, WorkflowNodeKind};
use editor::Editor;
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, Task, Window};
use std::sync::Arc;
use workspace::{DockPosition, Panel, PanelEvent, Workspace};

pub struct NodeInspectorPanel {
    workflow: Option<WorkflowDefinitionRecord>,
    selected_node_id: Option<String>,
    focus_handle: FocusHandle,
    label_editor: Entity<Editor>,
    required_reviews_editor: Entity<Editor>,
    required_checks_editor: Entity<Editor>,
    max_attempts_editor: Entity<Editor>,
    backoff_ms_editor: Entity<Editor>,
    is_dirty: bool,
    publish_state: PublishState,
    client: Arc<WorkflowClient>,
    _publish_task: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Clone, Debug)]
enum PublishState {
    Idle,
    Publishing,
    Success,
    Error(String),
}

impl NodeInspectorPanel {
    pub fn new(
        client: Arc<WorkflowClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let make_editor = |cx: &mut Context<NodeInspectorPanel>| {
            cx.new(|cx| {
                let mut e = Editor::single_line(window, cx);
                e.set_placeholder_text("", window, cx);
                e
            })
        };
        Self {
            workflow: None,
            selected_node_id: None,
            focus_handle: cx.focus_handle(),
            label_editor: make_editor(cx),
            required_reviews_editor: make_editor(cx),
            required_checks_editor: make_editor(cx),
            max_attempts_editor: make_editor(cx),
            backoff_ms_editor: make_editor(cx),
            is_dirty: false,
            publish_state: PublishState::Idle,
            client,
            _publish_task: None,
            _subscriptions: vec![],
        }
    }

    pub fn set_node(
        &mut self,
        workflow: Option<WorkflowDefinitionRecord>,
        node_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workflow = workflow.clone();
        self.selected_node_id = node_id.clone();
        self.is_dirty = false;

        if let (Some(ref wf), Some(ref id)) = (&workflow, &node_id) {
            if let Some(node) = wf.nodes.iter().find(|n| &n.id == id) {
                self.label_editor.update(cx, |e, cx| e.set_text(node.label.clone(), window, cx));
            }
            if let Some(policy) = wf.policy_for(id) {
                self.required_reviews_editor.update(cx, |e, cx| {
                    e.set_text(policy.required_reviews.to_string(), window, cx);
                });
                self.required_checks_editor.update(cx, |e, cx| {
                    e.set_text(policy.required_checks.join(", "), window, cx);
                });
                self.max_attempts_editor.update(cx, |e, cx| {
                    e.set_text(policy.retry_behavior.max_attempts.to_string(), window, cx);
                });
                self.backoff_ms_editor.update(cx, |e, cx| {
                    e.set_text(policy.retry_behavior.backoff_ms.to_string(), window, cx);
                });
            }
        } else {
            // Clear all editors
            for editor in [
                &self.label_editor,
                &self.required_reviews_editor,
                &self.required_checks_editor,
                &self.max_attempts_editor,
                &self.backoff_ms_editor,
            ] {
                editor.update(cx, |e, cx| e.set_text("", window, cx));
            }
        }
        cx.notify();
    }

    fn publish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ref workflow) = self.workflow else { return };
        let client = self.client.clone();
        let workflow_id = workflow.id;
        let req = workflow.to_request();
        self.publish_state = PublishState::Publishing;
        cx.notify();

        self._publish_task = Some(cx.spawn(async move |this, cx| {
            let result = if workflow_id.is_nil() {
                client.create_workflow(&req).await.map(|r| r.id)
            } else {
                client.update_workflow(workflow_id, &req).await.map(|r| r.id)
            };
            this.update(cx, |inspector, cx| {
                match result {
                    Ok(_) => {
                        inspector.publish_state = PublishState::Success;
                        inspector.is_dirty = false;
                        cx.notify();
                        // Reset to Idle after 3s
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_secs(3))
                                .await;
                            this.update(cx, |i, cx| {
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
}

impl EventEmitter<PanelEvent> for NodeInspectorPanel {}

impl Focusable for NodeInspectorPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for NodeInspectorPanel {
    fn persistent_name() -> &'static str { "Node Inspector" }
    fn panel_key() -> &'static str { "NodeInspector" }
    fn position(&self, _: &Window, _: &App) -> DockPosition { DockPosition::Right }
    fn position_is_valid(&self, p: DockPosition) -> bool { p == DockPosition::Right }
    fn set_position(&mut self, _: DockPosition, _: &mut Window, _: &mut Context<Self>) {}
    fn icon(&self, _: &Window, _: &App) -> Option<ui::IconName> { Some(ui::IconName::Sliders) }
    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> { Some("Node Inspector") }
    fn toggle_action(&self) -> Box<dyn gpui::Action> { Box::new(ToggleNodeInspector) }
    fn activation_priority(&self) -> u32 { 200 }
    fn starts_open(&self, _: &Window, _: &App) -> bool { false }
}
```

- [ ] **Step 2: Implement `Render` for `NodeInspectorPanel`**

```rust
impl Render for NodeInspectorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_node = self.selected_node_id.is_some() && self.workflow.is_some();

        v_flex()
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(
                h_flex()
                    .h_9()
                    .px_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new("Node Inspector").size(LabelSize::Small).color(Color::Muted)),
            )
            .when(!has_node, |this| {
                this.child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Label::new("Select a node to edit").color(Color::Muted).size(LabelSize::Small)),
                )
            })
            .when(has_node, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .overflow_y_scroll()
                        .px_3()
                        .py_2()
                        .gap_3()
                        // Label field
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Label::new("Label").size(LabelSize::Small).color(Color::Muted))
                                .child(self.label_editor.clone()),
                        )
                        // Required reviews
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Label::new("Required reviews").size(LabelSize::Small).color(Color::Muted))
                                .child(self.required_reviews_editor.clone()),
                        )
                        // Required checks
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Label::new("Required checks (comma-separated)").size(LabelSize::Small).color(Color::Muted))
                                .child(self.required_checks_editor.clone()),
                        )
                        // Retry behavior
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Label::new("Retry max attempts").size(LabelSize::Small).color(Color::Muted))
                                .child(self.max_attempts_editor.clone()),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Label::new("Retry backoff (ms)").size(LabelSize::Small).color(Color::Muted))
                                .child(self.backoff_ms_editor.clone()),
                        ),
                )
                .child(
                    // Publish / status bar
                    h_flex()
                        .h_9()
                        .px_3()
                        .gap_2()
                        .border_t_1()
                        .border_color(cx.theme().colors().border)
                        .child(
                            Button::new("publish", match &self.publish_state {
                                PublishState::Publishing => "Publishing…",
                                _ => "Publish",
                            })
                            .disabled(matches!(self.publish_state, PublishState::Publishing))
                            .on_click(cx.listener(|this, _, window, cx| this.publish(window, cx))),
                        )
                        .when_some(
                            match &self.publish_state {
                                PublishState::Success => Some(("✓ Published", Color::Success)),
                                PublishState::Error(e) => Some((e.as_str(), Color::Error)),
                                _ => None,
                            },
                            |this, (msg, color)| {
                                this.child(Label::new(msg).color(color).size(LabelSize::Small))
                            },
                        ),
                )
            })
    }
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Check:
- `Button` API: look for `Button::new` in `crates/ui/src/components/button/`
- `Label` color variants: `Color::Success`, `Color::Error` exist?
- `Editor::set_text` exists: `grep -n "fn set_text" crates/editor/src/editor.rs | head -5`

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/inspector.rs
git commit -m "workflow_ui: Add NodeInspectorPanel with node editing and publish"
```

---

### Task 3: Register inspector panel and wire into workspace

**Files:**
- Modify: `crates/workflow_ui/inspector.rs`
- Modify: `crates/workflow_ui/workflow_ui.rs`
- Modify: `crates/sidebar/Cargo.toml`
- Modify: `crates/sidebar/src/sidebar.rs` (add `workflow_ui` dep for `WorkflowDefsView`)

- [ ] **Step 1: Add `register` function to `inspector.rs`**

```rust
pub fn register(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    // Register the NodeInspectorPanel
    workspace.register_panel::<NodeInspectorPanel>(window, cx);

    // Register actions
    workspace.register_action(|workspace, action: &OpenWorkflowDef, window, cx| {
        let client = WorkflowClient::new(); // TODO: share a single instance
        // Fetch the workflow and open its canvas
        let id = action.id;
        let task = cx.background_spawn({
            let client = client.clone();
            async move { client.get_workflow(id).await }
        });
        cx.spawn(async move |workspace, mut cx| {
            if let Ok(Ok(workflow)) = task.await.map(|r| r) {
                workspace.update(&mut cx, |ws, cx| {
                    crate::canvas::open_workflow(workflow, client, ws, window, cx);
                }).ok();
            }
        }).detach();
    });

    workspace.register_action(|workspace, _: &NewWorkflow, window, cx| {
        // Create a blank workflow and open its canvas
        use crate::client::{RetryBehavior, WorkflowDefinitionRecord};
        use std::collections::BTreeMap;
        let blank = WorkflowDefinitionRecord {
            id: uuid::Uuid::nil(),
            name: "New Workflow".into(),
            nodes: vec![],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: BTreeMap::new(),
        };
        let client = WorkflowClient::new();
        crate::canvas::open_workflow(blank, client, workspace, window, cx);
    });
}
```

- [ ] **Step 2: Export types from `workflow_ui.rs`**

Update `workflow_ui.rs`:
```rust
pub use inspector::{NodeInspectorPanel, OpenWorkflowDef, WorkflowDefsView};
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Check `workspace.register_panel` — find actual signature:
```bash
grep -n "fn register_panel\|register_panel" crates/workspace/src/workspace.rs | head -10
```

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/inspector.rs crates/workflow_ui/workflow_ui.rs
git commit -m "workflow_ui: Register NodeInspectorPanel and workflow actions"
```
