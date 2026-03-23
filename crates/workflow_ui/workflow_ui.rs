mod canvas;
mod client;
mod inspector;
mod picker;
mod runs;

pub use client::{
    NodePolicy, RetryBehavior, TaskLifecycleStatus, TaskNodeStatus, TaskRecord, TaskStatusResponse,
    WorkflowClient, WorkflowDefinitionRecord, WorkflowDefinitionRequest, WorkflowEdge,
    WorkflowNode, WorkflowNodeKind, WorkflowRunRequest,
};

use gpui::App;
use workspace::Workspace;

pub fn init(_cx: &mut App) {}

pub fn register(
    _workspace: &mut Workspace,
    _window: &mut gpui::Window,
    _cx: &mut gpui::Context<Workspace>,
) {
}
