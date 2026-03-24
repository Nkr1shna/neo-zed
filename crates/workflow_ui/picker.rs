use std::sync::Arc;

use editor::Editor;
use gpui::{
    AnyElement, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, Task, WeakEntity, Window,
};
use picker::{Picker, PickerDelegate, PickerEditorPosition};
use ui::{
    Button, ButtonStyle, Color, Label, LabelSize, ListItem, ListItemSpacing, ParentElement, Render,
    SharedString, Styled, prelude::*,
};
use util::ResultExt;
use workspace::{ModalView, Workspace};

use crate::{
    canvas::open_run,
    client::{
        TaskRecord, TaskStatusResponse, WorkflowClient, WorkflowDefinitionRecord,
        WorkflowRunRequest,
    },
    runs::OpenWorkflowPicker,
};

pub struct WorkflowPicker {
    client: Arc<WorkflowClient>,
    workspace: WeakEntity<Workspace>,
    picker: Entity<Picker<WorkflowPickerDelegate>>,
    title_editor: Entity<Editor>,
    source_repo_editor: Entity<Editor>,
    task_description_editor: Entity<Editor>,
    selected_workflow: Option<WorkflowDefinitionRecord>,
    creating: bool,
    submission_error: Option<SharedString>,
    _load_workflows_task: Task<()>,
    submission_task: Option<Task<()>>,
}

impl WorkflowPicker {
    pub fn new(
        client: Arc<WorkflowClient>,
        workspace: WeakEntity<Workspace>,
        default_source_repo: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let modal = cx.entity().downgrade();
        let picker = cx.new(|cx| {
            Picker::uniform_list(WorkflowPickerDelegate::new(modal), window, cx).modal(false)
        });

        let title_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Run title", window, cx);
            editor
        });
        let source_repo_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Source repository", window, cx);
            if !default_source_repo.is_empty() {
                editor.set_text(default_source_repo, window, cx);
            }
            editor
        });
        let task_description_editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(3, 10, window, cx);
            editor.set_placeholder_text("Task description", window, cx);
            editor
        });

        let load_workflows_task = cx.spawn_in(window, {
            let client = client.clone();
            async move |this, cx| {
                let result = client.list_workflows().await;
                this.update_in(cx, |this, window, cx| {
                    this.finish_loading_workflows(result, window, cx);
                })
                .log_err();
            }
        });

        Self {
            client,
            workspace,
            picker,
            title_editor,
            source_repo_editor,
            task_description_editor,
            selected_workflow: None,
            creating: false,
            submission_error: None,
            _load_workflows_task: load_workflows_task,
            submission_task: None,
        }
    }

    fn finish_loading_workflows(
        &mut self,
        result: anyhow::Result<Vec<WorkflowDefinitionRecord>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picker.update(cx, |picker, cx| {
            match result {
                Ok(mut workflows) => {
                    workflows.sort_by(|left, right| {
                        left.name.to_lowercase().cmp(&right.name.to_lowercase())
                    });
                    picker.delegate.set_workflows(workflows);
                }
                Err(error) => picker.delegate.set_load_error(error.to_string()),
            }
            picker.refresh(window, cx);
        });
    }

    fn select_workflow(&mut self, workflow: WorkflowDefinitionRecord, cx: &mut Context<Self>) {
        self.selected_workflow = Some(workflow);
        self.creating = false;
        self.submission_error = None;
        cx.notify();
    }

    fn cancel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_workflow = None;
        self.creating = false;
        self.submission_error = None;
        self.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn start_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }

        let Some(workflow) = self.selected_workflow.clone() else {
            self.submission_error = Some("Select a workflow first".into());
            cx.notify();
            return;
        };

        let title = self.title_editor.read(cx).text(cx).trim().to_string();
        let source_repo = self.source_repo_editor.read(cx).text(cx).trim().to_string();
        let task_description = self
            .task_description_editor
            .read(cx)
            .text(cx)
            .trim()
            .to_string();

        if title.is_empty() {
            self.submission_error = Some("Title is required".into());
            self.focus_handle(cx).focus(window, cx);
            cx.notify();
            return;
        }

        if source_repo.is_empty() {
            self.submission_error = Some("Source repo is required".into());
            self.focus_handle(cx).focus(window, cx);
            cx.notify();
            return;
        }

        self.creating = true;
        self.submission_error = None;
        cx.notify();

        let request = WorkflowRunRequest {
            title,
            source_repo,
            task_description: (!task_description.is_empty()).then_some(task_description),
        };
        let client = self.client.clone();
        let workspace = self.workspace.clone();
        let workflow_id = workflow.id;
        let workflow_for_fallback = workflow;

        self.submission_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result: anyhow::Result<TaskStatusResponse> = async {
                let task = client.run_workflow(workflow_id, &request).await?;
                let run = match client.get_task_status(task.id).await {
                    Ok(run) => run,
                    Err(_) => fallback_task_status(task, workflow_for_fallback.clone()),
                };
                Ok(run)
            }
            .await;

            match result {
                Ok(run) => {
                    workspace
                        .update_in(cx, |workspace, window, cx| {
                            workspace.hide_modal(window, cx);
                            open_run(run, client.clone(), workspace, window, cx);
                        })
                        .log_err();
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.creating = false;
                        this.submission_task = None;
                        this.submission_error = Some(error.to_string().into());
                        cx.notify();
                    })
                    .log_err();
                }
            }
        }));
    }

    fn render_form(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let workflow = self
            .selected_workflow
            .as_ref()
            .expect("workflow must be selected before rendering the form");
        let start_label = if self.creating {
            "Starting…"
        } else {
            "Start"
        };

        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                Label::new("Selected workflow")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(Label::new(workflow.name.clone())),
                    )
                    .child(
                        Button::new("change-workflow", "Change")
                            .style(ButtonStyle::Subtle)
                            .disabled(self.creating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_selection(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Title")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(self.title_editor.clone()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Source repo")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(self.source_repo_editor.clone()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new("Task description")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(self.task_description_editor.clone()),
            )
            .when_some(self.submission_error.clone(), |this, error| {
                this.child(Label::new(error).color(Color::Error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-workflow-run", "Cancel")
                            .style(ButtonStyle::Subtle)
                            .disabled(self.creating)
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                    .child(
                        Button::new("start-workflow-run", start_label)
                            .style(ButtonStyle::Filled)
                            .disabled(self.creating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_run(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}

fn fallback_task_status(
    task: TaskRecord,
    workflow: WorkflowDefinitionRecord,
) -> TaskStatusResponse {
    TaskStatusResponse {
        task,
        workflow: Some(workflow),
        workspace_path: None,
        remote_target: None,
        nodes: Vec::new(),
        outcome: None,
        agent: None,
        lease: None,
        validation: None,
        integration: None,
        failure_message: None,
        agents: None,
    }
}

impl EventEmitter<DismissEvent> for WorkflowPicker {}

impl Focusable for WorkflowPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.selected_workflow.is_some() {
            self.title_editor.focus_handle(cx)
        } else {
            self.picker.focus_handle(cx)
        }
    }
}

impl ModalView for WorkflowPicker {}

impl Render for WorkflowPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let heading = if self.selected_workflow.is_some() {
            "New run"
        } else {
            "Select workflow"
        };

        v_flex()
            .key_context("WorkflowPicker")
            .elevation_2(cx)
            .w(rems(38.))
            .gap_3()
            .p_3()
            .child(Label::new(heading).size(LabelSize::Large))
            .child(if self.selected_workflow.is_some() {
                self.render_form(cx)
            } else {
                self.picker.clone().into_any_element()
            })
    }
}

pub struct WorkflowPickerDelegate {
    modal: WeakEntity<WorkflowPicker>,
    workflows: Vec<WorkflowDefinitionRecord>,
    matches: Vec<usize>,
    selected_index: usize,
    loading: bool,
    load_error: Option<SharedString>,
}

impl WorkflowPickerDelegate {
    pub fn new(modal: WeakEntity<WorkflowPicker>) -> Self {
        Self {
            modal,
            workflows: Vec::new(),
            matches: Vec::new(),
            selected_index: 0,
            loading: true,
            load_error: None,
        }
    }

    fn set_workflows(&mut self, workflows: Vec<WorkflowDefinitionRecord>) {
        self.workflows = workflows;
        self.matches = (0..self.workflows.len()).collect();
        self.selected_index = 0;
        self.loading = false;
        self.load_error = None;
    }

    fn set_load_error(&mut self, message: String) {
        self.workflows.clear();
        self.matches.clear();
        self.selected_index = 0;
        self.loading = false;
        self.load_error = Some(message.into());
    }
}

impl PickerDelegate for WorkflowPickerDelegate {
    type ListItem = ListItem;

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = ix.min(self.matches.len().saturating_sub(1));
        cx.notify();
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        "Select a workflow…".into()
    }

    fn no_matches_text(&self, _window: &mut Window, _cx: &mut App) -> Option<SharedString> {
        if self.loading {
            return Some("Loading workflows…".into());
        }

        if let Some(error) = &self.load_error {
            return Some(format!("Failed to load workflows: {error}").into());
        }

        Some("No workflows found".into())
    }

    fn editor_position(&self) -> PickerEditorPosition {
        PickerEditorPosition::Start
    }

    fn update_matches(
        &mut self,
        query: String,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        let query = query.trim().to_lowercase();

        if self.loading {
            self.matches.clear();
            self.selected_index = 0;
            cx.notify();
            return Task::ready(());
        }

        self.matches = self
            .workflows
            .iter()
            .enumerate()
            .filter_map(|(index, workflow)| {
                let name = workflow.name.to_lowercase();
                if query.is_empty() || name.contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();
        self.selected_index = self
            .selected_index
            .min(self.matches.len().saturating_sub(1));
        cx.notify();
        Task::ready(())
    }

    fn confirm(&mut self, secondary: bool, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if secondary {
            return;
        }

        let Some(&workflow_index) = self.matches.get(self.selected_index) else {
            return;
        };
        let Some(workflow) = self.workflows.get(workflow_index).cloned() else {
            return;
        };

        self.modal
            .update(cx, |modal, cx| {
                modal.select_workflow(workflow, cx);
            })
            .log_err();
        cx.notify();
    }

    fn dismissed(&mut self, _window: &mut Window, cx: &mut Context<Picker<Self>>) {
        self.modal
            .update(cx, |_modal, cx| cx.emit(DismissEvent))
            .log_err();
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let workflow_index = *self.matches.get(ix)?;
        let workflow = self.workflows.get(workflow_index)?;

        Some(
            ListItem::new(ix)
                .toggle_state(selected)
                .inset(true)
                .spacing(ListItemSpacing::Sparse)
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(Label::new(workflow.name.clone()))
                        .child(
                            Label::new(workflow.id.to_string())
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        ),
                ),
        )
    }
}

pub fn register(
    workspace: &mut Workspace,
    _window: Option<&mut Window>,
    _cx: &mut Context<Workspace>,
) {
    let client = WorkflowClient::new();

    workspace.register_action({
        let client = client.clone();
        move |workspace, _: &OpenWorkflowPicker, window, cx| {
            let workspace_handle = workspace.weak_handle();
            let default_source_repo = workspace
                .root_paths(cx)
                .first()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();

            workspace.toggle_modal(window, cx, {
                let client = client.clone();
                move |window, cx| {
                    WorkflowPicker::new(
                        client.clone(),
                        workspace_handle.clone(),
                        default_source_repo.clone(),
                        window,
                        cx,
                    )
                }
            });
        }
    });
}
