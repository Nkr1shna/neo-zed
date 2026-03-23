# Workflow HTTP Client & Crate Setup

**Date:** 2026-03-23
**Workstream:** 5 of 5 — HTTP client, shared models, crate scaffolding
**New crate:** `crates/workflow_ui`
**No dependencies on other workstreams** (this is the foundation)

---

## Goal

Create the `workflow_ui` crate with:
1. All shared data models (mirroring the runtime API types)
2. A thin async HTTP client (`WorkflowClient`) for the neo-zed-runtime REST API
3. The `register` function that wires everything into the workspace

This workstream should be completed first as all other workstreams depend on the models and client.

---

## Crate setup

### File layout

```
crates/workflow_ui/
├── Cargo.toml
├── workflow_ui.rs        ← crate root (NOT src/lib.rs)
├── canvas.rs
├── inspector.rs
├── runs.rs
├── picker.rs
└── client.rs
```

The `[lib] path = "workflow_ui.rs"` in `Cargo.toml` means the crate root is at
`crates/workflow_ui/workflow_ui.rs` (not `src/`). This matches the CLAUDE.md convention.

### `crates/workflow_ui/Cargo.toml`

```toml
[package]
name = "workflow_ui"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
path = "workflow_ui.rs"

[dependencies]
gpui = { path = "../gpui" }
ui = { path = "../ui" }
workspace = { path = "../workspace" }
editor = { path = "../editor" }
picker = { path = "../picker" }
project = { path = "../project" }
theme = { path = "../theme" }
settings = { path = "../settings" }
util = { path = "../util" }
language = { path = "../language" }

# All external deps use workspace versions to avoid duplication
anyhow.workspace = true
reqwest.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
uuid = { workspace = true, features = ["v4", "serde"] }
futures.workspace = true
```

> **Note:** The workspace uses a forked `reqwest` pinned to a specific git commit
> (`zed-reqwest`). Using `reqwest.workspace = true` picks up that fork automatically.
> Never specify `version = "0.12"` inline — that would pull from crates.io and
> create a duplicate conflicting dep.

### `crates/workflow_ui/workflow_ui.rs`

```rust
mod canvas;
mod client;
mod inspector;
mod picker;
mod runs;

pub use canvas::WorkflowCanvas;
pub use client::{WorkflowClient, WorkflowNodeKind, TaskLifecycleStatus};
pub use inspector::{NodeInspectorPanel, WorkflowDefsView};
pub use runs::{WorkflowRunsView, WorkflowPicker, RunCreationModal};

use gpui::{App, Window};
use workspace::Workspace;

pub fn init(_cx: &mut App) {
    // Reserved for future global init (feature flags, settings, etc.)
    // Do NOT register a GPUI Global here — pass Arc<WorkflowClient>
    // explicitly to each view constructor instead.
}

pub fn register(workspace: &mut Workspace, window: &mut Window, cx: &mut gpui::Context<Workspace>) {
    inspector::register(workspace, window, cx);
    runs::register(workspace, window, cx);
    canvas::register(workspace, window, cx);
}
```

---

## Data models (`client.rs`)

Mirror the runtime API types exactly, including proper enums for `kind` and `status` fields.

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::BTreeMap;
use std::sync::Arc;

// ── Enums ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeKind {
    Task,
    Validation,
    Review,
    Integration,
}

impl WorkflowNodeKind {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Validation => "Validation",
            Self::Review => "Review",
            Self::Integration => "Integration",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycleStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl TaskLifecycleStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

// ── Workflow Definition ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: WorkflowNodeKind,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryBehavior {
    pub max_attempts: u32,
    pub backoff_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodePolicy {
    pub node_id: String,
    pub required_reviews: u16,
    pub required_checks: Vec<String>,
    pub retry_behavior: RetryBehavior,
    pub validation_policy_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowDefinitionRequest {
    pub name: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub node_policies: Vec<NodePolicy>,
    pub retry_behavior: RetryBehavior,
    pub validation_policy_ref: Option<String>,
    pub trigger_metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowDefinitionRecord {
    pub id: Uuid,
    pub name: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub node_policies: Vec<NodePolicy>,
    pub retry_behavior: RetryBehavior,
    pub validation_policy_ref: Option<String>,
    pub trigger_metadata: BTreeMap<String, String>,
}

// ── Task / Run ──────────────────────────────────────────────────────────────

/// Used with `POST /workflows/{id}/run`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowRunRequest {
    pub title: String,
    pub source_repo: String,
    pub task_description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TaskRecord {
    pub id: Uuid,
    pub title: String,
    pub source_repo: String,
    pub status: TaskLifecycleStatus,
    pub workflow_id: Option<Uuid>,
    pub task_description: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TaskNodeStatus {
    pub id: String,
    pub kind: WorkflowNodeKind,
    pub label: String,
    pub status: TaskLifecycleStatus,
    pub output: Option<String>,
}

/// Full status snapshot from `GET /tasks/{id}/status`
/// Fields not used by the UI are captured but ignored via `#[allow(dead_code)]`.
#[derive(Clone, Debug, Deserialize)]
pub struct TaskStatusResponse {
    pub task: TaskRecord,
    pub workflow: Option<WorkflowDefinitionRecord>,
    pub nodes: Vec<TaskNodeStatus>,
    pub outcome: Option<serde_json::Value>,   // TaskOutcome — not yet used by UI
    pub agent: Option<serde_json::Value>,      // AgentStatus — not yet used by UI
    pub lease: Option<serde_json::Value>,      // LeaseStatus — not yet used by UI
    pub validation: Option<serde_json::Value>, // RunStatusSummary — not yet used
    pub integration: Option<serde_json::Value>,// RunStatusSummary — not yet used
    pub agents: Option<Vec<serde_json::Value>>,// Vec<TaskAgentStatus> — not yet used
}
```

> **Intentional use of `serde_json::Value` for unused fields:** The runtime's
> `TaskStatusResponse` has additional fields (`outcome`, `agent`, `lease`, etc.)
> that the UI does not yet consume. Capturing them as `Value` ensures
> deserialization succeeds even if the runtime adds sub-fields, and documents
> the presence of these fields for future implementers. Replace with typed
> structs as the UI evolves.

---

## `WorkflowClient`

Pass `Arc<WorkflowClient>` explicitly to each view constructor. Do **not** register
as a GPUI `Global` — explicit passing keeps components independently testable.

```rust
const DEFAULT_BASE_URL: &str = "http://localhost:3000";

pub struct WorkflowClient {
    client: reqwest::Client,
    base_url: String,
}

impl WorkflowClient {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            client: reqwest::Client::new(),
            base_url: DEFAULT_BASE_URL.to_string(),
        })
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
        })
    }

    // ── Workflows ────────────────────────────────────────────────────────────

    pub async fn list_workflows(&self) -> anyhow::Result<Vec<WorkflowDefinitionRecord>> {
        self.client
            .get(format!("{}/workflows", self.base_url))
            .send().await
            .context("GET /workflows failed")?
            .error_for_status().context("GET /workflows error status")?
            .json().await
            .context("failed to parse workflows response")
    }

    pub async fn get_workflow(&self, id: Uuid) -> anyhow::Result<WorkflowDefinitionRecord> {
        self.client
            .get(format!("{}/workflows/{}", self.base_url, id))
            .send().await.context("GET /workflows/{id} failed")?
            .error_for_status()?
            .json().await.context("failed to parse workflow")
    }

    pub async fn create_workflow(
        &self,
        req: &WorkflowDefinitionRequest,
    ) -> anyhow::Result<WorkflowDefinitionRecord> {
        self.client
            .post(format!("{}/workflows", self.base_url))
            .json(req)
            .send().await.context("POST /workflows failed")?
            .error_for_status()?
            .json().await.context("failed to parse create response")
    }

    pub async fn update_workflow(
        &self,
        id: Uuid,
        req: &WorkflowDefinitionRequest,
    ) -> anyhow::Result<WorkflowDefinitionRecord> {
        self.client
            .put(format!("{}/workflows/{}", self.base_url, id))
            .json(req)
            .send().await.context("PUT /workflows/{id} failed")?
            .error_for_status()?
            .json().await.context("failed to parse update response")
    }

    pub async fn run_workflow(
        &self,
        workflow_id: Uuid,
        req: &WorkflowRunRequest,
    ) -> anyhow::Result<TaskRecord> {
        self.client
            .post(format!("{}/workflows/{}/run", self.base_url, workflow_id))
            .json(req)
            .send().await.context("POST /workflows/{id}/run failed")?
            .error_for_status()?
            .json().await.context("failed to parse run response")
    }

    // ── Tasks ────────────────────────────────────────────────────────────────

    pub async fn list_tasks(&self) -> anyhow::Result<Vec<TaskRecord>> {
        self.client
            .get(format!("{}/tasks", self.base_url))
            .send().await.context("GET /tasks failed")?
            .error_for_status()?
            .json().await.context("failed to parse tasks")
    }

    pub async fn get_task_status(&self, task_id: Uuid) -> anyhow::Result<TaskStatusResponse> {
        self.client
            .get(format!("{}/tasks/{}/status", self.base_url, task_id))
            .send().await.context("GET /tasks/{id}/status failed")?
            .error_for_status()?
            .json().await.context("failed to parse task status")
    }

    pub async fn delete_task(&self, task_id: Uuid) -> anyhow::Result<()> {
        self.client
            .delete(format!("{}/tasks/{}", self.base_url, task_id))
            .send().await.context("DELETE /tasks/{id} failed")?
            .error_for_status()?;
        Ok(())
    }
}
```

---

## Integration with main app

In `crates/zed/src/main.rs` (or wherever workspace panels are registered), add:

```rust
use workflow_ui;

// During app init:
workflow_ui::init(cx);

// When registering workspace panels:
workspace.update(cx, |workspace, cx| {
    workflow_ui::register(workspace, window, cx);
});
```

Also add `workflow_ui` to the `[dependencies]` of `crates/zed/Cargo.toml`:
```toml
workflow_ui = { path = "../workflow_ui" }
```

And add `workflow_ui` to the workspace members list in the root `Cargo.toml`.

---

## Error surfacing

Use `workspace.show_error(err, cx)` directly at call sites. The signature is:
```rust
fn show_error<E: Debug + Display>(&mut self, err: &E, cx: &mut Context<Self>)
```
`anyhow::Error` satisfies both bounds. No helper wrapper needed.

---

## Testing checklist

- `list_workflows()` deserializes a valid response including `WorkflowNodeKind` enum
- `create_workflow()` serializes `WorkflowNodeKind` as snake_case in the JSON body
- `run_workflow()` sends `task_description: null` when `None` (not omitted)
- `get_task_status()` deserializes `TaskStatusResponse` with all fields present
- `TaskLifecycleStatus::is_terminal()` returns `true` for `Completed`/`Failed` only
- HTTP 4xx/5xx → `Err` via `error_for_status()`
- Connection refused → descriptive `anyhow::Error`
- `WorkflowClient::with_base_url` can point at a test server
