use crate::canvas::open_run;
use crate::client::{TaskLifecycleStatus, TaskRecord, WorkflowClient};
use gpui::{Action, App, Context, IntoElement, Render, Task, Window};
use schemars::JsonSchema;
use std::sync::Arc;
use std::time::Duration;
use ui::{ListItem, prelude::*};
use util::ResultExt;
use uuid::Uuid;
use workspace::Workspace;

#[derive(Clone, Debug, PartialEq, Action, serde::Deserialize, JsonSchema)]
#[action(namespace = workflow_ui)]
pub struct OpenWorkflowPicker;

#[derive(Clone, Debug, PartialEq, Action, serde::Deserialize, JsonSchema)]
#[action(namespace = workflow_ui)]
pub struct OpenWorkflowRun {
    pub task_id: String,
}

struct RunGroups {
    active: Vec<TaskRecord>,
    completed: Vec<TaskRecord>,
    failed: Vec<TaskRecord>,
}

pub struct WorkflowRunsView {
    runs: Vec<TaskRecord>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _fetch_task: Option<Task<()>>,
    _poll_task: Option<Task<()>>,
    _discard_task: Option<Task<()>>,
}

impl WorkflowRunsView {
    pub fn new(client: Arc<WorkflowClient>, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            runs: vec![],
            loading: true,
            error: None,
            client,
            _fetch_task: None,
            _poll_task: None,
            _discard_task: None,
        };
        view.fetch(cx);
        view.start_polling(cx);
        view
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self.loading = self.runs.is_empty();
        self.error = None;
        cx.notify();
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = client.list_tasks().await;
            if this
                .update(cx, |view, cx| {
                    view.loading = false;
                    match result {
                        Ok(runs) => view.runs = runs,
                        Err(error) => view.error = Some(error.to_string()),
                    }
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
        }));
    }

    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self._poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if this
                    .update(cx, |view, cx| {
                        view.fetch(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn discard_run(&mut self, task_id: Uuid, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._discard_task = Some(cx.spawn(async move |this, cx| {
            let result = client.delete_task(task_id).await;
            if this
                .update(cx, |view, cx| {
                    if let Err(error) = result {
                        view.error = Some(error.to_string());
                        view.loading = false;
                        cx.notify();
                        return;
                    }

                    view.fetch(cx);
                })
                .is_err()
            {
                return;
            }
        }));
    }

    fn grouped_runs(&self) -> RunGroups {
        let mut groups = RunGroups {
            active: Vec::new(),
            completed: Vec::new(),
            failed: Vec::new(),
        };

        for run in &self.runs {
            match run.status {
                TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running => {
                    groups.active.push(run.clone())
                }
                TaskLifecycleStatus::Completed => groups.completed.push(run.clone()),
                TaskLifecycleStatus::Failed => groups.failed.push(run.clone()),
            }
        }

        groups
    }

    fn render_group(
        &self,
        title: &'static str,
        runs: &[TaskRecord],
        group_index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = runs.iter().enumerate().map(move |(index, run)| {
            let task_id = run.id;
            let title = run.title.clone();
            let source_repo = run.source_repo.clone();
            let status = run.status.display_name();
            let can_discard = run.status.is_terminal();

            let mut list_item = ListItem::new(format!("{group_index}-{index}"))
                .inset(true)
                .spacing(ui::ListItemSpacing::Sparse)
                .child(
                    v_flex().gap_0p5().child(Label::new(title)).child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(source_repo).color(Color::Muted))
                            .child(Label::new(status).color(Color::Muted)),
                    ),
                )
                .on_click(move |_, window: &mut Window, cx: &mut App| {
                    window.dispatch_action(
                        Box::new(OpenWorkflowRun {
                            task_id: task_id.to_string(),
                        }),
                        cx,
                    );
                });

            if can_discard {
                let discard_task_id = run.id;
                list_item = list_item.end_slot(
                    IconButton::new(
                        format!("discard-workflow-run-{discard_task_id}"),
                        IconName::Trash,
                    )
                    .icon_color(Color::Muted)
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        this.discard_run(discard_task_id, cx);
                    })),
                );
            }

            list_item
        });

        v_flex()
            .gap_1()
            .child(Label::new(title).size(LabelSize::Small).color(Color::Muted))
            .child(v_flex().children(rows))
    }
}

impl Render for WorkflowRunsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .px_2()
            .py_1()
            .justify_between()
            .child(Label::new("Runs").size(LabelSize::Small))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        IconButton::new("refresh-workflow-runs", IconName::ArrowCircle).on_click(
                            cx.listener(|this, _, _window, cx| {
                                this.fetch(cx);
                            }),
                        ),
                    )
                    .child(
                        IconButton::new("new-workflow-run", IconName::Plus).on_click(cx.listener(
                            |_this, _, window, cx| {
                                window.dispatch_action(Box::new(OpenWorkflowPicker), cx);
                            },
                        )),
                    ),
            );

        let content: gpui::AnyElement = if self.loading && self.runs.is_empty() {
            Label::new("Loading...")
                .color(Color::Muted)
                .into_any_element()
        } else if let Some(ref error) = self.error {
            Label::new(error.clone())
                .color(Color::Error)
                .into_any_element()
        } else if self.runs.is_empty() {
            Label::new("No runs").color(Color::Muted).into_any_element()
        } else {
            let groups = self.grouped_runs();
            v_flex()
                .gap_2()
                .child(self.render_group("Running / queued", &groups.active, 0, cx))
                .child(self.render_group("Completed", &groups.completed, 1, cx))
                .child(self.render_group("Failed", &groups.failed, 2, cx))
                .into_any_element()
        };

        v_flex().size_full().child(header).child(content)
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
        move |_workspace, action: &OpenWorkflowRun, window, cx| {
            let client = client.clone();
            let Ok(task_id) = Uuid::parse_str(&action.task_id) else {
                return;
            };
            let workspace_handle = cx.entity().downgrade();
            cx.spawn_in(window, async move |_, cx| {
                let Ok(run) = client.get_task_status(task_id).await else {
                    return;
                };

                workspace_handle
                    .update_in(cx, |workspace, window, cx| {
                        open_run(run, client.clone(), workspace, window, cx);
                    })
                    .log_err();
            })
            .detach();
        }
    });
}
