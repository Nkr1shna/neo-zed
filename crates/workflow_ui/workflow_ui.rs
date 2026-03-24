mod canvas;
mod client;
mod inspector;
mod picker;
mod runs;

pub use canvas::{open_run, open_workflow, CanvasSelection, WorkflowCanvas, WorkflowCanvasEvent};
pub use inspector::{NodeInspectorPanel, OpenWorkflowDef, WorkflowDefsView};
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
    workspace: &mut Workspace,
    window: Option<&mut gpui::Window>,
    cx: &mut gpui::Context<Workspace>,
) {
    inspector::register(workspace, window, cx);
}
