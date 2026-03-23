# Workflow Runs — List, Picker, Run Creation, and Conversation View

**Date:** 2026-03-23
**Workstream:** 4 of 5 — Runs sidebar, picker modal, run canvas, conversation markdown
**New crate:** `crates/workflow_ui` (runs module)
**Depends on:** Workstream 2 (WorkflowCanvas run mode), Workstream 5 (HTTP client)

---

## Goal

Implement the full workflow runs experience:
1. **Runs list** in the sidebar — shows active + past runs grouped by status
2. **Workflow picker** modal — selects a workflow when creating a new run (styled like the branch picker)
3. **Run creation form** — collects `title`, `source_repo`, `task_description` before starting
4. **Run canvas** — opens `WorkflowCanvas` in run mode for the selected run
5. **Conversation view** — clicking a node opens a markdown buffer with its conversation output

---

## `WorkflowRunsView` (sidebar panel content)

```rust
pub struct WorkflowRunsView {
    runs: Vec<TaskStatusResponse>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _fetch_task: Option<Task<()>>,
    _poll_task: Option<Task<()>>,
}
```

### Render layout

```
┌────────────────────────────┐
│ ＋ New Run          [↺]    │  ← new run button + refresh
├────────────────────────────┤
│ RUNNING                    │
│ • Deploy Pipeline    1m ago│
│ • Feature Workflow  30s ago│
├────────────────────────────┤
│ COMPLETED                  │
│ • Build & Test      2h ago │
│ • Code Review       1d ago │
├────────────────────────────┤
│ FAILED                     │
│ • Lint Check        3h ago │
└────────────────────────────┘
```

Each row shows:
- Status indicator dot (color-coded: blue=running, green=completed, red=failed, gray=queued)
- Run title (from `task.title`)
- Workflow name (from `task_status.workflow.name`)
- Elapsed time
- On hover: "Discard" (×) button for completed/failed runs → `DELETE /tasks/{id}` with confirmation

### Loading

Fetches `GET /tasks` on creation, then polls every 5s (or subscribes to updates if any run is `running`).

---

## Workflow Picker modal

Styled identically to the branch picker (`crates/git_ui/src/branch_picker.rs`). Opens via `workspace.toggle_modal(...)`.

```rust
pub struct WorkflowPicker {
    picker: Entity<Picker<WorkflowPickerDelegate>>,
}

pub struct WorkflowPickerDelegate {
    workflows: Vec<WorkflowDefinitionRecord>,
    matches: Vec<WorkflowDefinitionRecord>,
    selected_index: usize,
    client: Arc<WorkflowClient>,
    on_selected: Box<dyn Fn(WorkflowDefinitionRecord, &mut Window, &mut App)>,
}
```

```rust
impl PickerDelegate for WorkflowPickerDelegate {
    type ListItem = ListItem;

    fn placeholder_text(&self, _cx: &App) -> Arc<str> {
        "Select a workflow…".into()
    }

    fn match_count(&self) -> usize { self.matches.len() }
    fn selected_index(&self) -> usize { self.selected_index }

    fn update_matches(&mut self, query: String, cx: &mut Context<Picker<Self>>) -> Task<()> {
        // Filter workflows by name with fuzzy match
        let query = query.to_lowercase();
        self.matches = self.workflows.iter()
            .filter(|w| w.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        Task::ready(())
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if let Some(workflow) = self.matches.get(self.selected_index).cloned() {
            (self.on_selected)(workflow, window, cx);
            cx.emit(DismissEvent);
        }
    }

    fn render_match(&self, ix: usize, selected: bool, _window: &Window, cx: &mut App)
        -> Option<Self::ListItem>
    {
        let workflow = self.matches.get(ix)?;
        Some(ListItem::new(ix)
            .selected(selected)
            .child(Label::new(workflow.name.clone())))
    }
}
```

### Opening the picker

```rust
pub fn open_workflow_picker(
    workspace: &mut Workspace,
    client: Arc<WorkflowClient>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.toggle_modal(window, cx, |window, cx| {
        WorkflowPicker::new(client, window, cx)
    });
}
```

---

## Run creation form

After the user selects a workflow from the picker, the modal transitions to a creation form (same modal, different content):

```
┌──────────────────────────────┐
│ New run: "My Workflow"        │
│                              │
│ Title:                       │
│ [______________________]     │
│                              │
│ Source repo:                 │
│ [/path/to/repo_____________] │  ← pre-filled with workspace root
│                              │
│ Task description:            │
│ [______________________]     │
│ [______________________]     │
│                              │
│            [Cancel] [Start →]│
└──────────────────────────────┘
```

The form is a separate `RunCreationModal` struct (or a second state in `WorkflowPicker`).

```rust
pub struct RunCreationModal {
    workflow: WorkflowDefinitionRecord,
    title_editor: Entity<Editor>,
    source_repo_editor: Entity<Editor>,
    description_editor: Entity<Editor>,
    creating: bool,
    client: Arc<WorkflowClient>,
    workspace: WeakEntity<Workspace>,
}
```

On "Start":
1. Validate that `title` and `source_repo` are non-empty
2. Call `client.run_workflow(workflow_id, RunRequest { title, source_repo, task_description }).await`
3. On success: dismiss modal, open `WorkflowCanvas` in run mode for the new task
4. On error: show inline error message, do not dismiss

---

## Run canvas

Opened via:
```rust
pub fn open_run(
    run: TaskStatusResponse,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let canvas = cx.new(|cx| WorkflowCanvas::new_run(run, window, cx));
    workspace.add_item_to_center(Box::new(canvas), window, cx);
}
```

`WorkflowCanvas::new_run` auto-layouts nodes from `run.nodes`, starts polling, and fires `on_node_activated` when a node is clicked.

---

## Conversation view (node output as markdown)

When a node is clicked in run mode, open a virtual buffer containing the node's conversation:

```rust
fn open_node_conversation(
    node: &TaskNodeStatus,
    run_title: &str,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let content = format!(
        "# {} — {}\n\n**Status:** {}\n\n---\n\n{}",
        run_title,
        node.label,
        node.status,
        node.output.as_deref().unwrap_or("*(No output yet)*"),
    );

    let buffer = cx.new(|cx| {
        Buffer::local(content, cx)
    });
    let buffer_model = cx.new(|cx| {
        project::Buffer::from_existing(buffer, cx)
    });
    workspace.open_path(
        // Open as a read-only markdown buffer
        ...
    );
}
```

Simpler approach: create a `project::Buffer` with the markdown content and open it via `workspace.open_buffer_in_new_tab(buffer, window, cx)`. Set the language to Markdown so syntax highlighting applies.

The buffer title is: `"{run_title} / {node_label} — conversation"`.

The content format:
```markdown
# Run: Deploy Pipeline
## Node: Ingest task
**Status:** completed

---

{node.output content verbatim — already in markdown format}
```

Since `output` is already markdown (from the Codex agent), it renders as-is.

---

## Discard run confirmation

Before calling `DELETE /tasks/{id}`, show a brief confirmation prompt:
```rust
window.push_notification(
    Notification::new("Discard this run? This cannot be undone.")
        .primary_action("Discard", move |window, cx| {
            // call DELETE
        })
        .dismiss_action("Cancel"),
    cx,
)
```

Use the existing notification/banner system in `crates/workspace`.

---

## Run time formatting

Use a helper:
```rust
fn format_elapsed(created_at_ms: u64) -> SharedString {
    let elapsed = now_ms() - created_at_ms;
    match elapsed {
        0..60_000 => format!("{}s ago", elapsed / 1000).into(),
        60_000..3_600_000 => format!("{}m ago", elapsed / 60_000).into(),
        _ => format!("{}h ago", elapsed / 3_600_000).into(),
    }
}
```

---

## Actions

```rust
gpui::actions!(workflow_ui, [
    NewWorkflowRun,
    DiscardRun,
    OpenRunConversation,
]);
```

---

## Testing checklist

- Runs list shows groups (Running / Completed / Failed) with correct status dots
- "New Run" opens workflow picker modal
- Picker filters workflows by name as user types
- Selecting workflow transitions to run creation form
- Pre-filled source_repo matches workspace root
- Validation blocks submit if title or source_repo empty
- Successful run creation opens run canvas and starts polling
- Run canvas shows node status badges (queued/running/completed/failed)
- Clicking node in run canvas opens markdown buffer with output
- Buffer title matches "{run} / {node} — conversation" format
- Discard run shows confirmation before deleting
- Discard only available on completed/failed runs
- Running runs auto-refresh every 2s; stops on terminal status
