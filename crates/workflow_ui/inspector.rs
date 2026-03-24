use crate::canvas::open_workflow;
use crate::client::{
    RetryBehavior, WorkflowClient, WorkflowDefinitionRecord, WorkflowDefinitionRequest,
};
use editor::Editor;
use gpui::{App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Task, Window};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use ui::{ListItem, prelude::*};
use uuid::Uuid;
use workspace::dock::{DockPosition, PanelEvent};
use workspace::{Panel, Workspace};

#[derive(Clone, Debug, PartialEq, gpui::Action, serde::Deserialize, schemars::JsonSchema)]
pub struct OpenWorkflowDef {
    pub id: String,
}

gpui::actions!(workflow_ui, [NewWorkflow, ToggleNodeInspector, PublishWorkflow, SaveWorkflowDraft]);

pub struct WorkflowDefsView {
    workflows: Vec<WorkflowDefinitionRecord>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _fetch_task: Option<Task<()>>,
}

impl WorkflowDefsView {
    pub fn new(client: Arc<WorkflowClient>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            workflows: vec![],
            loading: true,
            error: None,
            client,
            _fetch_task: None,
        };
        view.fetch(cx);
        view
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = client.list_workflows().await;
            this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(workflows) => view.workflows = workflows,
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Render for WorkflowDefsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .px_2()
            .py_1()
            .justify_between()
            .child(Label::new("Workflows").size(LabelSize::Small))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        IconButton::new("refresh-workflows", IconName::ArrowCircle).on_click(
                            cx.listener(|this, _, _window, cx| {
                                this.fetch(cx);
                            }),
                        ),
                    )
                    .child(
                        IconButton::new("new-workflow", IconName::Plus).on_click(cx.listener(
                            |_this, _, window, cx| {
                                window.dispatch_action(Box::new(NewWorkflow), cx);
                            },
                        )),
                    ),
            );

        let content: gpui::AnyElement = if self.loading {
            Label::new("Loading...")
                .color(Color::Muted)
                .into_any_element()
        } else if let Some(ref error) = self.error {
            Label::new(error.clone())
                .color(Color::Error)
                .into_any_element()
        } else if self.workflows.is_empty() {
            Label::new("No workflows")
                .color(Color::Muted)
                .into_any_element()
        } else {
            v_flex()
                .children(self.workflows.iter().enumerate().map(|(index, workflow)| {
                    let workflow_id = workflow.id.to_string();
                    let name = workflow.name.clone();
                    ListItem::new(index)
                        .child(Label::new(name))
                        .on_click(move |_, window: &mut Window, cx: &mut App| {
                            window.dispatch_action(
                                Box::new(OpenWorkflowDef { id: workflow_id.clone() }),
                                cx,
                            );
                        })
                }))
                .into_any_element()
        };

        v_flex().size_full().child(header).child(content)
    }
}

enum PublishState {
    Idle,
    Publishing,
    Success,
    Error(String),
}

pub struct NodeInspectorPanel {
    workflow: Option<WorkflowDefinitionRecord>,
    selected_node_id: Option<String>,
    focus_handle: FocusHandle,
    label_editor: gpui::Entity<Editor>,
    required_reviews_editor: gpui::Entity<Editor>,
    required_checks_editor: gpui::Entity<Editor>,
    max_attempts_editor: gpui::Entity<Editor>,
    backoff_ms_editor: gpui::Entity<Editor>,
    is_dirty: bool,
    publish_state: PublishState,
    client: Arc<WorkflowClient>,
    position: DockPosition,
    _publish_task: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl NodeInspectorPanel {
    pub fn new(client: Arc<WorkflowClient>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let label_editor = cx.new(|cx| Editor::single_line(window, cx));
        let required_reviews_editor = cx.new(|cx| Editor::single_line(window, cx));
        let required_checks_editor = cx.new(|cx| Editor::single_line(window, cx));
        let max_attempts_editor = cx.new(|cx| Editor::single_line(window, cx));
        let backoff_ms_editor = cx.new(|cx| Editor::single_line(window, cx));

        Self {
            workflow: None,
            selected_node_id: None,
            focus_handle: cx.focus_handle(),
            label_editor,
            required_reviews_editor,
            required_checks_editor,
            max_attempts_editor,
            backoff_ms_editor,
            is_dirty: false,
            publish_state: PublishState::Idle,
            client,
            position: DockPosition::Right,
            _publish_task: None,
            _subscriptions: Vec::new(),
        }
    }

    pub fn set_workflow(&mut self, workflow: WorkflowDefinitionRecord, cx: &mut Context<Self>) {
        self.workflow = Some(workflow);
        self.selected_node_id = None;
        self.is_dirty = false;
        cx.notify();
    }

    pub fn set_node(
        &mut self,
        node_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_node_id = node_id.clone();

        let Some(ref node_id) = node_id else {
            cx.notify();
            return;
        };

        let Some(ref workflow) = self.workflow else {
            cx.notify();
            return;
        };

        if let Some(node) = workflow.nodes.iter().find(|n| &n.id == node_id) {
            self.label_editor.update(cx, |editor, cx| {
                editor.set_text(node.label.clone(), window, cx);
            });
        }

        if let Some(policy) = workflow.policy_for(node_id) {
            self.required_reviews_editor.update(cx, |editor, cx| {
                editor.set_text(policy.required_reviews.to_string(), window, cx);
            });
            self.required_checks_editor.update(cx, |editor, cx| {
                editor.set_text(policy.required_checks.join(", "), window, cx);
            });
            self.max_attempts_editor.update(cx, |editor, cx| {
                editor.set_text(policy.retry_behavior.max_attempts.to_string(), window, cx);
            });
            self.backoff_ms_editor.update(cx, |editor, cx| {
                editor.set_text(policy.retry_behavior.backoff_ms.to_string(), window, cx);
            });
        } else {
            self.required_reviews_editor.update(cx, |editor, cx| {
                editor.set_text("0", window, cx);
            });
            self.required_checks_editor.update(cx, |editor, cx| {
                editor.set_text("", window, cx);
            });
            self.max_attempts_editor.update(cx, |editor, cx| {
                editor.set_text("3", window, cx);
            });
            self.backoff_ms_editor.update(cx, |editor, cx| {
                editor.set_text("1000", window, cx);
            });
        }

        cx.notify();
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let Some(ref workflow) = self.workflow else {
            return;
        };

        let request = WorkflowDefinitionRequest {
            name: workflow.name.clone(),
            nodes: workflow.nodes.clone(),
            edges: workflow.edges.clone(),
            node_policies: workflow.node_policies.clone(),
            retry_behavior: workflow.retry_behavior.clone(),
            validation_policy_ref: workflow.validation_policy_ref.clone(),
            trigger_metadata: workflow.trigger_metadata.clone(),
        };

        let client = self.client.clone();
        let workflow_id = workflow.id;
        let is_new = workflow_id.is_nil();

        self.publish_state = PublishState::Publishing;
        cx.notify();

        self._publish_task = Some(cx.spawn(async move |this, cx| {
            let result = if is_new {
                client.create_workflow(&request).await.map(|_| ())
            } else {
                client.update_workflow(workflow_id, &request).await.map(|_| ())
            };

            this.update(cx, |panel, cx| {
                panel.publish_state = match result {
                    Ok(()) => PublishState::Success,
                    Err(error) => PublishState::Error(error.to_string()),
                };
                cx.notify();
            })
            .ok();

            cx.background_executor().timer(Duration::from_secs(3)).await;

            this.update(cx, |panel, cx| {
                panel.publish_state = PublishState::Idle;
                cx.notify();
            })
            .ok();
        }));
    }
}

impl EventEmitter<PanelEvent> for NodeInspectorPanel {}

impl Focusable for NodeInspectorPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for NodeInspectorPanel {
    fn persistent_name() -> &'static str {
        "NodeInspectorPanel"
    }

    fn panel_key() -> &'static str {
        "NodeInspectorPanel"
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        self.position
    }

    fn position_is_valid(&self, _position: DockPosition) -> bool {
        true
    }

    fn set_position(
        &mut self,
        position: DockPosition,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.position = position;
        cx.notify();
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
        Some(IconName::Sliders)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Node Inspector")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleNodeInspector)
    }

    fn activation_priority(&self) -> u32 {
        3
    }

    fn set_active(&mut self, _active: bool, _window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
    }
}

impl Render for NodeInspectorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .px_2()
            .py_1()
            .child(Label::new("Node Inspector").size(LabelSize::Small));

        let body: gpui::AnyElement = if self.selected_node_id.is_none() {
            v_flex()
                .size_full()
                .justify_center()
                .items_center()
                .child(Label::new("Select a node to inspect").color(Color::Muted))
                .into_any_element()
        } else {
            let publish_label: SharedString = match &self.publish_state {
                PublishState::Idle => "Publish".into(),
                PublishState::Publishing => "Publishing...".into(),
                PublishState::Success => "Published!".into(),
                PublishState::Error(_) => "Error".into(),
            };

            let publish_color = match &self.publish_state {
                PublishState::Success => Color::Success,
                PublishState::Error(_) => Color::Error,
                _ => Color::Default,
            };

            let error_message: Option<gpui::AnyElement> =
                if let PublishState::Error(ref message) = self.publish_state {
                    Some(
                        Label::new(message.clone())
                            .color(Color::Error)
                            .into_any_element(),
                    )
                } else {
                    None
                };

            v_flex()
                .size_full()
                .gap_2()
                .p_2()
                .child(
                    v_flex()
                        .gap_1()
                        .child(Label::new("Label").size(LabelSize::Small).color(Color::Muted))
                        .child(self.label_editor.clone()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("Required Reviews")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(self.required_reviews_editor.clone()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("Required Checks")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(self.required_checks_editor.clone()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("Max Attempts")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(self.max_attempts_editor.clone()),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            Label::new("Backoff (ms)")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(self.backoff_ms_editor.clone()),
                )
                .when_some(error_message, |this, message| this.child(message))
                .child(
                    h_flex().justify_end().child(
                        Button::new("publish-workflow", publish_label)
                            .color(publish_color)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.publish(cx);
                            })),
                    ),
                )
                .into_any_element()
        };

        v_flex().size_full().child(header).child(body)
    }
}

pub fn register(
    workspace: &mut Workspace,
    window: Option<&mut Window>,
    cx: &mut Context<Workspace>,
) {
    let client = WorkflowClient::new();

    if let Some(window) = window {
        let panel = cx.new(|cx| NodeInspectorPanel::new(client.clone(), window, cx));
        workspace.add_panel(panel, window, cx);
    }

    workspace.register_action({
        let client = client.clone();
        move |workspace, _action: &NewWorkflow, window, cx| {
            let blank_workflow = WorkflowDefinitionRecord {
                id: Uuid::nil(),
                name: "New Workflow".to_string(),
                nodes: vec![],
                edges: vec![],
                node_policies: vec![],
                retry_behavior: RetryBehavior::default(),
                validation_policy_ref: None,
                trigger_metadata: BTreeMap::new(),
            };
            open_workflow(blank_workflow, client.clone(), workspace, window, cx);
        }
    });

    workspace.register_action({
        let client = client.clone();
        move |_workspace, action: &OpenWorkflowDef, window, cx| {
            let client = client.clone();
            let Ok(workflow_id) = action.id.parse::<Uuid>() else {
                return;
            };
            let workspace_handle = cx.entity().downgrade();
            cx.spawn_in(window, async move |_, cx| {
                let Ok(workflow) = client.get_workflow(workflow_id).await else {
                    return;
                };
                workspace_handle
                    .update_in(cx, |workspace, window, cx| {
                        open_workflow(workflow, client.clone(), workspace, window, cx);
                    })
                    .ok();
            })
            .detach();
        }
    });
}
