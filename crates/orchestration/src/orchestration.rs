use serde::{Deserialize, Serialize};

pub const CURRENT_ORCHESTRATION_STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationState {
    pub selected_node_id: Option<OrchestrationNodeId>,
    pub projects: Vec<OrchestrationProject>,
    pub features: Vec<OrchestrationFeature>,
    pub tasks: Vec<OrchestrationTask>,
    pub workflow_runs: Vec<WorkflowRun>,
}

impl OrchestrationState {
    pub fn serialize(&self) -> serde_json::Result<String> {
        serde_json::to_string(&VersionedOrchestrationState::new(self.clone()))
    }

    pub fn deserialize(json: &str) -> Result<Self, OrchestrationStatePersistenceError> {
        let version_only: VersionOnlyOrchestrationState = serde_json::from_str(json)?;
        if version_only.version != CURRENT_ORCHESTRATION_STATE_VERSION {
            return Err(OrchestrationStatePersistenceError::UnsupportedVersion(
                version_only.version,
            ));
        }
        let versioned_state: VersionedOrchestrationState = serde_json::from_str(json)?;
        Ok(versioned_state.state)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct VersionOnlyOrchestrationState {
    version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct VersionedOrchestrationState {
    version: u32,
    #[serde(flatten)]
    state: OrchestrationState,
}

impl VersionedOrchestrationState {
    fn new(state: OrchestrationState) -> Self {
        Self {
            version: CURRENT_ORCHESTRATION_STATE_VERSION,
            state,
        }
    }
}

#[derive(Debug)]
pub enum OrchestrationStatePersistenceError {
    InvalidJson(serde_json::Error),
    UnsupportedVersion(u32),
}

impl std::fmt::Display for OrchestrationStatePersistenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(
                formatter,
                "failed to deserialize orchestration state: {error}"
            ),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "unsupported orchestration state version: {version}"
            ),
        }
    }
}

impl std::error::Error for OrchestrationStatePersistenceError {}

impl From<serde_json::Error> for OrchestrationStatePersistenceError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidJson(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrchestrationProjectId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrchestrationFeatureId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrchestrationTaskId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowRunId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowStepId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrchestrationNodeId {
    Project(OrchestrationProjectId),
    Feature(OrchestrationFeatureId),
    Task(OrchestrationTaskId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationProject {
    pub id: OrchestrationProjectId,
    pub title: String,
    #[serde(default)]
    pub feature_ids: Vec<OrchestrationFeatureId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationFeature {
    pub id: OrchestrationFeatureId,
    pub project_id: OrchestrationProjectId,
    pub title: String,
    pub status: WorkflowStatus,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub specification: Option<OrchestrationArtifactRef>,
    pub implementation_plan: Option<OrchestrationArtifactRef>,
    #[serde(default)]
    pub task_ids: Vec<OrchestrationTaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationTask {
    pub id: OrchestrationTaskId,
    pub feature_id: OrchestrationFeatureId,
    pub title: String,
    pub status: WorkflowStatus,
    #[serde(default)]
    pub linked_artifacts: Vec<OrchestrationArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: WorkflowRunId,
    pub task_id: OrchestrationTaskId,
    pub active_step_id: Option<WorkflowStepId>,
    #[serde(default)]
    pub steps: Vec<WorkflowStep>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: WorkflowStepId,
    pub kind: WorkflowStepKind,
    pub status: WorkflowStatus,
    pub artifact: Option<OrchestrationArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStepKind {
    Brainstorm,
    Specification,
    Planning,
    Implementation,
    Review,
    CommitAndPush,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrchestrationArtifactRef {
    AgentThread { session_id: String },
    TextThread { path: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn sample_state() -> OrchestrationState {
        let project_id = OrchestrationProjectId("project-alpha".to_string());
        let feature_id = OrchestrationFeatureId("feature-auth".to_string());
        let task_id = OrchestrationTaskId("task-review-login".to_string());
        let workflow_run_id = WorkflowRunId("workflow-review-login".to_string());
        let brainstorming_step_id = WorkflowStepId("step-brainstorm".to_string());
        let planning_step_id = WorkflowStepId("step-plan".to_string());

        OrchestrationState {
            selected_node_id: Some(OrchestrationNodeId::Task(task_id.clone())),
            projects: vec![OrchestrationProject {
                id: project_id.clone(),
                title: "Project Alpha".to_string(),
                feature_ids: vec![feature_id.clone()],
            }],
            features: vec![OrchestrationFeature {
                id: feature_id.clone(),
                project_id,
                title: "Authentication".to_string(),
                status: WorkflowStatus::InProgress,
                acceptance_criteria: vec![
                    "User can sign in with valid credentials".to_string(),
                    "Review sign-off is captured before merge".to_string(),
                ],
                specification: Some(OrchestrationArtifactRef::TextThread {
                    path: ".zed/text-threads/spec-auth.json".to_string(),
                }),
                implementation_plan: Some(OrchestrationArtifactRef::AgentThread {
                    session_id: "session-plan-456".to_string(),
                }),
                task_ids: vec![task_id.clone()],
            }],
            tasks: vec![OrchestrationTask {
                id: task_id.clone(),
                feature_id,
                title: "Review login flow".to_string(),
                status: WorkflowStatus::InProgress,
                linked_artifacts: vec![OrchestrationArtifactRef::AgentThread {
                    session_id: "session-123".to_string(),
                }],
            }],
            workflow_runs: vec![WorkflowRun {
                id: workflow_run_id,
                task_id,
                active_step_id: Some(planning_step_id.clone()),
                steps: vec![
                    WorkflowStep {
                        id: brainstorming_step_id,
                        kind: WorkflowStepKind::Brainstorm,
                        status: WorkflowStatus::Completed,
                        artifact: Some(OrchestrationArtifactRef::TextThread {
                            path: ".zed/text-threads/brainstorm.json".to_string(),
                        }),
                    },
                    WorkflowStep {
                        id: planning_step_id,
                        kind: WorkflowStepKind::Planning,
                        status: WorkflowStatus::InProgress,
                        artifact: Some(OrchestrationArtifactRef::AgentThread {
                            session_id: "session-123".to_string(),
                        }),
                    },
                ],
            }],
        }
    }

    #[test]
    fn orchestration_state_round_trips_through_json() {
        let state = sample_state();

        let serialized_state = state.serialize().expect("state should serialize");
        let deserialized_state =
            OrchestrationState::deserialize(&serialized_state).expect("state should deserialize");

        assert_eq!(deserialized_state, state);
    }

    #[test]
    fn orchestration_state_rejects_unknown_versions() {
        let json = r#"{"version":999,"selected_node_id":null,"projects":[],"features":[],"tasks":[],"workflow_runs":[]}"#;

        let error =
            OrchestrationState::deserialize(json).expect_err("unsupported version should fail");

        assert!(matches!(
            error,
            OrchestrationStatePersistenceError::UnsupportedVersion(999)
        ));
    }

    #[test]
    fn orchestration_state_preserves_artifact_links() {
        let state = sample_state();

        let serialized_state = state.serialize().expect("state should serialize");
        let deserialized_state =
            OrchestrationState::deserialize(&serialized_state).expect("state should deserialize");

        assert_eq!(deserialized_state.workflow_runs.len(), 1);
        assert_eq!(deserialized_state.workflow_runs[0].steps.len(), 2);
        assert_eq!(
            deserialized_state.workflow_runs[0].steps[0].artifact,
            Some(OrchestrationArtifactRef::TextThread {
                path: ".zed/text-threads/brainstorm.json".to_string(),
            })
        );
        assert_eq!(
            deserialized_state.workflow_runs[0].steps[1].artifact,
            Some(OrchestrationArtifactRef::AgentThread {
                session_id: "session-123".to_string(),
            })
        );
    }

    #[test]
    fn orchestration_state_rejects_legacy_v2_payload_before_struct_deserialization() {
        let json = r#"{
            "version": 2,
            "selected_node": null,
            "templates": [],
            "template_revisions": [],
            "checkpoints": [],
            "ledger_events": [],
            "active_run_id": null,
            "workflow_runs": [
                {
                    "id": "run-1774067370226259",
                    "template_id": "template-1774064065549496",
                    "template_revision_id": "template-1774064065549496@8",
                    "bound_project_id": "19",
                    "status": "Pending",
                    "selected_node_id": null,
                    "active_checkpoint_id": null,
                    "started_at_epoch_millis": null,
                    "completed_at_epoch_millis": null,
                    "current_node_id": null,
                    "last_error": null
                }
            ]
        }"#;

        let error =
            OrchestrationState::deserialize(json).expect_err("legacy payload should be rejected");

        assert!(matches!(
            error,
            OrchestrationStatePersistenceError::UnsupportedVersion(2)
        ));
    }
}
