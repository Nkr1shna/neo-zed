use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use futures::AsyncReadExt;
use http_client::{AsyncBody, HttpClient as _, HttpClientWithUrl, Method, Request};
use reqwest_client::ReqwestClient;
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeTypeCategory {
    Task,
    Validation,
    Review,
    Integration,
}

impl WorkflowNodeTypeCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            WorkflowNodeTypeCategory::Task => "Task",
            WorkflowNodeTypeCategory::Validation => "Validation",
            WorkflowNodeTypeCategory::Review => "Review",
            WorkflowNodeTypeCategory::Integration => "Integration",
        }
    }

    pub fn all() -> &'static [WorkflowNodeTypeCategory] {
        &[
            WorkflowNodeTypeCategory::Task,
            WorkflowNodeTypeCategory::Validation,
            WorkflowNodeTypeCategory::Review,
            WorkflowNodeTypeCategory::Integration,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodePrimitive {
    Llm,
    ExecuteShellCommand,
    Conditional,
}

impl WorkflowNodePrimitive {
    pub fn display_name(&self) -> &'static str {
        match self {
            WorkflowNodePrimitive::Llm => "LLM",
            WorkflowNodePrimitive::ExecuteShellCommand => "Execute Shell Command",
            WorkflowNodePrimitive::Conditional => "Conditional",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeFieldType {
    #[serde(alias = "text")]
    String,
    Number,
    Boolean,
    Enum,
    Workspace,
    Repo,
    Artifact,
}

impl WorkflowNodeFieldType {
    pub fn display_name(&self) -> &'static str {
        match self {
            WorkflowNodeFieldType::String => "Text",
            WorkflowNodeFieldType::Number => "Number",
            WorkflowNodeFieldType::Boolean => "Boolean",
            WorkflowNodeFieldType::Enum => "Enum",
            WorkflowNodeFieldType::Workspace => "Workspace",
            WorkflowNodeFieldType::Repo => "Repository",
            WorkflowNodeFieldType::Artifact => "Artifact",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeFieldOption {
    pub value: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowNodeField {
    #[serde(alias = "id")]
    pub key: String,
    pub label: String,
    #[serde(alias = "kind")]
    pub field_type: WorkflowNodeFieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "deserialize_workflow_node_field_options")]
    pub options: Vec<WorkflowNodeFieldOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodePort {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeType {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub primitive: Option<WorkflowNodePrimitive>,
    #[serde(default)]
    pub category: Option<WorkflowNodeTypeCategory>,
    #[serde(default)]
    pub is_primitive: bool,
    pub inputs: Vec<WorkflowNodePort>,
    pub outputs: Vec<WorkflowNodePort>,
    #[serde(default)]
    pub configure_time_fields: Vec<WorkflowNodeField>,
    #[serde(default)]
    pub runtime_fields: Vec<WorkflowNodeField>,
}

impl WorkflowNodeType {
    pub fn primitive_kind(&self) -> WorkflowNodePrimitive {
        infer_workflow_node_primitive(&self.id, self.category.as_ref(), self.primitive)
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
        matches!(
            self,
            TaskLifecycleStatus::Completed | TaskLifecycleStatus::Failed
        )
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
    #[serde(alias = "kind")]
    pub node_type: String,
    pub label: String,
    #[serde(default = "default_json_object", alias = "config", alias = "configure_time")]
    pub configuration: serde_json::Value,
    #[serde(default = "default_json_object")]
    pub runtime: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    #[serde(alias = "from")]
    pub from_node_id: String,
    #[serde(default)]
    pub from_output_id: String,
    #[serde(alias = "to")]
    pub to_node_id: String,
    #[serde(default)]
    pub to_input_id: String,
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
        self.node_policies
            .iter()
            .find(|policy| policy.node_id == node_id)
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
    pub node_type: String,
    #[serde(default)]
    pub primitive: Option<WorkflowNodePrimitive>,
    #[serde(default)]
    pub category: Option<WorkflowNodeTypeCategory>,
    pub label: String,
    pub status: TaskLifecycleStatus,
    pub output: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, serde_json::Value>,
}

impl TaskNodeStatus {
    pub fn primitive_kind(&self) -> WorkflowNodePrimitive {
        infer_workflow_node_primitive(&self.node_type, self.category.as_ref(), self.primitive)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusResponse {
    pub task: TaskRecord,
    pub workflow: Option<WorkflowDefinitionRecord>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    pub nodes: Vec<TaskNodeStatus>,
    pub outcome: Option<serde_json::Value>,
    pub agent: Option<serde_json::Value>,
    pub lease: Option<serde_json::Value>,
    pub validation: Option<serde_json::Value>,
    pub integration: Option<serde_json::Value>,
    pub failure_message: Option<String>,
    pub agents: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNodeConversationResponse {
    pub task_id: Uuid,
    pub node_id: String,
    pub session_id: String,
    #[serde(default)]
    pub workspace_path: Option<String>,
    pub markdown: String,
}

fn default_json_object() -> serde_json::Value {
    serde_json::json!({})
}

fn deserialize_workflow_node_field_options<'de, D>(
    deserializer: D,
) -> Result<Vec<WorkflowNodeFieldOption>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw_values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    raw_values
        .into_iter()
        .map(|value| match value {
            serde_json::Value::String(option) => Ok(WorkflowNodeFieldOption {
                label: option.clone(),
                value: option,
            }),
            serde_json::Value::Object(_) => serde_json::from_value(value)
                .map_err(serde::de::Error::custom),
            other => Err(serde::de::Error::custom(format!(
                "unsupported workflow node field option: {other}"
            ))),
        })
        .collect()
}

pub(crate) fn infer_workflow_node_primitive(
    node_type_id: &str,
    legacy_category: Option<&WorkflowNodeTypeCategory>,
    explicit_primitive: Option<WorkflowNodePrimitive>,
) -> WorkflowNodePrimitive {
    if let Some(explicit_primitive) = explicit_primitive {
        return explicit_primitive;
    }

    if let Some(legacy_category) = legacy_category {
        return match legacy_category {
            WorkflowNodeTypeCategory::Task | WorkflowNodeTypeCategory::Review => {
                WorkflowNodePrimitive::Llm
            }
            WorkflowNodeTypeCategory::Validation => WorkflowNodePrimitive::Conditional,
            WorkflowNodeTypeCategory::Integration => WorkflowNodePrimitive::ExecuteShellCommand,
        };
    }

    match node_type_id {
        "execute_shell_command" | "integration" => WorkflowNodePrimitive::ExecuteShellCommand,
        "conditional" | "validation" => WorkflowNodePrimitive::Conditional,
        _ => WorkflowNodePrimitive::Llm,
    }
}

pub struct WorkflowClient {
    http: Arc<HttpClientWithUrl>,
}

impl WorkflowClient {
    pub fn new() -> Arc<Self> {
        Self::with_base_url(
            std::env::var("NEO_ZED_RUNTIME_URL")
                .unwrap_or_else(|_| "http://localhost:3000".to_string()),
        )
    }

    pub fn with_base_url(base_url: String) -> Arc<Self> {
        Arc::new(Self {
            http: Arc::new(HttpClientWithUrl::new(
                Arc::new(ReqwestClient::new()),
                base_url,
                None,
            )),
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let url = self.http.build_url(path);
        let mut response = self
            .http
            .get(&url, AsyncBody::default(), false)
            .await
            .with_context(|| format!("GET {url} failed"))?;

        let status = response.status();
        let mut bytes = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut bytes)
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
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.http.build_url(path);
        let body_bytes = serde_json::to_string(body).context("serializing request body failed")?;
        let mut response = self
            .http
            .post_json(&url, AsyncBody::from(body_bytes))
            .await
            .with_context(|| format!("POST {url} failed"))?;

        let status = response.status();
        let mut bytes = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut bytes)
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
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = self.http.build_url(path);
        let body_bytes = serde_json::to_string(body).context("serializing request body failed")?;
        let request = Request::builder()
            .uri(url.as_str())
            .method(Method::PUT)
            .header("Content-Type", "application/json")
            .body(AsyncBody::from(body_bytes))
            .context("building PUT request failed")?;
        let mut response = self
            .http
            .send(request)
            .await
            .with_context(|| format!("PUT {url} failed"))?;

        let status = response.status();
        let mut bytes = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut bytes)
            .await
            .with_context(|| format!("reading PUT {url} response body failed"))?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("PUT {url} failed with status {status}: {body}"));
        }

        serde_json::from_slice(&bytes)
            .with_context(|| format!("deserializing PUT {url} response failed"))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let url = self.http.build_url(path);
        let request = Request::builder()
            .uri(url.as_str())
            .method(Method::DELETE)
            .body(AsyncBody::default())
            .context("building DELETE request failed")?;
        let mut response = self
            .http
            .send(request)
            .await
            .with_context(|| format!("DELETE {url} failed"))?;

        let status = response.status();
        if !status.is_success() {
            let mut bytes = Vec::new();
            response
                .body_mut()
                .read_to_end(&mut bytes)
                .await
                .with_context(|| format!("reading DELETE {url} response body failed"))?;
            let body = String::from_utf8_lossy(&bytes);
            return Err(anyhow!("DELETE {url} failed with status {status}: {body}"));
        }

        Ok(())
    }

    pub async fn list_workflows(&self) -> Result<Vec<WorkflowDefinitionRecord>> {
        self.get_json("/workflows").await
    }

    pub async fn list_workflow_node_types(&self) -> Result<Vec<WorkflowNodeType>> {
        self.get_json("/workflow-node-types").await
    }

    pub async fn get_workflow(&self, id: Uuid) -> Result<WorkflowDefinitionRecord> {
        self.get_json(&format!("/workflows/{id}")).await
    }

    pub async fn create_workflow(
        &self,
        request: &WorkflowDefinitionRequest,
    ) -> Result<WorkflowDefinitionRecord> {
        self.post_json("/workflows", request).await
    }

    pub async fn update_workflow(
        &self,
        id: Uuid,
        request: &WorkflowDefinitionRequest,
    ) -> Result<WorkflowDefinitionRecord> {
        self.put_json(&format!("/workflows/{id}"), request).await
    }

    pub async fn run_workflow(&self, id: Uuid, request: &WorkflowRunRequest) -> Result<TaskRecord> {
        self.post_json(&format!("/workflows/{id}/run"), request)
            .await
    }

    pub async fn list_tasks(&self) -> Result<Vec<TaskRecord>> {
        self.get_json("/tasks").await
    }

    pub async fn get_task_status(&self, id: Uuid) -> Result<TaskStatusResponse> {
        self.get_json(&format!("/tasks/{id}/status")).await
    }

    pub async fn get_task_node_conversation(
        &self,
        task_id: Uuid,
        node_id: &str,
    ) -> Result<TaskNodeConversationResponse> {
        self.get_json(&format!("/tasks/{task_id}/nodes/{node_id}/conversation"))
            .await
    }

    pub async fn delete_task(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("/tasks/{id}")).await
    }

    pub async fn delete_workflow(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("/workflows/{id}")).await
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
    fn test_workflow_node_type_category_display_name() {
        assert_eq!(WorkflowNodeTypeCategory::Task.display_name(), "Task");
        assert_eq!(
            WorkflowNodeTypeCategory::Validation.display_name(),
            "Validation"
        );
        assert_eq!(WorkflowNodeTypeCategory::Review.display_name(), "Review");
        assert_eq!(
            WorkflowNodeTypeCategory::Integration.display_name(),
            "Integration"
        );
    }

    #[test]
    fn test_workflow_node_type_category_all() {
        let all = WorkflowNodeTypeCategory::all();
        assert_eq!(all.len(), 4);
        assert!(all.contains(&WorkflowNodeTypeCategory::Task));
        assert!(all.contains(&WorkflowNodeTypeCategory::Validation));
        assert!(all.contains(&WorkflowNodeTypeCategory::Review));
        assert!(all.contains(&WorkflowNodeTypeCategory::Integration));
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
                node_type: "task".to_string(),
                label: "Build".to_string(),
                configuration: serde_json::json!({
                    "repo": "example/runtime",
                }),
                runtime: serde_json::json!({}),
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
        assert_eq!(request.nodes[0].configuration["repo"], "example/runtime");
        assert_eq!(request.validation_policy_ref.as_deref(), Some("policy-ref"));
        assert_eq!(
            request.trigger_metadata.get("key").map(|s| s.as_str()),
            Some("value")
        );
    }

    #[test]
    fn test_workflow_node_type_deserialization() {
        let node_types: Vec<WorkflowNodeType> = serde_json::from_value(serde_json::json!([
            {
                "id": "summarize",
                "label": "Summarize",
                "primitive": "llm",
                "inputs": [{"id": "default", "label": "Input"}],
                "outputs": [{"id": "success", "label": "Success"}],
                "configure_time_fields": [
                    {
                        "key": "model",
                        "label": "Model",
                        "field_type": "string",
                        "required": true,
                        "default_value": "gpt-5.1"
                    },
                    {
                        "key": "max_tokens",
                        "label": "Max Tokens",
                        "field_type": "number"
                    }
                ],
                "runtime_fields": [
                    {
                        "key": "response_text",
                        "label": "Response Text",
                        "field_type": "string"
                    }
                ]
            }
        ]))
        .unwrap();

        assert_eq!(node_types.len(), 1);
        assert_eq!(node_types[0].id, "summarize");
        assert_eq!(node_types[0].primitive_kind(), WorkflowNodePrimitive::Llm);
        assert_eq!(node_types[0].category, None);
        assert_eq!(node_types[0].inputs[0].id, "default");
        assert_eq!(node_types[0].outputs[0].id, "success");
        assert_eq!(node_types[0].configure_time_fields.len(), 2);
        assert_eq!(
            node_types[0].configure_time_fields[0].field_type,
            WorkflowNodeFieldType::String
        );
        assert_eq!(node_types[0].runtime_fields.len(), 1);
    }

    #[test]
    fn test_workflow_node_type_deserialization_supports_legacy_category_catalog() {
        let node_types: Vec<WorkflowNodeType> = serde_json::from_value(serde_json::json!([
            {
                "id": "task",
                "label": "Task",
                "category": "task",
                "inputs": [{"id": "default", "label": "Input"}],
                "outputs": [{"id": "success", "label": "Success"}]
            }
        ]))
        .unwrap();

        assert_eq!(node_types.len(), 1);
        assert_eq!(node_types[0].primitive_kind(), WorkflowNodePrimitive::Llm);
        assert_eq!(node_types[0].category, Some(WorkflowNodeTypeCategory::Task));
    }

    #[test]
    fn test_workflow_definition_deserializes_node_types_and_port_edges() {
        let workflow: WorkflowDefinitionRecord = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(),
            "name": "workflow",
            "nodes": [
                {
                    "id": "n1",
                    "node_type": "task",
                    "label": "Build",
                    "configuration": {"repo": "example/runtime"}
                }
            ],
            "edges": [
                {
                    "from_node_id": "n1",
                    "from_output_id": "success",
                    "to_node_id": "n2",
                    "to_input_id": "default"
                }
            ],
            "node_policies": [],
            "retry_behavior": {"max_attempts": 1, "backoff_ms": 0},
            "validation_policy_ref": null,
            "trigger_metadata": {}
        }))
        .unwrap();

        assert_eq!(workflow.nodes[0].node_type, "task");
        assert_eq!(workflow.nodes[0].configuration["repo"], "example/runtime");
        assert_eq!(workflow.edges[0].from_output_id, "success");
        assert_eq!(workflow.edges[0].to_input_id, "default");
    }

    #[test]
    fn test_workflow_definition_deserializes_legacy_graph_json() {
        let workflow: WorkflowDefinitionRecord = serde_json::from_value(serde_json::json!({
            "id": Uuid::nil(),
            "name": "legacy-workflow",
            "nodes": [
                {
                    "id": "n1",
                    "kind": "task",
                    "label": "Build",
                    "config": {"repo": "legacy/repo"}
                }
            ],
            "edges": [
                {"from": "n1", "to": "n2"}
            ],
            "node_policies": [],
            "retry_behavior": {"max_attempts": 1, "backoff_ms": 0},
            "validation_policy_ref": null,
            "trigger_metadata": {}
        }))
        .unwrap();

        assert_eq!(workflow.nodes[0].node_type, "task");
        assert_eq!(workflow.nodes[0].configuration["repo"], "legacy/repo");
        assert_eq!(workflow.edges[0].from_node_id, "n1");
        assert_eq!(workflow.edges[0].to_node_id, "n2");
        assert!(workflow.edges[0].from_output_id.is_empty());
        assert!(workflow.edges[0].to_input_id.is_empty());
    }
}
