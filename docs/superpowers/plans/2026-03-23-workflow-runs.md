# Workflow Runs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the workflow runs sidebar list, workflow picker modal (styled like BranchList), run creation form, run canvas opening, conversation markdown view, and run deletion.

**Architecture:** `WorkflowRunsView` renders in the sidebar. `WorkflowPicker` uses `Picker<PickerDelegate>` pattern. `RunCreationModal` is a second-stage modal after workflow selection. Conversation output opens as a read-only markdown buffer in the center pane.

**Tech Stack:** Rust, GPUI, workspace::Picker, editor::Editor, markdown/language for buffer creation

**Spec:** `docs/superpowers/specs/2026-03-23-workflow-runs.md`

**Prerequisites:** Workstream 5 (HTTP client), Workstream 2 (canvas `open_run`)

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Modify | `crates/workflow_ui/runs.rs` | `WorkflowRunsView`, run list rendering, polling, deletion |
| Modify | `crates/workflow_ui/picker.rs` | `WorkflowPickerDelegate`, `WorkflowPicker`, `RunCreationModal` |
| Modify | `crates/workflow_ui/workflow_ui.rs` | Export runs/picker types |

---

### Task 1: `WorkflowRunsView` — run list with status groups

**Files:**
- Modify: `crates/workflow_ui/runs.rs`

- [ ] **Step 1: Implement `WorkflowRunsView`**

```rust
use crate::client::{TaskLifecycleStatus, TaskRecord, TaskStatusResponse, WorkflowClient};
use gpui::{App, Context, Task};
use std::sync::Arc;
use ui::prelude::*;

pub struct WorkflowRunsView {
    runs: Vec<TaskRecord>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _fetch_task: Option<Task<()>>,
    _poll_task: Option<Task<()>>,
}

impl WorkflowRunsView {
    pub fn new(client: Arc<WorkflowClient>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            runs: vec![],
            loading: true,
            error: None,
            client,
            _fetch_task: None,
            _poll_task: None,
        };
        view.fetch(cx);
        view.start_polling(cx);
        view
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = client.list_tasks().await;
            this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(runs) => view.runs = runs,
                    Err(e) => view.error = Some(e.to_string()),
                }
                cx.notify();
            }).ok();
        }));
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(5))
                    .await;
                let Ok(runs) = client.list_tasks().await else { continue };
                this.update(cx, |view, cx| {
                    view.runs = runs;
                    cx.notify();
                }).ok();
            }
        }));
    }

    fn discard_run(&mut self, task_id: uuid::Uuid, cx: &mut Context<Self>) {
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            if let Err(e) = client.delete_task(task_id).await {
                log::error!("workflow_ui: failed to delete run {task_id}: {e}");
            }
            // Refresh list after deletion
            this.update(cx, |view, cx| { view.fetch(cx); }).ok();
        }).detach();
    }

    fn format_elapsed(status: &TaskLifecycleStatus) -> &'static str {
        // Simplified — ideally use task creation time from API
        match status {
            TaskLifecycleStatus::Running => "running",
            TaskLifecycleStatus::Completed => "completed",
            TaskLifecycleStatus::Failed => "failed",
            TaskLifecycleStatus::Queued => "queued",
        }
    }
}

impl Render for WorkflowRunsView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running: Vec<_> = self.runs.iter()
            .filter(|r| r.status == TaskLifecycleStatus::Running || r.status == TaskLifecycleStatus::Queued)
            .collect();
        let completed: Vec<_> = self.runs.iter()
            .filter(|r| r.status == TaskLifecycleStatus::Completed)
            .collect();
        let failed: Vec<_> = self.runs.iter()
            .filter(|r| r.status == TaskLifecycleStatus::Failed)
            .collect();

        v_flex()
            .size_full()
            .gap_px()
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .child(Label::new("Runs").size(LabelSize::Small).color(Color::Muted))
                    .child(div().flex_1())
                    .child(
                        IconButton::new("refresh-runs", IconName::ArrowCircle)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Refresh"))
                            .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
                    )
                    .child(
                        IconButton::new("new-run", IconName::Plus)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("New Run"))
                            .on_click(cx.listener(|_this, _, window, cx| {
                                window.dispatch_action(OpenWorkflowPicker.boxed_clone(), cx);
                            })),
                    ),
            )
            .when(self.loading && self.runs.is_empty(), |this| {
                this.child(div().px_3().py_2().child(Label::new("Loading…").color(Color::Muted).size(LabelSize::Small)))
            })
            .when_some(self.error.clone(), |this, err| {
                this.child(div().px_3().py_2().child(Label::new(err).color(Color::Error).size(LabelSize::Small)))
            })
            // Running section
            .when(!running.is_empty(), |this| {
                this.child(
                    div().px_2().py_1()
                        .child(Label::new("RUNNING").size(LabelSize::XSmall).color(Color::Muted))
                ).children(running.iter().map(|run| self.render_run_item(run, cx)))
            })
            // Completed section
            .when(!completed.is_empty(), |this| {
                this.child(
                    div().px_2().py_1()
                        .child(Label::new("COMPLETED").size(LabelSize::XSmall).color(Color::Muted))
                ).children(completed.iter().map(|run| self.render_run_item(run, cx)))
            })
            // Failed section
            .when(!failed.is_empty(), |this| {
                this.child(
                    div().px_2().py_1()
                        .child(Label::new("FAILED").size(LabelSize::XSmall).color(Color::Muted))
                ).children(failed.iter().map(|run| self.render_run_item(run, cx)))
            })
    }
}

impl WorkflowRunsView {
    fn render_run_item(&self, run: &TaskRecord, cx: &mut Context<Self>) -> impl IntoElement {
        let run = run.clone();
        let status_color = match run.status {
            TaskLifecycleStatus::Running  => Color::Info,
            TaskLifecycleStatus::Queued   => Color::Muted,
            TaskLifecycleStatus::Completed => Color::Success,
            TaskLifecycleStatus::Failed   => Color::Error,
        };
        let can_discard = run.status.is_terminal();
        let run_id = run.id;

        ListItem::new(ElementId::Name(run.id.to_string().into()))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .size_2()
                            .rounded_full()
                            .bg(match run.status {
                                TaskLifecycleStatus::Running   => gpui::blue(),
                                TaskLifecycleStatus::Queued    => gpui::gray(),
                                TaskLifecycleStatus::Completed => gpui::green(),
                                TaskLifecycleStatus::Failed    => gpui::red(),
                            }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(Label::new(run.title.clone()).size(LabelSize::Small))
                            .child(Label::new(run.status.display_name()).size(LabelSize::XSmall).color(Color::Muted)),
                    )
                    .when(can_discard, |this| {
                        this.child(
                            IconButton::new(ElementId::Name(format!("discard-{}", run_id).into()), IconName::Close)
                                .icon_size(IconSize::XSmall)
                                .icon_color(Color::Muted)
                                .tooltip(Tooltip::text("Discard run"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.discard_run(run_id, cx);
                                })),
                        )
                    }),
            )
            .on_click(move |_, window, cx| {
                window.dispatch_action(OpenWorkflowRun { task_id: run_id }.boxed_clone(), cx);
            })
    }
}
```

- [ ] **Step 2: Add run actions**

```rust
gpui::actions!(workflow_ui, [OpenWorkflowPicker]);

#[derive(Clone, Debug, gpui::Action, serde::Deserialize)]
pub struct OpenWorkflowRun {
    pub task_id: uuid::Uuid,
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Check `gpui::blue()`, `gpui::gray()`, `gpui::green()`, `gpui::red()` color helpers exist.
If not, use `cx.theme().colors().status_info` etc.

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/runs.rs
git commit -m "workflow_ui: Add WorkflowRunsView with status groups and polling"
```

---

### Task 2: Workflow picker modal

**Files:**
- Modify: `crates/workflow_ui/picker.rs`

- [ ] **Step 1: Implement `WorkflowPickerDelegate`**

```rust
use crate::client::{WorkflowClient, WorkflowDefinitionRecord};
use gpui::{App, Context, DismissEvent, EventEmitter, Render, Task, WeakEntity, Window};
use picker::{Picker, PickerDelegate};
use std::sync::Arc;
use ui::prelude::*;

pub struct WorkflowPickerDelegate {
    workflows: Vec<WorkflowDefinitionRecord>,
    matches: Vec<WorkflowDefinitionRecord>,
    selected_index: usize,
    on_selected: Box<dyn Fn(WorkflowDefinitionRecord, &mut Window, &mut App) + Send>,
    client: Arc<WorkflowClient>,
}

impl WorkflowPickerDelegate {
    pub fn new(
        client: Arc<WorkflowClient>,
        on_selected: impl Fn(WorkflowDefinitionRecord, &mut Window, &mut App) + Send + 'static,
    ) -> Self {
        Self {
            workflows: vec![],
            matches: vec![],
            selected_index: 0,
            on_selected: Box::new(on_selected),
            client,
        }
    }
}

impl PickerDelegate for WorkflowPickerDelegate {
    type ListItem = ListItem;

    fn placeholder_text(&self, _cx: &App) -> std::sync::Arc<str> {
        "Select a workflow…".into()
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(&mut self, ix: usize, _cx: &mut Context<Picker<Self>>) {
        self.selected_index = ix;
    }

    fn update_matches(&mut self, query: String, cx: &mut Context<Picker<Self>>) -> Task<()> {
        let query = query.to_lowercase();
        self.matches = if query.is_empty() {
            self.workflows.clone()
        } else {
            self.workflows.iter()
                .filter(|w| w.name.to_lowercase().contains(&query))
                .cloned()
                .collect()
        };
        self.selected_index = 0;
        Task::ready(())
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(workflow) = self.matches.get(self.selected_index).cloned() {
            (self.on_selected)(workflow, window, cx);
            cx.emit(DismissEvent);
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _window: &Window,
        _cx: &mut App,
    ) -> Option<Self::ListItem> {
        let workflow = self.matches.get(ix)?;
        Some(
            ListItem::new(ix)
                .selected(selected)
                .child(Label::new(workflow.name.clone())),
        )
    }
}
```

- [ ] **Step 2: Implement `WorkflowPicker` wrapper**

```rust
pub struct WorkflowPicker {
    picker: gpui::Entity<Picker<WorkflowPickerDelegate>>,
    _fetch_task: Option<Task<()>>,
}

impl WorkflowPicker {
    pub fn new(
        client: Arc<WorkflowClient>,
        on_selected: impl Fn(WorkflowDefinitionRecord, &mut Window, &mut App) + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = WorkflowPickerDelegate::new(client.clone(), on_selected);
        let picker = cx.new(|cx| Picker::uniform_list(delegate, window, cx));

        // Fetch workflows and populate picker
        let fetch_task = {
            let picker = picker.clone();
            cx.spawn(async move |_this, cx| {
                let Ok(workflows) = client.list_workflows().await else { return };
                picker.update(cx, |picker, cx| {
                    picker.delegate_mut().workflows = workflows.clone();
                    picker.delegate_mut().matches = workflows;
                    cx.notify();
                }).ok();
            })
        };

        Self { picker, _fetch_task: Some(fetch_task) }
    }
}

impl EventEmitter<DismissEvent> for WorkflowPicker {}

impl Focusable for WorkflowPicker {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl Render for WorkflowPicker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().w(gpui::px(480.0)).child(self.picker.clone())
    }
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Check `Picker::uniform_list` signature in `crates/picker/src/picker.rs`:
```bash
grep -n "pub fn uniform_list\|fn new\b" crates/picker/src/picker.rs | head -10
```

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/picker.rs
git commit -m "workflow_ui: Add WorkflowPicker modal using Picker delegate pattern"
```

---

### Task 3: Run creation form

**Files:**
- Modify: `crates/workflow_ui/picker.rs`

- [ ] **Step 1: Implement `RunCreationModal`**

```rust
use editor::Editor;
use workspace::Workspace;

pub struct RunCreationModal {
    workflow: WorkflowDefinitionRecord,
    title_editor: gpui::Entity<Editor>,
    source_repo_editor: gpui::Entity<Editor>,
    description_editor: gpui::Entity<Editor>,
    creating: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    workspace: gpui::WeakEntity<Workspace>,
}

impl RunCreationModal {
    pub fn new(
        workflow: WorkflowDefinitionRecord,
        client: Arc<WorkflowClient>,
        workspace: gpui::WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            e.set_placeholder_text("Run title…", window, cx);
            e
        });
        let source_repo_editor = cx.new(|cx| {
            let mut e = Editor::single_line(window, cx);
            // Pre-fill with workspace root if available
            if let Some(ws) = workspace.upgrade() {
                let paths = ws.read(cx).root_paths(cx);
                if let Some(path) = paths.first() {
                    e.set_text(path.to_string_lossy(), window, cx);
                }
            }
            e.set_placeholder_text("/path/to/repo", window, cx);
            e
        });
        let description_editor = cx.new(|cx| {
            let mut e = Editor::auto_height(4, window, cx);
            e.set_placeholder_text("Task description (optional)…", window, cx);
            e
        });
        Self {
            workflow,
            title_editor,
            source_repo_editor,
            description_editor,
            creating: false,
            error: None,
            client,
            workspace,
        }
    }

    fn start_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.title_editor.read(cx).text(cx).to_string();
        let source_repo = self.source_repo_editor.read(cx).text(cx).to_string();
        let description = {
            let text = self.description_editor.read(cx).text(cx).to_string();
            if text.is_empty() { None } else { Some(text) }
        };

        if title.is_empty() || source_repo.is_empty() {
            self.error = Some("Title and source repo are required".into());
            cx.notify();
            return;
        }

        self.creating = true;
        self.error = None;
        cx.notify();

        let client = self.client.clone();
        let workflow_id = self.workflow.id;
        let workspace = self.workspace.clone();
        let req = crate::client::WorkflowRunRequest { title, source_repo, task_description: description };

        cx.spawn(async move |this, cx| {
            let result = client.run_workflow(workflow_id, &req).await;
            match result {
                Ok(task_record) => {
                    // Fetch full status to open run canvas
                    let status_result = client.get_task_status(task_record.id).await;
                    this.update(cx, |modal, cx| {
                        modal.creating = false;
                        cx.emit(DismissEvent);
                        cx.notify();
                    }).ok();
                    if let Ok(status) = status_result {
                        workspace.update(cx, |ws, cx| {
                            // Need window here — use dispatch_action as workaround
                            // Dispatch OpenWorkflowRun action which the workspace handles
                        }).ok();
                    }
                }
                Err(e) => {
                    this.update(cx, |modal, cx| {
                        modal.creating = false;
                        modal.error = Some(e.to_string());
                        cx.notify();
                    }).ok();
                }
            }
        }).detach();
    }
}

impl EventEmitter<DismissEvent> for RunCreationModal {}

impl Focusable for RunCreationModal {
    fn focus_handle(&self, cx: &App) -> gpui::FocusHandle {
        self.title_editor.focus_handle(cx)
    }
}

impl Render for RunCreationModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w(gpui::px(480.0))
            .p_4()
            .gap_3()
            .child(Label::new(format!("New run: {}", self.workflow.name)).size(LabelSize::Large))
            .child(
                v_flex().gap_1()
                    .child(Label::new("Title").size(LabelSize::Small).color(Color::Muted))
                    .child(self.title_editor.clone()),
            )
            .child(
                v_flex().gap_1()
                    .child(Label::new("Source repo").size(LabelSize::Small).color(Color::Muted))
                    .child(self.source_repo_editor.clone()),
            )
            .child(
                v_flex().gap_1()
                    .child(Label::new("Task description").size(LabelSize::Small).color(Color::Muted))
                    .child(self.description_editor.clone()),
            )
            .when_some(self.error.clone(), |this, err| {
                this.child(Label::new(err).color(Color::Error).size(LabelSize::Small))
            })
            .child(
                h_flex().gap_2().justify_end()
                    .child(
                        Button::new("cancel", "Cancel")
                            .on_click(cx.listener(|_this, _, _window, cx| {
                                cx.emit(DismissEvent);
                            })),
                    )
                    .child(
                        Button::new("start", if self.creating { "Starting…" } else { "Start →" })
                            .disabled(self.creating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_run(window, cx);
                            })),
                    ),
            )
    }
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

Check:
- `Editor::auto_height` exists: `grep -n "fn auto_height" crates/editor/src/editor.rs | head`
- `Button::new` API: `grep -n "pub fn new" crates/ui/src/components/button/button.rs | head`

- [ ] **Step 3: Commit**

```bash
git add crates/workflow_ui/picker.rs
git commit -m "workflow_ui: Add RunCreationModal with form validation"
```

---

### Task 4: Register runs actions in workspace

**Files:**
- Modify: `crates/workflow_ui/runs.rs`

- [ ] **Step 1: Add `register` function**

```rust
pub fn register(
    workspace: &mut workspace::Workspace,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    workspace.register_action(|workspace, _: &OpenWorkflowPicker, window, cx| {
        let client = crate::client::WorkflowClient::new();
        let workspace_weak = cx.entity().downgrade();
        workspace.toggle_modal(window, cx, move |window, cx| {
            crate::picker::WorkflowPicker::new(
                client.clone(),
                move |workflow, window, cx| {
                    // After selection: open run creation modal
                    let client2 = crate::client::WorkflowClient::new();
                    let ws = workspace_weak.clone();
                    window.dispatch_action(
                        OpenRunCreationForm { workflow, client: client2, workspace: ws }.boxed_clone(),
                        cx,
                    );
                },
                window,
                cx,
            )
        });
    });

    workspace.register_action(|workspace, action: &OpenWorkflowRun, window, cx| {
        let client = crate::client::WorkflowClient::new();
        let task_id = action.task_id;
        cx.spawn(async move |workspace, mut cx| {
            let Ok(status) = client.get_task_status(task_id).await else { return };
            workspace.update(&mut cx, |ws, cx| {
                crate::canvas::open_run(status, client, ws, window, cx);
            }).ok();
        }).detach();
    });
}
```

> **Note on two-stage modal:** The `OpenRunCreationForm` action carries its data inline.
> If GPUI actions don't support non-trivial data, open the `RunCreationModal` as a new
> `toggle_modal` call from within the picker's `on_selected` closure using a
> `WeakEntity<Workspace>` captured from outside.

- [ ] **Step 2: Export from `workflow_ui.rs`**

```rust
pub use runs::{OpenWorkflowPicker, OpenWorkflowRun, WorkflowRunsView};
pub use picker::{RunCreationModal, WorkflowPicker};
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/runs.rs crates/workflow_ui/workflow_ui.rs
git commit -m "workflow_ui: Register workflow run actions in workspace"
```

---

### Task 5: Conversation view — node output as markdown buffer

**Files:**
- Modify: `crates/workflow_ui/runs.rs`

- [ ] **Step 1: Add `open_node_conversation` helper**

```rust
pub fn open_node_conversation(
    node_label: &str,
    run_title: &str,
    output: Option<&str>,
    workspace: &mut workspace::Workspace,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<workspace::Workspace>,
) {
    let content = format!(
        "# {} — {}\n\n---\n\n{}",
        run_title,
        node_label,
        output.unwrap_or("*(No output yet)*"),
    );

    let project = workspace.project().clone();
    let buffer = project.update(cx, |project, cx| {
        project.create_local_buffer(&content, None, cx)
    });

    // Set language to markdown
    let markdown_language = cx
        .background_spawn(async move {
            // Language will be auto-detected from content or set explicitly
        });

    // Open as a new buffer in the center pane
    workspace.open_project_item::<editor::Editor>(
        workspace.active_pane().clone(),
        buffer,
        true,
        window,
        cx,
    );
}
```

> **Note:** The exact API for opening a buffer depends on the codebase. Look up:
> ```bash
> grep -n "open_project_item\|create_local_buffer\|open_buffer" crates/workspace/src/workspace.rs | head -20
> grep -n "create_local_buffer\|create_buffer" crates/project/src/project.rs | head -10
> ```
> Use whatever pattern `agent_ui` uses for displaying conversation content.

- [ ] **Step 2: Wire into run canvas node click**

In `canvas.rs`, the `on_node_activated` callback should be set when opening a run canvas:

```rust
// In open_run function, after creating canvas entity:
canvas.update(cx, |c, _cx| {
    let workspace = cx.entity().downgrade(); // workspace entity
    c.on_node_activated = Some(Box::new(move |node_id, window, cx| {
        // Get run data and open conversation
        workspace.update(cx, |ws, cx| {
            // find node by id in run.nodes, call open_node_conversation
        }).ok();
    }));
});
```

> The exact wiring depends on how `open_run` has access to the workspace entity.
> Check how other canvas items pass workspace context to their click handlers.

- [ ] **Step 3: Verify compile**

```bash
cargo check -p workflow_ui 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add crates/workflow_ui/runs.rs crates/workflow_ui/canvas.rs
git commit -m "workflow_ui: Add conversation view for node output as markdown buffer"
```
