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

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        register(workspace, window, cx);
    })
    .detach();
}

pub fn register(
    _workspace: &mut Workspace,
    _window: Option<&mut gpui::Window>,
    _cx: &mut gpui::Context<Workspace>,
) {
    // Panel and action registration — filled in by Workstreams 2, 3, 4
}
