mod canvas;
mod client;
mod inspector;
mod picker;
mod runs;

pub use canvas::{CanvasSelection, WorkflowCanvas, WorkflowCanvasEvent, open_run, open_workflow};
pub use client::{
    NodePolicy, RetryBehavior, TaskLifecycleStatus, TaskNodeStatus, TaskRecord, TaskStatusResponse,
    WorkflowClient, WorkflowDefinitionRecord, WorkflowDefinitionRequest, WorkflowEdge,
    WorkflowNode, WorkflowNodeKind, WorkflowRunRequest,
};
pub use inspector::{NodeInspectorPanel, OpenWorkflowDef, WorkflowDefsView};
pub use picker::{WorkflowPicker, WorkflowPickerDelegate};
pub use runs::{OpenWorkflowPicker, OpenWorkflowRun, WorkflowRunsView};

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
    runs::register(workspace, None, cx);
    picker::register(workspace, None, cx);
}
