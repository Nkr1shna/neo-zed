mod canvas;
mod client;
mod inspector;
mod picker;
mod runs;

pub use canvas::{CanvasSelection, WorkflowCanvas, WorkflowCanvasEvent, open_run, open_workflow};
pub use client::{
    NodePolicy, RetryBehavior, TaskLifecycleStatus, TaskNodeStatus, TaskRecord, TaskStatusResponse,
    WorkflowClient, WorkflowDefinitionRecord, WorkflowDefinitionRequest, WorkflowEdge,
    WorkflowNode, WorkflowNodePort, WorkflowNodeType, WorkflowNodeTypeCategory, WorkflowRunRequest,
};
pub use inspector::{NewWorkflow, NodeInspectorPanel, OpenWorkflowDef, WorkflowDefsView};
pub use picker::{WorkflowPicker, WorkflowPickerDelegate};
pub use runs::{OpenWorkflowPicker, OpenWorkflowRun, WorkflowRunsView};

use gpui::App;
use ui::{Color, IconButton, IconName, IconSize};
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

pub(crate) const WORKFLOW_TOOLBAR_ICON_SIZE: IconSize = IconSize::Small;
pub(crate) const WORKFLOW_TOOLBAR_ICON_COLOR: Color = Color::Muted;

pub(crate) fn workflow_toolbar_icon_button(
    id: impl Into<gpui::ElementId>,
    icon: IconName,
) -> IconButton {
    IconButton::new(id, icon)
        .icon_size(WORKFLOW_TOOLBAR_ICON_SIZE)
        .icon_color(WORKFLOW_TOOLBAR_ICON_COLOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_toolbar_icons_use_small_muted_styling() {
        assert!(WORKFLOW_TOOLBAR_ICON_SIZE == IconSize::Small);
        assert!(WORKFLOW_TOOLBAR_ICON_COLOR == Color::Muted);
    }
}
