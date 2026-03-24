use crate::canvas::open_run;
use crate::client::{TaskLifecycleStatus, TaskRecord, WorkflowClient};
use crate::workflow_toolbar_icon_button;
use editor::Editor;
use gpui::{
    Action, App, Context, Entity, Focusable, IntoElement, Render, Subscription, Task, Window,
};
use schemars::JsonSchema;
use std::sync::Arc;
use std::time::Duration;
use ui::{Divider, ListItem, Tooltip, prelude::*, utils::platform_title_bar_height};
use util::ResultExt;
use uuid::Uuid;
use workspace::{Toast, Workspace, notifications::NotificationId};

#[derive(Clone, Debug, PartialEq, Action, serde::Deserialize, JsonSchema)]
#[action(namespace = workflow_ui)]
pub struct OpenWorkflowPicker;

#[derive(Clone, Debug, PartialEq, Action, serde::Deserialize, JsonSchema)]
#[action(namespace = workflow_ui)]
pub struct OpenWorkflowRun {
    pub task_id: String,
}

struct OpenWorkflowRunErrorToast;

struct RunGroups {
    active: Vec<TaskRecord>,
    completed: Vec<TaskRecord>,
    failed: Vec<TaskRecord>,
}

pub struct WorkflowRunsView {
    runs: Vec<TaskRecord>,
    search_query: String,
    filter_editor: Entity<Editor>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    _subscriptions: Vec<Subscription>,
    _fetch_task: Option<Task<()>>,
    _poll_task: Option<Task<()>>,
    _discard_task: Option<Task<()>>,
}

impl WorkflowRunsView {
    pub fn new(client: Arc<WorkflowClient>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            runs: vec![],
            search_query: String::new(),
            filter_editor: cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text("Search runs…", window, cx);
                editor
            }),
            loading: true,
            error: None,
            client,
            _subscriptions: Vec::new(),
            _fetch_task: None,
            _poll_task: None,
            _discard_task: None,
        };
        view.bind_filter_editor(cx);
        view.fetch(cx);
        view.start_polling(cx);
        view
    }

    #[cfg(test)]
    fn new_for_test(
        client: Arc<WorkflowClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            runs: vec![],
            search_query: String::new(),
            filter_editor: cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text("Search runs…", window, cx);
                editor
            }),
            loading: false,
            error: None,
            client,
            _subscriptions: Vec::new(),
            _fetch_task: None,
            _poll_task: None,
            _discard_task: None,
        };
        view.bind_filter_editor(cx);
        view
    }

    fn bind_filter_editor(&mut self, cx: &mut Context<Self>) {
        let filter_editor = self.filter_editor.clone();
        self._subscriptions.push(
            cx.subscribe(&filter_editor, |view: &mut Self, _, event, cx| {
                if let editor::EditorEvent::BufferEdited = event {
                    let query = view.filter_editor.read(cx).text(cx).to_string();
                    if view.set_search_query(query) {
                        cx.notify();
                    }
                }
            }),
        );
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

    pub fn set_search_query(&mut self, query: impl Into<String>) -> bool {
        let query = query.into();
        if self.search_query == query {
            return false;
        }
        self.search_query = query;
        true
    }

    pub fn focus_filter_editor(&self, window: &mut Window, cx: &mut App) {
        let handle = self.filter_editor.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.fetch(cx);
    }

    pub fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.filter_editor.update(cx, |editor, cx| {
            if editor.buffer().read(cx).len(cx).0 > 0 {
                editor.set_text("", window, cx);
                true
            } else {
                false
            }
        })
    }

    fn filtered_runs(&self) -> Vec<&TaskRecord> {
        let query = self.search_query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return self.runs.iter().collect();
        }

        self.runs
            .iter()
            .filter(|run| run_matches_query(run, &query))
            .collect()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let traffic_lights = cfg!(target_os = "macos") && !window.is_fullscreen();
        let header_height = platform_title_bar_height(window);
        let has_query = !self.filter_editor.read(cx).text(cx).is_empty();
        let filtered_runs = self.filtered_runs();
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
        } else if filtered_runs.is_empty() {
            Label::new("No workflow runs match your search.")
                .color(Color::Muted)
                .into_any_element()
        } else {
            let groups = grouped_runs_from_records(filtered_runs);
            v_flex()
                .gap_2()
                .child(self.render_group("Running / queued", &groups.active, 0, cx))
                .child(self.render_group("Completed", &groups.completed, 1, cx))
                .child(self.render_group("Failed", &groups.failed, 2, cx))
                .into_any_element()
        };

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(header_height)
                    .mt_px()
                    .pb_px()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .when(traffic_lights, |this| {
                        this.pl(px(ui::utils::TRAFFIC_LIGHT_PADDING))
                    })
                    .pr_1p5()
                    .gap_1()
                    .child(Divider::vertical().color(ui::DividerColor::Border))
                    .child(
                        h_flex()
                            .ml_1()
                            .min_w_0()
                            .w_full()
                            .gap_1()
                            .child(
                                Icon::new(IconName::MagnifyingGlass)
                                    .size(IconSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(self.filter_editor.clone()),
                    )
                    .when(has_query, |this| {
                        this.child(
                            IconButton::new("clear_runs_filter", IconName::Close)
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Clear Search"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_search(window, cx);
                                })),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                workflow_toolbar_icon_button(
                                    "refresh-workflow-runs",
                                    IconName::ArrowCircle,
                                )
                                .on_click(cx.listener(
                                    |this, _, _window, cx| {
                                        this.refresh(cx);
                                    },
                                )),
                            )
                            .child(
                                workflow_toolbar_icon_button("new-workflow-run", IconName::Plus)
                                    .on_click(cx.listener(|_this, _, window, cx| {
                                        window.dispatch_action(Box::new(OpenWorkflowPicker), cx);
                                    })),
                            ),
                    ),
            )
            .child(content)
    }
}

fn grouped_runs_from_records(runs: Vec<&TaskRecord>) -> RunGroups {
    let mut groups = RunGroups {
        active: Vec::new(),
        completed: Vec::new(),
        failed: Vec::new(),
    };

    for run in runs {
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

fn run_matches_query(run: &TaskRecord, query: &str) -> bool {
    run.title.to_ascii_lowercase().contains(query)
        || run.source_repo.to_ascii_lowercase().contains(query)
        || run
            .status
            .display_name()
            .to_ascii_lowercase()
            .contains(query)
        || run
            .task_description
            .as_ref()
            .is_some_and(|task_description| task_description.to_ascii_lowercase().contains(query))
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
                match client.get_task_status(task_id).await {
                    Ok(run) => {
                        workspace_handle
                            .update_in(cx, |workspace, window, cx| {
                                open_run(run, client.clone(), workspace, window, cx);
                            })
                            .log_err();
                    }
                    Err(error) => {
                        workspace_handle
                            .update_in(cx, |workspace, _window, cx| {
                                workspace.show_toast(
                                    Toast::new(
                                        NotificationId::composite::<OpenWorkflowRunErrorToast>(
                                            task_id.to_string(),
                                        ),
                                        format!("Failed to open run: {error}"),
                                    )
                                    .autohide(),
                                    cx,
                                );
                            })
                            .log_err();
                    }
                }
            })
            .detach();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_test(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme::init(theme::LoadThemes::JustBase, cx);
        });
    }

    fn sample_run(title: &str, source_repo: &str, status: TaskLifecycleStatus) -> TaskRecord {
        TaskRecord {
            id: Uuid::new_v4(),
            title: title.to_string(),
            source_repo: source_repo.to_string(),
            status,
            workflow_id: None,
            task_description: None,
        }
    }

    #[gpui::test]
    async fn workflow_runs_view_filters_runs_by_search_query(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = WorkflowRunsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            );
            view.runs = vec![
                sample_run("Deploy release", "neo-zed", TaskLifecycleStatus::Running),
                sample_run("Triage failures", "zed", TaskLifecycleStatus::Failed),
            ];
            view
        });

        view.update(cx, |view, _cx| {
            view.set_search_query("zed");

            let filtered = view
                .filtered_runs()
                .into_iter()
                .map(|run| run.title.clone())
                .collect::<Vec<_>>();

            assert_eq!(
                filtered,
                vec!["Deploy release".to_string(), "Triage failures".to_string()]
            );

            view.set_search_query("failed");

            let filtered = view
                .filtered_runs()
                .into_iter()
                .map(|run| run.title.clone())
                .collect::<Vec<_>>();

            assert_eq!(filtered, vec!["Triage failures".to_string()]);
        });
    }
}
