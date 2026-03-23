# Workflow HTTP Client & Crate Setup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `workflow_ui` crate with shared data models and an async HTTP client for the neo-zed-runtime REST API at `http://localhost:3000`.

**Architecture:** A new crate `crates/workflow_ui/` with its root at `workflow_ui.rs`. All other workstreams depend on this crate. The HTTP client (`WorkflowClient`) is constructed once and passed explicitly via `Arc<WorkflowClient>` to each view. No GPUI Global used.

**Tech Stack:** Rust, GPUI, reqwest (workspace fork), serde/serde_json (workspace versions), uuid

**Spec:** `docs/superpowers/specs/2026-03-23-workflow-http-client.md`

---

## File Map

| Action | Path | Responsibility |
|--------|------|---------------|
| Create | `crates/workflow_ui/Cargo.toml` | Crate manifest |
| Create | `crates/workflow_ui/workflow_ui.rs` | Crate root, re-exports, `register`/`init` |
| Create | `crates/workflow_ui/client.rs` | `WorkflowClient`, all data models |
| Create | `crates/workflow_ui/canvas.rs` | Stub (empty mod, filled by Workstream 2) |
| Create | `crates/workflow_ui/inspector.rs` | Stub (empty mod, filled by Workstream 3) |
| Create | `crates/workflow_ui/runs.rs` | Stub (empty mod, filled by Workstream 4) |
| Create | `crates/workflow_ui/picker.rs` | Stub (empty mod, filled by Workstream 4) |
| Modify | `Cargo.toml` (workspace root) | Add `workflow_ui` to `[workspace.members]` |
| Modify | `crates/zed/Cargo.toml` | Add `workflow_ui` dependency |
| Modify | `crates/zed/src/main.rs` | Call `workflow_ui::init(cx)` at startup |

---

### Task 1: Create the crate skeleton

**Files:**
- Create: `crates/workflow_ui/Cargo.toml`
- Create: `crates/workflow_ui/workflow_ui.rs`
- Create: `crates/workflow_ui/client.rs` (data models only, no HTTP yet)
- Create: `crates/workflow_ui/canvas.rs`
- Create: `crates/workflow_ui/inspector.rs`
- Create: `crates/workflow_ui/runs.rs`
- Create: `crates/workflow_ui/picker.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Create `crates/workflow_ui/Cargo.toml`**

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
paths = { path = "../paths" }
log = { workspace = true }

anyhow.workspace = true
reqwest.workspace = true
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
uuid = { workspace = true, features = ["v4", "serde"] }
futures.workspace = true
```

- [ ] **Step 2: Create stub module files**

Create `crates/workflow_ui/canvas.rs`:
```rust
// Workflow graph canvas — implemented in Workstream 2
```

Create `crates/workflow_ui/inspector.rs`:
```rust
// Node inspector panel — implemented in Workstream 3
```

Create `crates/workflow_ui/runs.rs`:
```rust
// Workflow runs list and run creation — implemented in Workstream 4
```

Create `crates/workflow_ui/picker.rs`:
```rust
// Workflow picker modal — implemented in Workstream 4
```

- [ ] **Step 3: Create `crates/workflow_ui/client.rs` with data models**

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Enums ────────────────────────────────────────────────────────────────────

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

    pub fn all() -> &'static [WorkflowNodeKind] {
        &[
            WorkflowNodeKind::Task,
            WorkflowNodeKind::Validation,
            WorkflowNodeKind::Review,
            WorkflowNodeKind::Integration,
        ]
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

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
        }
    }
}

impl Default for TaskLifecycleStatus {
    fn default() -> Self {
        Self::Queued
    }
}

// ── Workflow Definition ───────────────────────────────────────────────────────

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

impl Default for RetryBehavior {
    fn default() -> Self {
        Self { max_attempts: 3, backoff_ms: 1000 }
    }
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

impl WorkflowDefinitionRecord {
    pub fn to_request(&self) -> WorkflowDefinitionRequest {
        WorkflowDefinitionRequest {
            name: self.name.clone(),
            nodes: self.nodes.clone(),
            edges: self.edges.clone(),
            node_policies: self.node_policies.clone(),
            retry_behavior: self.retry_behavior.clone(),
            validation_policy_ref: self.validation_policy_ref.clone(),
            trigger_metadata: self.trigger_metadata.clone(),
        }
    }

    pub fn policy_for(&self, node_id: &str) -> Option<&NodePolicy> {
        self.node_policies.iter().find(|p| p.node_id == node_id)
    }
}

// ── Task / Run ────────────────────────────────────────────────────────────────

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

#[derive(Clone, Debug, Deserialize)]
pub struct TaskStatusResponse {
    pub task: TaskRecord,
    pub workflow: Option<WorkflowDefinitionRecord>,
    pub nodes: Vec<TaskNodeStatus>,
    // Additional runtime fields captured but not yet used by the UI
    pub outcome: Option<serde_json::Value>,
    pub agent: Option<serde_json::Value>,
    pub lease: Option<serde_json::Value>,
    pub validation: Option<serde_json::Value>,
    pub integration: Option<serde_json::Value>,
    pub agents: Option<Vec<serde_json::Value>>,
}

// ── WorkflowClient ────────────────────────────────────────────────────────────

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

    pub async fn list_workflows(&self) -> anyhow::Result<Vec<WorkflowDefinitionRecord>> {
        use anyhow::Context as _;
        self.client
            .get(format!("{}/workflows", self.base_url))
            .send().await.context("GET /workflows failed")?
            .error_for_status().context("GET /workflows error status")?
            .json().await.context("failed to parse workflows response")
    }

    pub async fn get_workflow(&self, id: Uuid) -> anyhow::Result<WorkflowDefinitionRecord> {
        use anyhow::Context as _;
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
        use anyhow::Context as _;
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
        use anyhow::Context as _;
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
        use anyhow::Context as _;
        self.client
            .post(format!("{}/workflows/{}/run", self.base_url, workflow_id))
            .json(req)
            .send().await.context("POST /workflows/{id}/run failed")?
            .error_for_status()?
            .json().await.context("failed to parse run response")
    }

    pub async fn list_tasks(&self) -> anyhow::Result<Vec<TaskRecord>> {
        use anyhow::Context as _;
        self.client
            .get(format!("{}/tasks", self.base_url))
            .send().await.context("GET /tasks failed")?
            .error_for_status()?
            .json().await.context("failed to parse tasks")
    }

    pub async fn get_task_status(&self, task_id: Uuid) -> anyhow::Result<TaskStatusResponse> {
        use anyhow::Context as _;
        self.client
            .get(format!("{}/tasks/{}/status", self.base_url, task_id))
            .send().await.context("GET /tasks/{id}/status failed")?
            .error_for_status()?
            .json().await.context("failed to parse task status")
    }

    pub async fn delete_task(&self, task_id: Uuid) -> anyhow::Result<()> {
        use anyhow::Context as _;
        self.client
            .delete(format!("{}/tasks/{}", self.base_url, task_id))
            .send().await.context("DELETE /tasks/{id} failed")?
            .error_for_status()?;
        Ok(())
    }
}
```

- [ ] **Step 4: Create `crates/workflow_ui/workflow_ui.rs` (crate root)**

```rust
mod canvas;
mod client;
mod inspector;
mod picker;
mod runs;

pub use client::{
    NodePolicy, RetryBehavior, TaskLifecycleStatus, TaskNodeStatus, TaskRecord,
    TaskStatusResponse, WorkflowClient, WorkflowDefinitionRecord, WorkflowDefinitionRequest,
    WorkflowEdge, WorkflowNode, WorkflowNodeKind, WorkflowRunRequest,
};

use gpui::App;
use workspace::Workspace;

pub fn init(_cx: &mut App) {
    // Reserved for future global initialization
}

pub fn register(
    _workspace: &mut Workspace,
    _window: &mut gpui::Window,
    _cx: &mut gpui::Context<Workspace>,
) {
    // Panel and action registration — filled in by Workstreams 2, 3, 4
}
```

- [ ] **Step 5: Add `workflow_ui` to workspace `Cargo.toml`**

In the root `Cargo.toml`, find the `[workspace]` `members = [...]` array and add:
```toml
"crates/workflow_ui",
```

- [ ] **Step 6: Verify compile**

```bash
cd /Users/nest/Developer/neo-zed
cargo check -p workflow_ui 2>&1 | head -40
```

Expected: no errors. If there are missing workspace deps, look them up in the root `Cargo.toml` `[workspace.dependencies]` table.

- [ ] **Step 7: Commit**

```bash
git add crates/workflow_ui/ Cargo.toml
git commit -m "workflow_ui: Add crate skeleton with HTTP client and data models"
```

---

### Task 2: Wire crate into main app

**Files:**
- Modify: `crates/zed/Cargo.toml`
- Modify: `crates/zed/src/main.rs` (find startup init site)

- [ ] **Step 1: Add dependency to `crates/zed/Cargo.toml`**

Find the `[dependencies]` section and add:
```toml
workflow_ui = { path = "../workflow_ui" }
```

- [ ] **Step 2: Add `workflow_ui::init` call in `main.rs`**

Search for where other UI crates call their init functions (e.g., `agent_ui::init`, `git_ui::init`):
```bash
grep -n "::init(cx)" crates/zed/src/main.rs | head -20
```

Add alongside them:
```rust
workflow_ui::init(cx);
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p zed 2>&1 | head -40
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/zed/Cargo.toml crates/zed/src/main.rs
git commit -m "zed: Wire workflow_ui crate into main app startup"
```
