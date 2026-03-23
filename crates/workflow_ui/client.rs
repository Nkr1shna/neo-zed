use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            WorkflowNodeKind::Task => "Task",
            WorkflowNodeKind::Validation => "Validation",
            WorkflowNodeKind::Review => "Review",
            WorkflowNodeKind::Integration => "Integration",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycleStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl TaskLifecycleStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskLifecycleStatus::Completed | TaskLifecycleStatus::Failed)
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            TaskLifecycleStatus::Queued => "Queued",
            TaskLifecycleStatus::Running => "Running",
            TaskLifecycleStatus::Completed => "Completed",
            TaskLifecycleStatus::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: WorkflowNodeKind,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryBehavior {
    pub max_attempts: u32,
    pub backoff_ms: u64,
}

impl Default for RetryBehavior {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodePolicy {
    pub node_id: String,
    pub required_reviews: u16,
    pub required_checks: Vec<String>,
    pub retry_behavior: RetryBehavior,
    pub validation_policy_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinitionRequest {
    pub name: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub node_policies: Vec<NodePolicy>,
    pub retry_behavior: RetryBehavior,
    pub validation_policy_ref: Option<String>,
    pub trigger_metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        self.node_policies.iter().find(|policy| policy.node_id == node_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRunRequest {
    pub title: String,
    pub source_repo: String,
    pub task_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: Uuid,
    pub title: String,
    pub source_repo: String,
    pub status: TaskLifecycleStatus,
    pub workflow_id: Option<Uuid>,
    pub task_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNodeStatus {
    pub id: String,
    pub kind: WorkflowNodeKind,
    pub label: String,
    pub status: TaskLifecycleStatus,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusResponse {
    pub task: TaskRecord,
    pub workflow: Option<WorkflowDefinitionRecord>,
    pub nodes: Vec<TaskNodeStatus>,
    pub outcome: Option<serde_json::Value>,
    pub agent: Option<serde_json::Value>,
    pub lease: Option<serde_json::Value>,
    pub validation: Option<serde_json::Value>,
    pub integration: Option<serde_json::Value>,
    pub agents: Option<Vec<serde_json::Value>>,
}

pub struct WorkflowClient {
    base_url: String,
    http: reqwest::Client,
}

impl WorkflowClient {
    pub fn new() -> Arc<Self> {
        Self::with_base_url("http://localhost:3000".to_string())
    }

    pub fn with_base_url(base_url: String) -> Arc<Self> {
        Arc::new(Self {
            base_url,
            http: reqwest::Client::new(),
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("reading GET {url} response body failed"))?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("GET {url} failed with status {status}: {body}"));
        }

        serde_json::from_slice(&bytes)
            .with_context(|| format!("deserializing GET {url} response failed"))
    }

    async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        let body_bytes = serde_json::to_vec(body).context("serializing request body failed")?;
        let response = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .body(body_bytes)
            .send()
            .await
            .with_context(|| format!("POST {url} failed"))?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("reading POST {url} response body failed"))?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("POST {url} failed with status {status}: {body}"));
        }

        serde_json::from_slice(&bytes)
            .with_context(|| format!("deserializing POST {url} response failed"))
    }

    async fn put_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        let body_bytes = serde_json::to_vec(body).context("serializing request body failed")?;
        let response = self
            .http
            .put(url)
            .header("Content-Type", "application/json")
            .body(body_bytes)
            .send()
            .await
            .with_context(|| format!("PUT {url} failed"))?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("reading PUT {url} response body failed"))?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("PUT {url} failed with status {status}: {body}"));
        }

        serde_json::from_slice(&bytes)
            .with_context(|| format!("deserializing PUT {url} response failed"))
    }

    async fn delete(&self, url: &str) -> Result<()> {
        let response = self
            .http
            .delete(url)
            .send()
            .await
            .with_context(|| format!("DELETE {url} failed"))?;

        let status = response.status();
        if !status.is_success() {
            let bytes = response
                .bytes()
                .await
                .with_context(|| format!("reading DELETE {url} response body failed"))?;
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("DELETE {url} failed with status {status}: {body}"));
        }

        Ok(())
    }

    pub async fn list_workflows(&self) -> Result<Vec<WorkflowDefinitionRecord>> {
        self.get_json(&format!("{}/workflows", self.base_url)).await
    }

    pub async fn get_workflow(&self, id: Uuid) -> Result<WorkflowDefinitionRecord> {
        self.get_json(&format!("{}/workflows/{id}", self.base_url)).await
    }

    pub async fn create_workflow(
        &self,
        request: &WorkflowDefinitionRequest,
    ) -> Result<WorkflowDefinitionRecord> {
        self.post_json(&format!("{}/workflows", self.base_url), request).await
    }

    pub async fn update_workflow(
        &self,
        id: Uuid,
        request: &WorkflowDefinitionRequest,
    ) -> Result<WorkflowDefinitionRecord> {
        self.put_json(&format!("{}/workflows/{id}", self.base_url), request).await
    }

    pub async fn run_workflow(&self, id: Uuid, request: &WorkflowRunRequest) -> Result<TaskRecord> {
        self.post_json(&format!("{}/workflows/{id}/run", self.base_url), request).await
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskRecord>> {
        self.get_json(&format!("{}/tasks", self.base_url)).await
    }

    pub async fn get_task_status(&self, id: Uuid) -> Result<TaskStatusResponse> {
        self.get_json(&format!("{}/tasks/{id}", self.base_url)).await
    }

    pub async fn delete_task(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("{}/tasks/{id}", self.base_url)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_lifecycle_status_is_terminal() {
        assert!(!TaskLifecycleStatus::Queued.is_terminal());
        assert!(!TaskLifecycleStatus::Running.is_terminal());
        assert!(TaskLifecycleStatus::Completed.is_terminal());
        assert!(TaskLifecycleStatus::Failed.is_terminal());
    }

    #[test]
    fn test_task_lifecycle_status_display_name() {
        assert_eq!(TaskLifecycleStatus::Queued.display_name(), "Queued");
        assert_eq!(TaskLifecycleStatus::Running.display_name(), "Running");
        assert_eq!(TaskLifecycleStatus::Completed.display_name(), "Completed");
        assert_eq!(TaskLifecycleStatus::Failed.display_name(), "Failed");
    }

    #[test]
    fn test_workflow_node_kind_display_name() {
        assert_eq!(WorkflowNodeKind::Task.display_name(), "Task");
        assert_eq!(WorkflowNodeKind::Validation.display_name(), "Validation");
        assert_eq!(WorkflowNodeKind::Review.display_name(), "Review");
        assert_eq!(WorkflowNodeKind::Integration.display_name(), "Integration");
    }

    #[test]
    fn test_workflow_node_kind_all() {
        let all = WorkflowNodeKind::all();
        assert_eq!(all.len(), 4);
        assert!(all.contains(&WorkflowNodeKind::Task));
        assert!(all.contains(&WorkflowNodeKind::Validation));
        assert!(all.contains(&WorkflowNodeKind::Review));
        assert!(all.contains(&WorkflowNodeKind::Integration));
    }

    #[test]
    fn test_retry_behavior_default() {
        let default = RetryBehavior::default();
        assert_eq!(default.max_attempts, 3);
        assert_eq!(default.backoff_ms, 1000);
    }

    #[test]
    fn test_workflow_definition_record_policy_for() {
        let record = WorkflowDefinitionRecord {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            node_policies: vec![NodePolicy {
                node_id: "node-1".to_string(),
                required_reviews: 1,
                required_checks: vec![],
                retry_behavior: RetryBehavior::default(),
                validation_policy_ref: None,
            }],
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: BTreeMap::new(),
        };

        assert!(record.policy_for("node-1").is_some());
        assert!(record.policy_for("node-2").is_none());
    }

    #[test]
    fn test_workflow_definition_record_to_request() {
        let record = WorkflowDefinitionRecord {
            id: Uuid::new_v4(),
            name: "my-workflow".to_string(),
            nodes: vec![WorkflowNode {
                id: "n1".to_string(),
                kind: WorkflowNodeKind::Task,
                label: "Build".to_string(),
            }],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: Some("policy-ref".to_string()),
            trigger_metadata: BTreeMap::from([("key".to_string(), "value".to_string())]),
        };

        let request = record.to_request();
        assert_eq!(request.name, "my-workflow");
        assert_eq!(request.nodes.len(), 1);
        assert_eq!(request.validation_policy_ref.as_deref(), Some("policy-ref"));
        assert_eq!(
            request.trigger_metadata.get("key").map(|s| s.as_str()),
            Some("value")
        );
    }
}
