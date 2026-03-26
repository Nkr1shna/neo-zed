use crate::canvas::open_workflow;
use crate::client::{
    NodePolicy, RetryBehavior, WORKFLOW_GLOBALS_NODE_TYPE_ID, WorkflowClient,
    WorkflowComparisonOperator, WorkflowConditionGroup, WorkflowConditionGroupMode,
    WorkflowConditionNode, WorkflowConditionPredicate, WorkflowConditionalBranch,
    WorkflowConditionalBranchKind, WorkflowConditionalConfiguration, WorkflowDefinitionRecord,
    WorkflowDefinitionRequest, WorkflowGlobalVariable, WorkflowGlobalsConfiguration,
    WorkflowNodeField, WorkflowNodeFieldType, WorkflowNodePrimitive, WorkflowNodeType,
    WorkflowValueReference, WorkflowValueType, conditional_configuration_from_value,
    editor_node_types, globals_configuration_from_value,
};
use crate::workflow_toolbar_icon_button;
use editor::Editor;
use gpui::{
    App, Context, Corner, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Global,
    IntoElement, Render, Subscription, Task, Window,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use ui::{
    ContextMenu, Divider, DropdownMenu, DropdownStyle, ListItem, ListItemSpacing, Tooltip,
    prelude::*, utils::platform_title_bar_height,
};
use uuid::Uuid;
use workspace::dock::{DockPosition, PanelEvent};
use workspace::{Panel, Workspace};

#[cfg(test)]
use crate::client::{
    WORKFLOW_CONDITIONAL_NODE_TYPE_ID, WORKFLOW_EXECUTE_SHELL_COMMAND_NODE_TYPE_ID,
};

struct WorkflowDefsCache(Entity<WorkflowDefsCacheStore>);

impl Global for WorkflowDefsCache {}

struct WorkflowDefsCacheStore {
    workflows: Vec<WorkflowDefinitionRecord>,
}

impl WorkflowDefsCacheStore {
    fn new() -> Self {
        Self { workflows: vec![] }
    }

    fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<WorkflowDefsCache>()
            .map(|cache| cache.0.clone())
    }

    fn global_or_init(cx: &mut App) -> Entity<Self> {
        if let Some(cache) = Self::global(cx) {
            return cache;
        }

        let cache = cx.new(|_| Self::new());
        cx.set_global(WorkflowDefsCache(cache.clone()));
        cache
    }

    fn replace_all(&mut self, workflows: Vec<WorkflowDefinitionRecord>, cx: &mut Context<Self>) {
        self.workflows = workflows;
        cx.notify();
    }

    fn upsert_workflow(&mut self, workflow: WorkflowDefinitionRecord, cx: &mut Context<Self>) {
        if let Some(existing_workflow) = self
            .workflows
            .iter_mut()
            .find(|existing_workflow| existing_workflow.id == workflow.id)
        {
            *existing_workflow = workflow;
        } else {
            self.workflows.push(workflow);
        }
        cx.notify();
    }

    fn remove_workflow(&mut self, workflow_id: Uuid, cx: &mut Context<Self>) {
        self.workflows.retain(|workflow| workflow.id != workflow_id);
        cx.notify();
    }
}

pub(crate) fn replace_workflow_defs_cache(workflows: Vec<WorkflowDefinitionRecord>, cx: &mut App) {
    let cache = WorkflowDefsCacheStore::global_or_init(cx);
    cache.update(cx, |cache, cx| {
        cache.replace_all(workflows, cx);
    });
}

pub(crate) fn upsert_workflow_def_cache(workflow: WorkflowDefinitionRecord, cx: &mut App) {
    let cache = WorkflowDefsCacheStore::global_or_init(cx);
    cache.update(cx, |cache, cx| {
        cache.upsert_workflow(workflow, cx);
    });
}

pub(crate) fn remove_workflow_def_cache(workflow_id: Uuid, cx: &mut App) {
    let cache = WorkflowDefsCacheStore::global_or_init(cx);
    cache.update(cx, |cache, cx| {
        cache.remove_workflow(workflow_id, cx);
    });
}

#[derive(Clone, Debug, PartialEq, gpui::Action, serde::Deserialize, schemars::JsonSchema)]
pub struct OpenWorkflowDef {
    pub id: String,
}

gpui::actions!(
    workflow_ui,
    [
        NewWorkflow,
        ToggleNodeInspector,
        PublishWorkflow,
        SaveWorkflowDraft
    ]
);

pub struct WorkflowDefsView {
    workflows: Vec<WorkflowDefinitionRecord>,
    search_query: String,
    filter_editor: Entity<Editor>,
    loading: bool,
    error: Option<String>,
    client: Arc<WorkflowClient>,
    context_menu: Option<(
        gpui::Entity<ContextMenu>,
        gpui::Point<gpui::Pixels>,
        Subscription,
    )>,
    renaming_workflow_id: Option<Uuid>,
    rename_editor: gpui::Entity<Editor>,
    pending_focus_rename_editor: bool,
    _subscriptions: Vec<Subscription>,
    _fetch_task: Option<Task<()>>,
    _mutation_task: Option<Task<()>>,
}

impl WorkflowDefsView {
    pub fn new(client: Arc<WorkflowClient>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            workflows: vec![],
            search_query: String::new(),
            filter_editor: cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text("Search workflows…", window, cx);
                editor
            }),
            loading: true,
            error: None,
            client,
            context_menu: None,
            renaming_workflow_id: None,
            rename_editor: cx.new(|cx| Editor::single_line(window, cx)),
            pending_focus_rename_editor: false,
            _subscriptions: vec![],
            _fetch_task: None,
            _mutation_task: None,
        };
        view.bind_filter_editor(cx);
        view.bind_rename_editor(cx);
        view.bind_cache(cx);
        view.fetch(cx);
        view
    }

    #[cfg(test)]
    fn new_for_test(
        client: Arc<WorkflowClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            workflows: vec![],
            search_query: String::new(),
            filter_editor: cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text("Search workflows…", window, cx);
                editor
            }),
            loading: false,
            error: None,
            client,
            context_menu: None,
            renaming_workflow_id: None,
            rename_editor: cx.new(|cx| Editor::single_line(window, cx)),
            pending_focus_rename_editor: false,
            _subscriptions: vec![],
            _fetch_task: None,
            _mutation_task: None,
        };
        view.bind_filter_editor(cx);
        view.bind_rename_editor(cx);
        view.bind_cache(cx);
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

    fn bind_rename_editor(&mut self, cx: &mut Context<Self>) {
        let rename_editor = self.rename_editor.clone();
        self._subscriptions.push(
            cx.subscribe(
                &rename_editor,
                |view: &mut Self, _, event, cx| match event {
                    editor::EditorEvent::BufferEdited => {
                        if view.error.is_some() {
                            view.error = None;
                            cx.notify();
                        }
                    }
                    editor::EditorEvent::Blurred => {
                        if view.renaming_workflow_id.is_some() {
                            view.commit_rename(cx);
                        }
                    }
                    _ => {}
                },
            ),
        );
    }

    fn bind_cache(&mut self, cx: &mut Context<Self>) {
        let cache = WorkflowDefsCacheStore::global_or_init(cx);
        self.workflows = cache.read(cx).workflows.clone();
        if !self.workflows.is_empty() {
            self.loading = false;
        }
        self._subscriptions
            .push(cx.observe(&cache, |view, cache, cx| {
                view.workflows = cache.read(cx).workflows.clone();
                view.loading = false;
                view.error = None;
                cx.notify();
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

    pub fn is_renaming(&self) -> bool {
        self.renaming_workflow_id.is_some()
    }

    pub fn rename_editor_is_focused(&self, window: &Window, cx: &App) -> bool {
        self.rename_editor.read(cx).is_focused(window)
    }

    fn filtered_workflows(&self) -> Vec<&WorkflowDefinitionRecord> {
        let query = self.search_query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return self.workflows.iter().collect();
        }

        self.workflows
            .iter()
            .filter(|workflow| workflow_matches_query(workflow, &query))
            .collect()
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        self._fetch_task = Some(cx.spawn(async move |this, cx| {
            let result = client.list_workflows().await;
            match result {
                Ok(workflows) => {
                    let fetched_workflows = workflows.clone();
                    this.update(cx, |view, cx| {
                        view.loading = false;
                        view.workflows = fetched_workflows;
                        cx.notify();
                    })
                    .ok();
                    cx.update(|cx| replace_workflow_defs_cache(workflows, cx));
                }
                Err(error) => {
                    this.update(cx, |view, cx| {
                        view.loading = false;
                        view.error = Some(error.to_string());
                        cx.notify();
                    })
                    .ok();
                }
            }
        }));
    }

    fn begin_rename_workflow(
        &mut self,
        workflow_id: Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workflow) = self
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
        else {
            return;
        };
        self.context_menu.take();
        self.renaming_workflow_id = Some(workflow_id);
        self.pending_focus_rename_editor = true;
        self.rename_editor.update(cx, |editor, cx| {
            editor.set_text(workflow.name.clone(), window, cx);
            window.focus(&editor.focus_handle(cx), cx);
        });
        cx.notify();
        cx.on_next_frame(window, |_, window, cx| {
            cx.on_next_frame(window, |view, window, cx| {
                if !view.pending_focus_rename_editor {
                    return;
                }

                view.pending_focus_rename_editor = false;
                view.rename_editor.update(cx, |editor, cx| {
                    window.focus(&editor.focus_handle(cx), cx);
                });
            });
        });
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.renaming_workflow_id = None;
        self.pending_focus_rename_editor = false;
        cx.notify();
    }

    pub fn confirm_rename(&mut self, cx: &mut Context<Self>) {
        self.commit_rename(cx);
    }

    pub fn cancel_active_rename(&mut self, cx: &mut Context<Self>) {
        self.cancel_rename(cx);
    }

    fn replace_workflow(&mut self, workflow: WorkflowDefinitionRecord) {
        if let Some(existing_workflow) = self
            .workflows
            .iter_mut()
            .find(|existing_workflow| existing_workflow.id == workflow.id)
        {
            *existing_workflow = workflow;
        }
    }

    fn remove_workflow(&mut self, workflow_id: Uuid) {
        self.workflows.retain(|workflow| workflow.id != workflow_id);
        if self.renaming_workflow_id == Some(workflow_id) {
            self.renaming_workflow_id = None;
        }
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(workflow_id) = self.renaming_workflow_id.take() else {
            return;
        };
        let new_name = self.rename_editor.read(cx).text(cx).trim().to_string();
        self.pending_focus_rename_editor = false;
        self.context_menu.take();
        if new_name.is_empty() {
            cx.notify();
            return;
        }

        let Some(existing_workflow) = self
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .cloned()
        else {
            return;
        };

        if existing_workflow.name == new_name {
            cx.notify();
            return;
        }

        self.error = None;
        let client = self.client.clone();
        let mut request = existing_workflow.to_request();
        request.name = new_name;
        cx.notify();

        self._mutation_task = Some(cx.spawn(async move |this, cx| {
            let result = client.update_workflow(workflow_id, &request).await;
            match result {
                Ok(workflow) => {
                    let saved_workflow = workflow.clone();
                    this.update(cx, |view, cx| {
                        view.replace_workflow(saved_workflow);
                        cx.notify();
                    })
                    .ok();
                    cx.update(|cx| upsert_workflow_def_cache(workflow, cx));
                }
                Err(error) => {
                    this.update(cx, |view, cx| {
                        view.error = Some(error.to_string());
                        cx.notify();
                    })
                    .ok();
                }
            }
        }));
    }

    fn delete_workflow(&mut self, workflow_id: Uuid, cx: &mut Context<Self>) {
        self.context_menu.take();
        self.error = None;
        let client = self.client.clone();
        self._mutation_task = Some(cx.spawn(async move |this, cx| {
            let result = client.delete_workflow(workflow_id).await;
            match result {
                Ok(()) => {
                    this.update(cx, |view, cx| {
                        view.remove_workflow(workflow_id);
                        cx.notify();
                    })
                    .ok();
                    cx.update(|cx| remove_workflow_def_cache(workflow_id, cx));
                }
                Err(error) => {
                    this.update(cx, |view, cx| {
                        view.error = Some(error.to_string());
                        cx.notify();
                    })
                    .ok();
                }
            }
        }));
    }

    fn deploy_workflow_context_menu(
        &mut self,
        workflow_id: Uuid,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu.take();
        let this = cx.weak_entity();
        let context_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            menu.entry("Rename workflow", None, {
                let this = this.clone();
                move |window, cx| {
                    this.update(cx, |view, cx| {
                        view.begin_rename_workflow(workflow_id, window, cx);
                    })
                    .ok();
                }
            })
            .entry("Delete workflow", None, {
                let this = this.clone();
                move |_window, cx| {
                    this.update(cx, |view, cx| {
                        view.delete_workflow(workflow_id, cx);
                    })
                    .ok();
                }
            })
        });
        window.focus(&context_menu.focus_handle(cx), cx);
        let subscription = cx.subscribe(&context_menu, |this, _, _: &DismissEvent, cx| {
            this.context_menu.take();
            cx.notify();
        });
        self.context_menu = Some((context_menu, position, subscription));
    }
}

impl Render for WorkflowDefsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let traffic_lights = cfg!(target_os = "macos") && !_window.is_fullscreen();
        let header_height = platform_title_bar_height(_window);
        let has_query = !self.filter_editor.read(cx).text(cx).is_empty();
        let workflows = self.filtered_workflows();
        let error_banner = (!self.loading && !self.workflows.is_empty())
            .then(|| self.error.clone())
            .flatten();
        let content: gpui::AnyElement = if self.loading {
            Label::new("Loading...")
                .color(Color::Muted)
                .into_any_element()
        } else if self.workflows.is_empty() {
            if let Some(ref error) = self.error {
                Label::new(error.clone())
                    .color(Color::Error)
                    .into_any_element()
            } else {
                Label::new("No workflows")
                    .color(Color::Muted)
                    .into_any_element()
            }
        } else if workflows.is_empty() {
            Label::new("No workflows match your search.")
                .color(Color::Muted)
                .into_any_element()
        } else {
            v_flex()
                .children(workflows.into_iter().enumerate().map(|(index, workflow)| {
                    let item_colors = workflow_defs_item_colors(cx);
                    let is_renaming = self.renaming_workflow_id == Some(workflow.id);
                    let border_color = if is_renaming {
                        item_colors.focused
                    } else {
                        item_colors.default
                    };
                    let workflow_id = workflow.id.to_string();
                    let workflow_uuid = workflow.id;
                    let name = workflow.name.clone();
                    div()
                        .bg(item_colors.default)
                        .border_1()
                        .border_r_2()
                        .border_color(border_color)
                        .hover(|style| {
                            style.bg(item_colors.hover).border_color(if is_renaming {
                                item_colors.focused
                            } else {
                                item_colors.hover
                            })
                        })
                        .child(
                            ListItem::new(index)
                                .spacing(ListItemSpacing::ExtraDense)
                                .selectable(false)
                                .start_slot(
                                    Icon::new(IconName::WorkflowDefs)
                                        .size(IconSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(if is_renaming {
                                    h_flex()
                                        .h_6()
                                        .w_full()
                                        .child(self.rename_editor.clone())
                                        .into_any_element()
                                } else {
                                    Label::new(name).single_line().into_any_element()
                                })
                                .when(!is_renaming, |this| {
                                    this.on_click(move |_, window: &mut Window, cx: &mut App| {
                                        window.dispatch_action(
                                            Box::new(OpenWorkflowDef {
                                                id: workflow_id.clone(),
                                            }),
                                            cx,
                                        );
                                    })
                                })
                                .on_secondary_mouse_down(cx.listener(
                                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.deploy_workflow_context_menu(
                                            workflow_uuid,
                                            event.position,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                        )
                }))
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
                            IconButton::new("clear_workflows_filter", IconName::Close)
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
                                    "refresh-workflows",
                                    IconName::ArrowCircle,
                                )
                                .on_click(cx.listener(
                                    |this, _, _window, cx| {
                                        this.refresh(cx);
                                    },
                                )),
                            )
                            .child(
                                workflow_toolbar_icon_button("new-workflow", IconName::Plus)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(NewWorkflow), cx);
                                    }),
                            ),
                    ),
            )
            .when_some(error_banner, |this, error| {
                this.child(
                    h_flex()
                        .mx_2()
                        .mt_2()
                        .rounded_sm()
                        .border_1()
                        .border_color(Color::Error.color(cx))
                        .bg(cx.theme().colors().panel_background)
                        .px_2()
                        .py_1()
                        .child(Label::new(error).color(Color::Error).size(LabelSize::Small)),
                )
            })
            .child(content)
            .children(self.context_menu.as_ref().map(|(menu, position, _)| {
                gpui::deferred(
                    gpui::anchored()
                        .position(*position)
                        .anchor(Corner::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

fn workflow_matches_query(workflow: &WorkflowDefinitionRecord, query: &str) -> bool {
    workflow.name.to_ascii_lowercase().contains(query)
        || workflow.nodes.iter().any(|node| {
            node.label.to_ascii_lowercase().contains(query)
                || node.node_type.to_ascii_lowercase().contains(query)
        })
        || workflow.trigger_metadata.iter().any(|(key, value)| {
            key.to_ascii_lowercase().contains(query) || value.to_ascii_lowercase().contains(query)
        })
        || workflow
            .validation_policy_ref
            .as_ref()
            .is_some_and(|validation_policy| validation_policy.to_ascii_lowercase().contains(query))
}

struct WorkflowDefsItemColors {
    default: gpui::Hsla,
    hover: gpui::Hsla,
    focused: gpui::Hsla,
}

fn workflow_defs_item_colors(cx: &App) -> WorkflowDefsItemColors {
    let colors = cx.theme().colors();

    WorkflowDefsItemColors {
        default: colors.panel_background,
        hover: colors.element_hover,
        focused: colors.panel_focused_border,
    }
}

enum SaveState {
    Idle,
    Saving,
    Success,
    Error(String),
}

struct WorkflowNodeFieldEditor {
    field: WorkflowNodeField,
    editor: gpui::Entity<Editor>,
}

struct WorkflowConditionalEditorState {
    configuration: WorkflowConditionalConfiguration,
    rhs_editors: BTreeMap<String, gpui::Entity<Editor>>,
}

struct WorkflowGlobalsEditorState {
    configuration: WorkflowGlobalsConfiguration,
    key_editors: Vec<gpui::Entity<Editor>>,
    value_editors: Vec<gpui::Entity<Editor>>,
}

enum ParsedFieldUpdate {
    Set(serde_json::Value),
    Clear,
    KeepExisting,
}

fn displayed_field_value(field: &WorkflowNodeField, value: Option<&serde_json::Value>) -> String {
    let value = value.or(field.default_value.as_ref());
    match (field.field_type, value) {
        (_, None) => String::new(),
        (
            WorkflowNodeFieldType::String
            | WorkflowNodeFieldType::Enum
            | WorkflowNodeFieldType::Workspace
            | WorkflowNodeFieldType::Repo
            | WorkflowNodeFieldType::Artifact,
            Some(value),
        ) => value
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string()),
        (WorkflowNodeFieldType::Number, Some(value)) => value.to_string(),
        (WorkflowNodeFieldType::Boolean, Some(value)) => value
            .as_bool()
            .map(|value| value.to_string())
            .unwrap_or_else(|| value.to_string()),
    }
}

fn parse_number_value(field_text: &str) -> Option<serde_json::Value> {
    if let Ok(value) = field_text.parse::<i64>() {
        return Some(serde_json::json!(value));
    }
    if let Ok(value) = field_text.parse::<u64>() {
        return Some(serde_json::json!(value));
    }
    let Ok(value) = field_text.parse::<f64>() else {
        return None;
    };
    serde_json::Number::from_f64(value).map(serde_json::Value::Number)
}

fn parse_field_update(field: &WorkflowNodeField, field_text: &str) -> ParsedFieldUpdate {
    if field_text.is_empty() {
        return if let Some(default_value) = field.default_value.clone() {
            ParsedFieldUpdate::Set(default_value)
        } else if field.required {
            ParsedFieldUpdate::KeepExisting
        } else {
            ParsedFieldUpdate::Clear
        };
    }

    match field.field_type {
        WorkflowNodeFieldType::String
        | WorkflowNodeFieldType::Workspace
        | WorkflowNodeFieldType::Repo
        | WorkflowNodeFieldType::Artifact => {
            ParsedFieldUpdate::Set(serde_json::Value::String(field_text.to_string()))
        }
        WorkflowNodeFieldType::Number => parse_number_value(field_text)
            .map(ParsedFieldUpdate::Set)
            .unwrap_or(ParsedFieldUpdate::KeepExisting),
        WorkflowNodeFieldType::Boolean => match field_text.to_ascii_lowercase().as_str() {
            "true" => ParsedFieldUpdate::Set(serde_json::Value::Bool(true)),
            "false" => ParsedFieldUpdate::Set(serde_json::Value::Bool(false)),
            _ => ParsedFieldUpdate::KeepExisting,
        },
        WorkflowNodeFieldType::Enum => {
            if field.options.is_empty()
                || field
                    .options
                    .iter()
                    .any(|option| option.value == field_text)
            {
                ParsedFieldUpdate::Set(serde_json::Value::String(field_text.to_string()))
            } else {
                ParsedFieldUpdate::KeepExisting
            }
        }
    }
}

fn condition_editor_key(branch_index: usize, path: &[usize]) -> String {
    let mut key = format!("branch-{branch_index}");
    for index in path {
        key.push('.');
        key.push_str(&index.to_string());
    }
    key
}

fn build_conditional_editor_state(
    configuration: WorkflowConditionalConfiguration,
    window: &mut Window,
    cx: &mut Context<NodeInspectorPanel>,
) -> WorkflowConditionalEditorState {
    let mut rhs_editors = BTreeMap::new();
    for (branch_index, branch) in configuration.branches.iter().enumerate() {
        if let Some(group) = branch.condition.as_ref() {
            populate_condition_rhs_editors(
                group,
                branch_index,
                &mut Vec::new(),
                &mut rhs_editors,
                window,
                cx,
            );
        }
    }
    WorkflowConditionalEditorState {
        configuration,
        rhs_editors,
    }
}

fn populate_condition_rhs_editors(
    group: &WorkflowConditionGroup,
    branch_index: usize,
    path: &mut Vec<usize>,
    rhs_editors: &mut BTreeMap<String, gpui::Entity<Editor>>,
    window: &mut Window,
    cx: &mut Context<NodeInspectorPanel>,
) {
    for (child_index, child) in group.children.iter().enumerate() {
        path.push(child_index);
        match child {
            WorkflowConditionNode::Predicate(predicate) => {
                let key = condition_editor_key(branch_index, path);
                let editor = cx.new(|cx| Editor::single_line(window, cx));
                editor.update(cx, |editor, cx| {
                    editor.set_text(
                        predicate
                            .rhs
                            .as_ref()
                            .map(display_scalar_json_value)
                            .unwrap_or_default(),
                        window,
                        cx,
                    );
                });
                rhs_editors.insert(key, editor);
            }
            WorkflowConditionNode::Group(group) => {
                populate_condition_rhs_editors(group, branch_index, path, rhs_editors, window, cx);
            }
        }
        path.pop();
    }
}

fn build_globals_editor_state(
    configuration: WorkflowGlobalsConfiguration,
    window: &mut Window,
    cx: &mut Context<NodeInspectorPanel>,
) -> WorkflowGlobalsEditorState {
    let mut key_editors = Vec::with_capacity(configuration.variables.len());
    let mut value_editors = Vec::with_capacity(configuration.variables.len());

    for variable in &configuration.variables {
        let key_editor = cx.new(|cx| Editor::single_line(window, cx));
        key_editor.update(cx, |editor, cx| {
            editor.set_text(variable.key.clone(), window, cx);
        });
        key_editors.push(key_editor);

        let value_editor = cx.new(|cx| Editor::single_line(window, cx));
        value_editor.update(cx, |editor, cx| {
            editor.set_text(
                display_scalar_json_value(&variable.default_value),
                window,
                cx,
            );
        });
        value_editors.push(value_editor);
    }

    WorkflowGlobalsEditorState {
        configuration,
        key_editors,
        value_editors,
    }
}

fn display_scalar_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        _ => value.to_string(),
    }
}

fn parse_scalar_json_value(value_type: WorkflowValueType, text: &str) -> serde_json::Value {
    match value_type {
        WorkflowValueType::String => serde_json::Value::String(text.to_string()),
        WorkflowValueType::Number => {
            parse_number_value(text).unwrap_or_else(|| serde_json::json!(0))
        }
        WorkflowValueType::Boolean => match text.to_ascii_lowercase().as_str() {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => serde_json::Value::Bool(false),
        },
    }
}

fn group_mut_at_path<'a>(
    group: &'a mut WorkflowConditionGroup,
    path: &[usize],
) -> Option<&'a mut WorkflowConditionGroup> {
    let mut current = group;
    for index in path {
        let child = current.children.get_mut(*index)?;
        let WorkflowConditionNode::Group(next) = child else {
            return None;
        };
        current = next;
    }
    Some(current)
}

fn node_mut_at_path<'a>(
    group: &'a mut WorkflowConditionGroup,
    path: &[usize],
) -> Option<&'a mut WorkflowConditionNode> {
    let mut current_group = group;
    for (path_index, index) in path.iter().enumerate() {
        let child = current_group.children.get_mut(*index)?;
        if path_index + 1 == path.len() {
            return Some(child);
        }
        let WorkflowConditionNode::Group(next) = child else {
            return None;
        };
        current_group = next;
    }
    None
}

fn branch_title(index: usize, branch: &WorkflowConditionalBranch) -> SharedString {
    match branch.kind {
        WorkflowConditionalBranchKind::When if index == 0 => "If".into(),
        WorkflowConditionalBranchKind::When => format!("Else If {index}").into(),
        WorkflowConditionalBranchKind::Else => "Else".into(),
    }
}

fn sync_group_predicate_rhs(
    group: &mut WorkflowConditionGroup,
    branch_index: usize,
    path: &mut Vec<usize>,
    rhs_editors: &BTreeMap<String, gpui::Entity<Editor>>,
    cx: &mut Context<NodeInspectorPanel>,
) {
    for (child_index, child) in group.children.iter_mut().enumerate() {
        path.push(child_index);
        match child {
            WorkflowConditionNode::Predicate(predicate) => {
                let key = condition_editor_key(branch_index, path);
                if predicate
                    .operator
                    .is_some_and(|operator| !operator.requires_rhs())
                {
                    predicate.rhs = None;
                } else if let Some(editor) = rhs_editors.get(&key) {
                    let rhs_text = editor.read(cx).text(cx).trim().to_string();
                    if rhs_text.is_empty() {
                        predicate.rhs = None;
                    } else {
                        let value_type = predicate
                            .lhs
                            .as_ref()
                            .map(|lhs| lhs.value_type)
                            .unwrap_or(WorkflowValueType::String);
                        predicate.rhs = Some(parse_scalar_json_value(value_type, &rhs_text));
                    }
                }
            }
            WorkflowConditionNode::Group(group) => {
                sync_group_predicate_rhs(group, branch_index, path, rhs_editors, cx);
            }
        }
        path.pop();
    }
}

pub struct NodeInspectorPanel {
    workflow: Option<WorkflowDefinitionRecord>,
    active_canvas: Option<gpui::WeakEntity<crate::canvas::WorkflowCanvas>>,
    node_types: Vec<WorkflowNodeType>,
    selected_node_id: Option<String>,
    focus_handle: FocusHandle,
    label_editor: gpui::Entity<Editor>,
    configure_time_field_editors: BTreeMap<String, WorkflowNodeFieldEditor>,
    conditional_editor_state: Option<WorkflowConditionalEditorState>,
    globals_editor_state: Option<WorkflowGlobalsEditorState>,
    runtime_fields: Vec<WorkflowNodeField>,
    required_reviews_editor: gpui::Entity<Editor>,
    required_checks_editor: gpui::Entity<Editor>,
    max_attempts_editor: gpui::Entity<Editor>,
    backoff_ms_editor: gpui::Entity<Editor>,
    is_dirty: bool,
    save_state: SaveState,
    client: Arc<WorkflowClient>,
    position: DockPosition,
    _node_types_task: Option<Task<()>>,
    _save_task: Option<Task<()>>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl NodeInspectorPanel {
    pub fn new(client: Arc<WorkflowClient>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut panel = Self::new_internal(client, window, cx);
        panel.start_loading_node_types(window, cx);
        panel
    }

    #[cfg(test)]
    fn new_for_test(
        client: Arc<WorkflowClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_internal(client, window, cx)
    }

    fn new_internal(
        client: Arc<WorkflowClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let label_editor = cx.new(|cx| Editor::single_line(window, cx));
        let required_reviews_editor = cx.new(|cx| Editor::single_line(window, cx));
        let required_checks_editor = cx.new(|cx| Editor::single_line(window, cx));
        let max_attempts_editor = cx.new(|cx| Editor::single_line(window, cx));
        let backoff_ms_editor = cx.new(|cx| Editor::single_line(window, cx));

        Self {
            workflow: None,
            active_canvas: None,
            node_types: Vec::new(),
            selected_node_id: None,
            focus_handle: cx.focus_handle(),
            label_editor,
            configure_time_field_editors: BTreeMap::new(),
            conditional_editor_state: None,
            globals_editor_state: None,
            runtime_fields: Vec::new(),
            required_reviews_editor,
            required_checks_editor,
            max_attempts_editor,
            backoff_ms_editor,
            is_dirty: false,
            save_state: SaveState::Idle,
            client,
            position: DockPosition::Right,
            _node_types_task: None,
            _save_task: None,
            _subscriptions: Vec::new(),
        }
    }

    pub fn set_active_canvas(&mut self, canvas: &gpui::Entity<crate::canvas::WorkflowCanvas>) {
        self.active_canvas = Some(canvas.downgrade());
    }

    pub fn set_workflow(&mut self, workflow: WorkflowDefinitionRecord, cx: &mut Context<Self>) {
        self.workflow = Some(workflow);
        self.selected_node_id = None;
        self.configure_time_field_editors.clear();
        self.conditional_editor_state = None;
        self.globals_editor_state = None;
        self.runtime_fields.clear();
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
            self.configure_time_field_editors.clear();
            self.conditional_editor_state = None;
            self.globals_editor_state = None;
            self.runtime_fields.clear();
            cx.notify();
            return;
        };

        let Some(ref workflow) = self.workflow else {
            self.configure_time_field_editors.clear();
            self.conditional_editor_state = None;
            self.globals_editor_state = None;
            self.runtime_fields.clear();
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

        self.refresh_configure_time_fields(window, cx);

        cx.notify();
    }

    fn default_policy(node_id: String) -> NodePolicy {
        NodePolicy {
            node_id,
            required_reviews: 0,
            required_checks: Vec::new(),
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: None,
        }
    }

    fn start_loading_node_types(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.client.clone();
        self._node_types_task = Some(cx.spawn_in(window, async move |this, cx| {
            let Ok(node_types) = client.list_workflow_node_types().await else {
                return;
            };

            this.update_in(cx, |panel, window, cx| {
                panel.node_types = editor_node_types(node_types);
                panel.refresh_configure_time_fields(window, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn selected_node_type(&self) -> Option<WorkflowNodeType> {
        let selected_node_id = self.selected_node_id.as_ref()?;
        let workflow = self.workflow.as_ref()?;
        let node = workflow
            .nodes
            .iter()
            .find(|node| &node.id == selected_node_id)?;
        self.node_types
            .iter()
            .find(|node_type| node_type.id == node.node_type)
            .cloned()
    }

    fn selected_node_primitive(&self) -> Option<WorkflowNodePrimitive> {
        Some(self.selected_node_type()?.primitive_kind())
    }

    fn refresh_configure_time_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.configure_time_field_editors.clear();
        self.conditional_editor_state = None;
        self.globals_editor_state = None;
        self.runtime_fields.clear();

        let Some(selected_node_id) = self.selected_node_id.as_ref() else {
            return;
        };
        let Some(workflow) = self.workflow.as_ref() else {
            return;
        };
        let Some(node) = workflow
            .nodes
            .iter()
            .find(|node| &node.id == selected_node_id)
        else {
            return;
        };
        let Some(node_type) = self.selected_node_type() else {
            return;
        };

        match node_type.primitive_kind() {
            WorkflowNodePrimitive::Conditional => {
                self.conditional_editor_state = Some(build_conditional_editor_state(
                    conditional_configuration_from_value(&node.configuration),
                    window,
                    cx,
                ));
                return;
            }
            WorkflowNodePrimitive::Globals => {
                self.globals_editor_state = Some(build_globals_editor_state(
                    globals_configuration_from_value(&node.configuration),
                    window,
                    cx,
                ));
                return;
            }
            _ => {}
        }

        for field in node_type.configure_time_fields {
            let editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text(&field.label, window, cx);
                editor
            });
            let initial_text = displayed_field_value(&field, node.configuration.get(&field.key));
            editor.update(cx, |editor, cx| {
                editor.set_text(initial_text.clone(), window, cx);
            });
            self.configure_time_field_editors
                .insert(field.key.clone(), WorkflowNodeFieldEditor { field, editor });
        }
    }

    fn available_value_references(&self) -> Vec<WorkflowValueReference> {
        let Some(selected_node_id) = self.selected_node_id.as_ref() else {
            return Vec::new();
        };
        let Some(workflow) = self.workflow.as_ref() else {
            return Vec::new();
        };

        let mut references = Vec::new();

        if let Some(globals_node) = workflow
            .nodes
            .iter()
            .find(|node| node.node_type == WORKFLOW_GLOBALS_NODE_TYPE_ID)
        {
            let globals = globals_configuration_from_value(&globals_node.configuration);
            references.extend(globals.variables.into_iter().map(|variable| {
                WorkflowValueReference {
                    source: crate::client::WorkflowReferenceSource::Global,
                    node_id: Some(globals_node.id.clone()),
                    path: variable.key.clone(),
                    label: format!("Global: {}", variable.key),
                    value_type: variable.value_type,
                }
            }));
        }

        for edge in workflow
            .edges
            .iter()
            .filter(|edge| &edge.to_node_id == selected_node_id)
        {
            let Some(source_node) = workflow
                .nodes
                .iter()
                .find(|node| node.id == edge.from_node_id)
            else {
                continue;
            };
            let Some(source_type) = self
                .node_types
                .iter()
                .find(|node_type| node_type.id == source_node.node_type)
            else {
                continue;
            };

            if source_type.primitive_kind() == WorkflowNodePrimitive::ExecuteShellCommand {
                references.push(WorkflowValueReference {
                    source: crate::client::WorkflowReferenceSource::Input,
                    node_id: Some(source_node.id.clone()),
                    path: "shell.exit_code".into(),
                    label: "Shell exit code".into(),
                    value_type: WorkflowValueType::Number,
                });
                references.push(WorkflowValueReference {
                    source: crate::client::WorkflowReferenceSource::Input,
                    node_id: Some(source_node.id.clone()),
                    path: "shell.stdout".into(),
                    label: "Standard output".into(),
                    value_type: WorkflowValueType::String,
                });
                references.push(WorkflowValueReference {
                    source: crate::client::WorkflowReferenceSource::Input,
                    node_id: Some(source_node.id.clone()),
                    path: "shell.stderr".into(),
                    label: "Standard error".into(),
                    value_type: WorkflowValueType::String,
                });
            }

            references.extend(source_type.runtime_fields.iter().filter_map(|field| {
                let value_type = match field.field_type {
                    WorkflowNodeFieldType::String => WorkflowValueType::String,
                    WorkflowNodeFieldType::Number => WorkflowValueType::Number,
                    WorkflowNodeFieldType::Boolean => WorkflowValueType::Boolean,
                    _ => return None,
                };
                Some(WorkflowValueReference {
                    source: crate::client::WorkflowReferenceSource::Input,
                    node_id: Some(source_node.id.clone()),
                    path: field.key.clone(),
                    label: field.label.clone(),
                    value_type,
                })
            }));
        }

        references.sort_by(|left, right| left.label.cmp(&right.label));
        references.dedup_by(|left, right| {
            left.source == right.source && left.node_id == right.node_id && left.path == right.path
        });
        references
    }

    fn sync_conditional_editor_state_from_editors(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };

        for (branch_index, branch) in state.configuration.branches.iter_mut().enumerate() {
            let Some(group) = branch.condition.as_mut() else {
                continue;
            };
            sync_group_predicate_rhs(group, branch_index, &mut Vec::new(), &state.rhs_editors, cx);
        }
    }

    fn sync_globals_editor_state_from_editors(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.globals_editor_state.as_mut() else {
            return;
        };

        for (index, variable) in state.configuration.variables.iter_mut().enumerate() {
            if let Some(editor) = state.key_editors.get(index) {
                variable.key = editor.read(cx).text(cx).trim().to_string();
            }
            if let Some(editor) = state.value_editors.get(index) {
                variable.default_value =
                    parse_scalar_json_value(variable.value_type, editor.read(cx).text(cx).trim());
            }
        }
    }

    fn conditional_add_when_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_conditional_editor_state_from_editors(cx);
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        let next_index = state
            .configuration
            .branches
            .iter()
            .filter(|branch| branch.kind == WorkflowConditionalBranchKind::When)
            .count()
            + 1;
        let branch = WorkflowConditionalBranch {
            output_id: format!("if_{next_index}"),
            kind: WorkflowConditionalBranchKind::When,
            condition: Some(WorkflowConditionGroup::default()),
        };
        let insert_index = state
            .configuration
            .branches
            .iter()
            .position(|branch| branch.kind == WorkflowConditionalBranchKind::Else)
            .unwrap_or(state.configuration.branches.len());
        state.configuration.branches.insert(insert_index, branch);
        let configuration = state.configuration.clone();
        self.conditional_editor_state =
            Some(build_conditional_editor_state(configuration, window, cx));
        cx.notify();
    }

    fn conditional_add_else_branch(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        if state
            .configuration
            .branches
            .iter()
            .any(|branch| branch.kind == WorkflowConditionalBranchKind::Else)
        {
            return;
        }
        state
            .configuration
            .branches
            .push(WorkflowConditionalBranch {
                output_id: "else".into(),
                kind: WorkflowConditionalBranchKind::Else,
                condition: None,
            });
        cx.notify();
    }

    fn conditional_add_group(
        &mut self,
        branch_index: usize,
        path: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_conditional_editor_state_from_editors(cx);
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        let Some(branch) = state.configuration.branches.get_mut(branch_index) else {
            return;
        };
        let Some(group) = branch.condition.as_mut() else {
            return;
        };
        if let Some(target_group) = group_mut_at_path(group, &path) {
            target_group.children.push(WorkflowConditionNode::Group(
                WorkflowConditionGroup::default(),
            ));
        }
        let configuration = state.configuration.clone();
        self.conditional_editor_state =
            Some(build_conditional_editor_state(configuration, window, cx));
        cx.notify();
    }

    fn conditional_add_predicate(
        &mut self,
        branch_index: usize,
        path: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_conditional_editor_state_from_editors(cx);
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        let Some(branch) = state.configuration.branches.get_mut(branch_index) else {
            return;
        };
        let Some(group) = branch.condition.as_mut() else {
            return;
        };
        if let Some(target_group) = group_mut_at_path(group, &path) {
            target_group.children.push(WorkflowConditionNode::Predicate(
                WorkflowConditionPredicate::default(),
            ));
        }
        let configuration = state.configuration.clone();
        self.conditional_editor_state =
            Some(build_conditional_editor_state(configuration, window, cx));
        cx.notify();
    }

    fn conditional_set_group_mode(
        &mut self,
        branch_index: usize,
        path: Vec<usize>,
        mode: WorkflowConditionGroupMode,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        let Some(branch) = state.configuration.branches.get_mut(branch_index) else {
            return;
        };
        let Some(group) = branch.condition.as_mut() else {
            return;
        };
        if let Some(target_group) = group_mut_at_path(group, &path) {
            target_group.mode = mode;
            cx.notify();
        }
    }

    fn conditional_set_predicate_lhs(
        &mut self,
        branch_index: usize,
        path: Vec<usize>,
        lhs: WorkflowValueReference,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        let Some(branch) = state.configuration.branches.get_mut(branch_index) else {
            return;
        };
        let Some(group) = branch.condition.as_mut() else {
            return;
        };
        let Some(WorkflowConditionNode::Predicate(predicate)) = node_mut_at_path(group, &path)
        else {
            return;
        };
        predicate.lhs = Some(lhs.clone());
        if predicate.operator.is_some_and(|operator| {
            !WorkflowComparisonOperator::supported_for(lhs.value_type).contains(&operator)
        }) {
            predicate.operator = None;
            predicate.rhs = None;
        }
        cx.notify();
    }

    fn conditional_set_predicate_operator(
        &mut self,
        branch_index: usize,
        path: Vec<usize>,
        operator: WorkflowComparisonOperator,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.conditional_editor_state.as_mut() else {
            return;
        };
        let Some(branch) = state.configuration.branches.get_mut(branch_index) else {
            return;
        };
        let Some(group) = branch.condition.as_mut() else {
            return;
        };
        let Some(WorkflowConditionNode::Predicate(predicate)) = node_mut_at_path(group, &path)
        else {
            return;
        };
        predicate.operator = Some(operator);
        if !operator.requires_rhs() {
            predicate.rhs = None;
        }
        cx.notify();
    }

    fn globals_add_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_globals_editor_state_from_editors(cx);
        let Some(state) = self.globals_editor_state.as_mut() else {
            return;
        };
        state.configuration.variables.push(WorkflowGlobalVariable {
            key: format!("var_{}", state.configuration.variables.len() + 1),
            value_type: WorkflowValueType::String,
            default_value: serde_json::json!(""),
            allow_runtime_override: false,
            allow_task_mutation: false,
        });
        let configuration = state.configuration.clone();
        self.globals_editor_state = Some(build_globals_editor_state(configuration, window, cx));
        cx.notify();
    }

    fn globals_set_variable_type(
        &mut self,
        index: usize,
        value_type: WorkflowValueType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_globals_editor_state_from_editors(cx);
        let Some(state) = self.globals_editor_state.as_mut() else {
            return;
        };
        let Some(variable) = state.configuration.variables.get_mut(index) else {
            return;
        };
        variable.value_type = value_type;
        variable.default_value = match value_type {
            WorkflowValueType::String => serde_json::json!(""),
            WorkflowValueType::Number => serde_json::json!(0),
            WorkflowValueType::Boolean => serde_json::json!(false),
        };
        let configuration = state.configuration.clone();
        self.globals_editor_state = Some(build_globals_editor_state(configuration, window, cx));
        cx.notify();
    }

    fn globals_toggle_runtime_override(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(state) = self.globals_editor_state.as_mut() else {
            return;
        };
        let Some(variable) = state.configuration.variables.get_mut(index) else {
            return;
        };
        variable.allow_runtime_override = !variable.allow_runtime_override;
        cx.notify();
    }

    fn globals_toggle_task_mutation(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(state) = self.globals_editor_state.as_mut() else {
            return;
        };
        let Some(variable) = state.configuration.variables.get_mut(index) else {
            return;
        };
        variable.allow_task_mutation = !variable.allow_task_mutation;
        cx.notify();
    }

    fn apply_pending_node_edits(&mut self, cx: &mut Context<Self>) {
        let Some(selected_node_id) = self.selected_node_id.clone() else {
            return;
        };
        let selected_primitive = self.selected_node_primitive();
        let label = self.label_editor.read(cx).text(cx).trim().to_string();
        let required_reviews_text = self
            .required_reviews_editor
            .read(cx)
            .text(cx)
            .trim()
            .to_string();
        let required_checks_text = self
            .required_checks_editor
            .read(cx)
            .text(cx)
            .trim()
            .to_string();
        let max_attempts_text = self
            .max_attempts_editor
            .read(cx)
            .text(cx)
            .trim()
            .to_string();
        let backoff_ms_text = self.backoff_ms_editor.read(cx).text(cx).trim().to_string();

        match selected_primitive {
            Some(WorkflowNodePrimitive::Conditional) => {
                self.sync_conditional_editor_state_from_editors(cx);
            }
            Some(WorkflowNodePrimitive::Globals) => {
                self.sync_globals_editor_state_from_editors(cx);
            }
            _ => {}
        }

        let Some(workflow) = self.workflow.as_mut() else {
            return;
        };

        if let Some(node) = workflow
            .nodes
            .iter_mut()
            .find(|node| node.id == selected_node_id)
            && !label.is_empty()
        {
            node.label = label;
            match selected_primitive {
                Some(WorkflowNodePrimitive::Conditional) => {
                    if let Some(state) = self.conditional_editor_state.as_ref() {
                        node.configuration = serde_json::to_value(&state.configuration)
                            .unwrap_or_else(|_| serde_json::json!({}));
                    }
                }
                Some(WorkflowNodePrimitive::Globals) => {
                    if let Some(state) = self.globals_editor_state.as_ref() {
                        node.configuration = serde_json::to_value(&state.configuration)
                            .unwrap_or_else(|_| serde_json::json!({}));
                    }
                }
                _ => {
                    let mut configuration = node
                        .configuration
                        .as_object()
                        .cloned()
                        .unwrap_or_else(serde_json::Map::new);
                    for (field_key, field_editor) in &self.configure_time_field_editors {
                        let field_text = field_editor.editor.read(cx).text(cx).trim().to_string();
                        match parse_field_update(&field_editor.field, &field_text) {
                            ParsedFieldUpdate::Set(value) => {
                                configuration.insert(field_key.clone(), value);
                            }
                            ParsedFieldUpdate::Clear => {
                                configuration.remove(field_key);
                            }
                            ParsedFieldUpdate::KeepExisting => {}
                        }
                    }
                    node.configuration = serde_json::Value::Object(configuration);
                }
            }
        }

        let existing_policy = workflow.policy_for(&selected_node_id).cloned();
        let mut policy = existing_policy
            .clone()
            .unwrap_or_else(|| Self::default_policy(selected_node_id.clone()));
        policy.required_reviews = required_reviews_text
            .parse()
            .unwrap_or(policy.required_reviews);
        policy.required_checks = required_checks_text
            .split(',')
            .map(str::trim)
            .filter(|check| !check.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        policy.retry_behavior.max_attempts = max_attempts_text
            .parse()
            .unwrap_or(policy.retry_behavior.max_attempts);
        policy.retry_behavior.backoff_ms = backoff_ms_text
            .parse()
            .unwrap_or(policy.retry_behavior.backoff_ms);

        let should_store_policy = existing_policy.is_some()
            || policy.required_reviews != 0
            || !policy.required_checks.is_empty()
            || policy.retry_behavior.max_attempts != RetryBehavior::default().max_attempts
            || policy.retry_behavior.backoff_ms != RetryBehavior::default().backoff_ms
            || policy.validation_policy_ref.is_some();

        if should_store_policy {
            if let Some(existing_policy) = workflow
                .node_policies
                .iter_mut()
                .find(|existing_policy| existing_policy.node_id == selected_node_id)
            {
                *existing_policy = policy;
            } else {
                workflow.node_policies.push(policy);
            }
        } else {
            workflow
                .node_policies
                .retain(|policy| policy.node_id != selected_node_id);
        }

        let updated_workflow = workflow.clone();
        if let Some(active_canvas) = self.active_canvas.clone() {
            active_canvas
                .update(cx, |canvas, cx| {
                    canvas.workflow = Some(updated_workflow.clone());
                    cx.notify();
                })
                .ok();
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.apply_pending_node_edits(cx);

        let Some(workflow) = self.workflow.clone() else {
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

        self.save_state = SaveState::Saving;
        cx.notify();

        self._save_task = Some(cx.spawn(async move |this, cx| {
            let result = if is_new {
                client.create_workflow(&request).await
            } else {
                client.update_workflow(workflow_id, &request).await
            };

            this.update(cx, |panel, cx| {
                match result {
                    Ok(workflow) => {
                        panel.workflow = Some(workflow.clone());
                        if let Some(active_canvas) = panel.active_canvas.clone() {
                            active_canvas
                                .update(cx, |canvas, cx| {
                                    canvas.workflow = Some(workflow.clone());
                                    cx.notify();
                                })
                                .ok();
                        }
                        panel.is_dirty = false;
                        panel.save_state = SaveState::Success;
                        upsert_workflow_def_cache(workflow, cx);
                    }
                    Err(error) => {
                        panel.save_state = SaveState::Error(error.to_string());
                    }
                }
                cx.notify();
            })
            .ok();

            cx.background_executor().timer(Duration::from_secs(3)).await;

            this.update(cx, |panel, cx| {
                panel.save_state = SaveState::Idle;
                cx.notify();
            })
            .ok();
        }));
    }

    fn render_conditional_editor(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(state) = self.conditional_editor_state.as_ref() else {
            return div().into_any_element();
        };
        let available_refs = self.available_value_references();
        let has_else = state
            .configuration
            .branches
            .iter()
            .any(|branch| branch.kind == WorkflowConditionalBranchKind::Else);

        v_flex()
            .gap_2()
            .child(
                Label::new("Conditional Routing")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(state.configuration.branches.iter().enumerate().map(
                |(branch_index, branch)| {
                    v_flex()
                        .gap_1()
                        .p_2()
                        .border_1()
                        .border_color(cx.theme().colors().border)
                        .rounded_md()
                        .child(
                            h_flex()
                                .justify_between()
                                .child(Label::new(branch_title(branch_index, branch)))
                                .child(
                                    Label::new(format!("Edge output: {}", branch.output_id))
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(if let Some(group) = branch.condition.as_ref() {
                            self.render_condition_group(
                                branch_index,
                                Vec::new(),
                                group,
                                &available_refs,
                                window,
                                cx,
                            )
                        } else {
                            Label::new("Fallback branch")
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .into_any_element()
                        })
                },
            ))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("conditional-add-branch", "Add Else If")
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.conditional_add_when_branch(window, cx);
                            })),
                    )
                    .child(
                        Button::new("conditional-add-else", "Add Else")
                            .style(ButtonStyle::Subtle)
                            .disabled(has_else)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.conditional_add_else_branch(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_condition_group(
        &self,
        branch_index: usize,
        path: Vec<usize>,
        group: &WorkflowConditionGroup,
        available_refs: &[WorkflowValueReference],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mode_entity = cx.weak_entity();
        let mode_path = path.clone();
        let mode_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            menu.entry("All of", None, {
                let this = mode_entity.clone();
                let path = mode_path.clone();
                move |_window, cx| {
                    this.update(cx, |panel, cx| {
                        panel.conditional_set_group_mode(
                            branch_index,
                            path.clone(),
                            WorkflowConditionGroupMode::All,
                            cx,
                        );
                    })
                    .ok();
                }
            })
            .entry("Any of", None, {
                let this = mode_entity.clone();
                let path = mode_path.clone();
                move |_window, cx| {
                    this.update(cx, |panel, cx| {
                        panel.conditional_set_group_mode(
                            branch_index,
                            path.clone(),
                            WorkflowConditionGroupMode::Any,
                            cx,
                        );
                    })
                    .ok();
                }
            })
        });

        v_flex()
            .gap_1()
            .p_1()
            .border_l_2()
            .border_color(cx.theme().colors().border_variant)
            .child(
                DropdownMenu::new(
                    format!(
                        "conditional-group-mode-{branch_index}-{}",
                        condition_editor_key(branch_index, &path)
                    ),
                    group.mode.display_name(),
                    mode_menu,
                )
                .style(DropdownStyle::Outlined)
                .into_any_element(),
            )
            .children(
                group
                    .children
                    .iter()
                    .enumerate()
                    .map(|(child_index, child)| {
                        let mut child_path = path.clone();
                        child_path.push(child_index);
                        match child {
                            WorkflowConditionNode::Predicate(predicate) => self
                                .render_condition_predicate(
                                    branch_index,
                                    child_path,
                                    predicate,
                                    available_refs,
                                    window,
                                    cx,
                                ),
                            WorkflowConditionNode::Group(group) => self.render_condition_group(
                                branch_index,
                                child_path,
                                group,
                                available_refs,
                                window,
                                cx,
                            ),
                        }
                    }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child({
                        let add_predicate_path = path.clone();
                        Button::new(
                            format!(
                                "conditional-add-predicate-{branch_index}-{}",
                                condition_editor_key(branch_index, &path)
                            ),
                            "Add Condition",
                        )
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.conditional_add_predicate(
                                    branch_index,
                                    add_predicate_path.clone(),
                                    window,
                                    cx,
                                );
                            },
                        ))
                    })
                    .child({
                        let add_group_path = path.clone();
                        Button::new(
                            format!(
                                "conditional-add-group-{branch_index}-{}",
                                condition_editor_key(branch_index, &path)
                            ),
                            "Add Group",
                        )
                        .style(ButtonStyle::Subtle)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.conditional_add_group(
                                    branch_index,
                                    add_group_path.clone(),
                                    window,
                                    cx,
                                );
                            },
                        ))
                    }),
            )
            .into_any_element()
    }

    fn render_condition_predicate(
        &self,
        branch_index: usize,
        path: Vec<usize>,
        predicate: &WorkflowConditionPredicate,
        available_refs: &[WorkflowValueReference],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let lhs_entity = cx.weak_entity();
        let lhs_path = path.clone();
        let lhs_label = predicate
            .lhs
            .as_ref()
            .map(|lhs| lhs.label.clone())
            .unwrap_or_else(|| "Select value".into());
        let lhs_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            available_refs
                .iter()
                .cloned()
                .fold(menu, |menu, reference| {
                    let label = reference.label.clone();
                    menu.entry(label, None, {
                        let this = lhs_entity.clone();
                        let path = lhs_path.clone();
                        move |_window, cx| {
                            this.update(cx, |panel, cx| {
                                panel.conditional_set_predicate_lhs(
                                    branch_index,
                                    path.clone(),
                                    reference.clone(),
                                    cx,
                                );
                            })
                            .ok();
                        }
                    })
                })
        });

        let supported_operators = predicate
            .lhs
            .as_ref()
            .map(|lhs| WorkflowComparisonOperator::supported_for(lhs.value_type))
            .unwrap_or(&[]);
        let operator_label = predicate
            .operator
            .map(|operator| operator.display_name())
            .unwrap_or("Select comparison");
        let operator_entity = cx.weak_entity();
        let operator_path = path.clone();
        let operator_menu = ContextMenu::build(window, cx, move |menu, _window, _cx| {
            supported_operators
                .iter()
                .copied()
                .fold(menu, |menu, operator| {
                    menu.entry(operator.display_name(), None, {
                        let this = operator_entity.clone();
                        let path = operator_path.clone();
                        move |_window, cx| {
                            this.update(cx, |panel, cx| {
                                panel.conditional_set_predicate_operator(
                                    branch_index,
                                    path.clone(),
                                    operator,
                                    cx,
                                );
                            })
                            .ok();
                        }
                    })
                })
        });

        let rhs_editor = self
            .conditional_editor_state
            .as_ref()
            .and_then(|state| {
                state
                    .rhs_editors
                    .get(&condition_editor_key(branch_index, &path))
            })
            .cloned();

        h_flex()
            .gap_1()
            .items_center()
            .child(
                DropdownMenu::new(
                    format!(
                        "conditional-lhs-{branch_index}-{}",
                        condition_editor_key(branch_index, &path)
                    ),
                    lhs_label,
                    lhs_menu,
                )
                .style(DropdownStyle::Outlined)
                .into_any_element(),
            )
            .child(
                DropdownMenu::new(
                    format!(
                        "conditional-operator-{branch_index}-{}",
                        condition_editor_key(branch_index, &path)
                    ),
                    operator_label,
                    operator_menu,
                )
                .style(DropdownStyle::Outlined)
                .disabled(predicate.lhs.is_none())
                .into_any_element(),
            )
            .when(
                predicate
                    .operator
                    .is_some_and(|operator| operator.requires_rhs()),
                |this| this.child(rhs_editor.expect("rhs editor must exist when rhs is required")),
            )
            .into_any_element()
    }

    fn render_globals_editor(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(state) = self.globals_editor_state.as_ref() else {
            return div().into_any_element();
        };

        v_flex()
            .gap_2()
            .child(
                Label::new("Workflow Globals")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(
                state
                    .configuration
                    .variables
                    .iter()
                    .enumerate()
                    .map(|(index, variable)| {
                        let this = cx.weak_entity();
                        let type_menu =
                            ContextMenu::build(window, cx, move |menu, _window, _cx| {
                                [
                                    WorkflowValueType::String,
                                    WorkflowValueType::Number,
                                    WorkflowValueType::Boolean,
                                ]
                                .into_iter()
                                .fold(menu, |menu, value_type| {
                                    menu.entry(value_type.display_name(), None, {
                                        let this = this.clone();
                                        move |window, cx| {
                                            this.update(cx, |panel, cx| {
                                                panel.globals_set_variable_type(
                                                    index, value_type, window, cx,
                                                );
                                            })
                                            .ok();
                                        }
                                    })
                                })
                            });

                        v_flex()
                            .gap_1()
                            .p_2()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(state.key_editors[index].clone())
                                    .child(
                                        DropdownMenu::new(
                                            format!("globals-type-{index}"),
                                            variable.value_type.display_name(),
                                            type_menu,
                                        )
                                        .style(DropdownStyle::Outlined),
                                    ),
                            )
                            .child(state.value_editors[index].clone())
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new(
                                            format!("globals-runtime-override-{index}"),
                                            if variable.allow_runtime_override {
                                                "Runtime Override: On"
                                            } else {
                                                "Runtime Override: Off"
                                            },
                                        )
                                        .style(ButtonStyle::Subtle)
                                        .on_click(
                                            cx.listener(move |this, _, _window, cx| {
                                                this.globals_toggle_runtime_override(index, cx);
                                            }),
                                        ),
                                    )
                                    .child(
                                        Button::new(
                                            format!("globals-task-mutation-{index}"),
                                            if variable.allow_task_mutation {
                                                "Task Mutation: On"
                                            } else {
                                                "Task Mutation: Off"
                                            },
                                        )
                                        .style(ButtonStyle::Subtle)
                                        .on_click(
                                            cx.listener(move |this, _, _window, cx| {
                                                this.globals_toggle_task_mutation(index, cx);
                                            }),
                                        ),
                                    ),
                            )
                            .into_any_element()
                    }),
            )
            .child(
                Button::new("globals-add-variable", "Add Variable")
                    .style(ButtonStyle::Subtle)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.globals_add_variable(window, cx);
                    })),
            )
            .into_any_element()
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

    fn default_size(&self, _window: &Window, _cx: &App) -> Pixels {
        px(320.)
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
        4
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
            let selected_node_type = self.selected_node_type();
            let save_label: SharedString = match &self.save_state {
                SaveState::Idle => "Save".into(),
                SaveState::Saving => "Saving...".into(),
                SaveState::Success => "Saved!".into(),
                SaveState::Error(_) => "Error".into(),
            };

            let save_color = match &self.save_state {
                SaveState::Success => Color::Success,
                SaveState::Error(_) => Color::Error,
                _ => Color::Default,
            };

            let error_message: Option<gpui::AnyElement> =
                if let SaveState::Error(ref message) = self.save_state {
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
                        .child(
                            Label::new("Label")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(self.label_editor.clone()),
                )
                .when_some(selected_node_type.as_ref(), |this, node_type| {
                    this.child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                Label::new("Node Definition")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(Label::new(node_type.label.clone()))
                            .child(
                                Label::new(node_type.primitive_kind().display_name())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                })
                .when(self.conditional_editor_state.is_some(), |this| {
                    this.child(self.render_conditional_editor(_window, cx))
                })
                .when(self.globals_editor_state.is_some(), |this| {
                    this.child(self.render_globals_editor(_window, cx))
                })
                .when(!self.configure_time_field_editors.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .gap_1()
                            .child(
                                Label::new("Configure-Time Fields")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .children(self.configure_time_field_editors.values().map(
                                |field_editor| {
                                    let field_label = if field_editor.field.required {
                                        format!(
                                            "{} ({})*",
                                            field_editor.field.label,
                                            field_editor.field.field_type.display_name()
                                        )
                                    } else {
                                        format!(
                                            "{} ({})",
                                            field_editor.field.label,
                                            field_editor.field.field_type.display_name()
                                        )
                                    };
                                    v_flex()
                                        .gap_1()
                                        .child(
                                            Label::new(field_label)
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        )
                                        .child(field_editor.editor.clone())
                                },
                            )),
                    )
                })
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
                        Button::new("save-workflow", save_label)
                            .color(save_color)
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.save(cx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::WorkflowCanvas;

    fn init_test(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme::init(theme::LoadThemes::JustBase, cx);
        });
    }

    fn sample_workflow() -> WorkflowDefinitionRecord {
        WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Workflow".into(),
            nodes: vec![crate::client::WorkflowNode {
                id: "node-1".into(),
                node_type: "summarize".into(),
                label: "Default Label".into(),
                configuration: serde_json::json!({
                    "model": "gpt-5.1",
                    "stream": false,
                }),
                runtime: serde_json::json!({}),
            }],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: BTreeMap::new(),
        }
    }

    fn sample_sidebar_workflows() -> Vec<WorkflowDefinitionRecord> {
        vec![
            WorkflowDefinitionRecord {
                id: Uuid::new_v4(),
                name: "Alpha".into(),
                nodes: vec![],
                edges: vec![],
                node_policies: vec![],
                retry_behavior: RetryBehavior::default(),
                validation_policy_ref: None,
                trigger_metadata: BTreeMap::new(),
            },
            WorkflowDefinitionRecord {
                id: Uuid::new_v4(),
                name: "Beta".into(),
                nodes: vec![],
                edges: vec![],
                node_policies: vec![],
                retry_behavior: RetryBehavior::default(),
                validation_policy_ref: None,
                trigger_metadata: BTreeMap::new(),
            },
        ]
    }

    fn conditional_node_type() -> crate::client::WorkflowNodeType {
        crate::client::WorkflowNodeType {
            id: WORKFLOW_CONDITIONAL_NODE_TYPE_ID.into(),
            label: "Conditional".into(),
            primitive: Some(crate::client::WorkflowNodePrimitive::Conditional),
            category: None,
            is_primitive: true,
            inputs: vec![crate::client::WorkflowNodePort {
                id: "default".into(),
                label: "Input".into(),
            }],
            outputs: vec![crate::client::WorkflowNodePort {
                id: "if_1".into(),
                label: "If".into(),
            }],
            configure_time_fields: vec![],
            runtime_fields: vec![],
        }
    }

    #[gpui::test]
    async fn test_apply_pending_node_edits_updates_panel_and_canvas_workflow(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx);

        let workflow = sample_workflow();
        let canvas_holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let canvas_holder_for_view = canvas_holder.clone();

        let (panel, cx) = cx.add_window_view(|window, cx| {
            let canvas = cx.new(|cx| {
                WorkflowCanvas::new_edit_for_test(workflow.clone(), WorkflowClient::new(), cx)
            });
            *canvas_holder_for_view.borrow_mut() = Some(canvas.clone());

            let mut panel = NodeInspectorPanel::new_for_test(WorkflowClient::new(), window, cx);
            panel.node_types = vec![crate::client::WorkflowNodeType {
                id: "summarize".into(),
                label: "Summarize".into(),
                primitive: Some(crate::client::WorkflowNodePrimitive::Llm),
                category: None,
                is_primitive: false,
                inputs: vec![],
                outputs: vec![],
                configure_time_fields: vec![
                    crate::client::WorkflowNodeField {
                        key: "model".into(),
                        label: "Model".into(),
                        field_type: crate::client::WorkflowNodeFieldType::String,
                        required: true,
                        default_value: Some(serde_json::json!("gpt-5.1")),
                        options: vec![],
                    },
                    crate::client::WorkflowNodeField {
                        key: "max_tokens".into(),
                        label: "Max Tokens".into(),
                        field_type: crate::client::WorkflowNodeFieldType::Number,
                        required: false,
                        default_value: None,
                        options: vec![],
                    },
                    crate::client::WorkflowNodeField {
                        key: "stream".into(),
                        label: "Stream".into(),
                        field_type: crate::client::WorkflowNodeFieldType::Boolean,
                        required: false,
                        default_value: Some(serde_json::json!(false)),
                        options: vec![],
                    },
                ],
                runtime_fields: vec![crate::client::WorkflowNodeField {
                    key: "response_text".into(),
                    label: "Response Text".into(),
                    field_type: crate::client::WorkflowNodeFieldType::String,
                    required: false,
                    default_value: None,
                    options: vec![],
                }],
            }];
            panel.set_active_canvas(&canvas);
            panel.set_workflow(workflow.clone(), cx);
            panel.set_node(Some("node-1".into()), window, cx);
            panel
        });

        panel.update_in(cx, |panel, window, cx| {
            assert!(
                panel.runtime_fields.is_empty(),
                "runtime fields should not be surfaced in the node inspector"
            );
            panel.label_editor.update(cx, |editor, cx| {
                editor.set_text("Saved Label", window, cx);
            });
            panel.required_reviews_editor.update(cx, |editor, cx| {
                editor.set_text("2", window, cx);
            });
            panel.required_checks_editor.update(cx, |editor, cx| {
                editor.set_text("lint, test", window, cx);
            });
            panel.max_attempts_editor.update(cx, |editor, cx| {
                editor.set_text("5", window, cx);
            });
            panel.backoff_ms_editor.update(cx, |editor, cx| {
                editor.set_text("2500", window, cx);
            });
            panel
                .configure_time_field_editors
                .get("model")
                .unwrap()
                .editor
                .update(cx, |editor, cx| {
                    editor.set_text("gpt-5.2", window, cx);
                });
            panel
                .configure_time_field_editors
                .get("max_tokens")
                .unwrap()
                .editor
                .update(cx, |editor, cx| {
                    editor.set_text("4096", window, cx);
                });
            panel
                .configure_time_field_editors
                .get("stream")
                .unwrap()
                .editor
                .update(cx, |editor, cx| {
                    editor.set_text("true", window, cx);
                });

            panel.apply_pending_node_edits(cx);
            panel.set_node(None, window, cx);
            panel.set_node(Some("node-1".into()), window, cx);

            assert_eq!(panel.label_editor.read(cx).text(cx), "Saved Label");
            assert_eq!(panel.required_reviews_editor.read(cx).text(cx), "2");
            assert_eq!(panel.required_checks_editor.read(cx).text(cx), "lint, test");
            assert_eq!(panel.max_attempts_editor.read(cx).text(cx), "5");
            assert_eq!(panel.backoff_ms_editor.read(cx).text(cx), "2500");

            let workflow = panel.workflow.as_ref().unwrap();
            let node = workflow
                .nodes
                .iter()
                .find(|node| node.id == "node-1")
                .unwrap();
            assert_eq!(node.label, "Saved Label");
            assert_eq!(node.configuration["model"], "gpt-5.2");
            assert_eq!(node.configuration["max_tokens"], 4096);
            assert_eq!(node.configuration["stream"], true);
            let policy = workflow.policy_for("node-1").unwrap();
            assert_eq!(policy.required_reviews, 2);
            assert_eq!(policy.required_checks, vec!["lint", "test"]);
            assert_eq!(policy.retry_behavior.max_attempts, 5);
            assert_eq!(policy.retry_behavior.backoff_ms, 2500);
        });

        let canvas = canvas_holder.borrow().clone().unwrap();
        canvas.read_with(cx, |canvas, _cx| {
            let workflow = canvas.workflow.as_ref().unwrap();
            let node = workflow
                .nodes
                .iter()
                .find(|node| node.id == "node-1")
                .unwrap();
            assert_eq!(node.label, "Saved Label");
            assert_eq!(node.configuration["model"], "gpt-5.2");
            assert_eq!(node.configuration["max_tokens"], 4096);
            assert_eq!(node.configuration["stream"], true);
            let policy = workflow.policy_for("node-1").unwrap();
            assert_eq!(policy.required_reviews, 2);
            assert_eq!(policy.required_checks, vec!["lint", "test"]);
            assert_eq!(policy.retry_behavior.max_attempts, 5);
            assert_eq!(policy.retry_behavior.backoff_ms, 2500);
        });
    }

    #[gpui::test]
    async fn test_apply_pending_node_edits_persists_conditional_configuration(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Conditional Workflow".into(),
            nodes: vec![
                crate::client::WorkflowNode {
                    id: "shell".into(),
                    node_type: WORKFLOW_EXECUTE_SHELL_COMMAND_NODE_TYPE_ID.into(),
                    label: "Build".into(),
                    configuration: serde_json::json!({}),
                    runtime: serde_json::json!({}),
                },
                crate::client::WorkflowNode {
                    id: "condition".into(),
                    node_type: WORKFLOW_CONDITIONAL_NODE_TYPE_ID.into(),
                    label: "Route".into(),
                    configuration: serde_json::to_value(
                        crate::client::WorkflowConditionalConfiguration::default(),
                    )
                    .unwrap(),
                    runtime: serde_json::json!({}),
                },
            ],
            edges: vec![crate::client::WorkflowEdge {
                from_node_id: "shell".into(),
                from_output_id: "success".into(),
                to_node_id: "condition".into(),
                to_input_id: "default".into(),
            }],
            node_policies: vec![],
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: BTreeMap::new(),
        };

        let (panel, cx) = cx.add_window_view(|window, cx| {
            let mut panel = NodeInspectorPanel::new_for_test(WorkflowClient::new(), window, cx);
            panel.node_types = crate::client::editor_node_types(vec![
                conditional_node_type(),
                crate::client::WorkflowNodeType {
                    id: WORKFLOW_EXECUTE_SHELL_COMMAND_NODE_TYPE_ID.into(),
                    label: "Execute Shell Command".into(),
                    primitive: Some(crate::client::WorkflowNodePrimitive::ExecuteShellCommand),
                    category: None,
                    is_primitive: true,
                    inputs: vec![crate::client::WorkflowNodePort {
                        id: "default".into(),
                        label: "Input".into(),
                    }],
                    outputs: vec![crate::client::WorkflowNodePort {
                        id: "success".into(),
                        label: "Success".into(),
                    }],
                    configure_time_fields: vec![],
                    runtime_fields: vec![],
                },
            ]);
            panel.set_workflow(workflow.clone(), cx);
            panel.set_node(Some("condition".into()), window, cx);
            panel
        });

        panel.update_in(cx, |panel, _window, cx| {
            let available_refs = panel.available_value_references();
            let exit_code_ref = available_refs
                .into_iter()
                .find(|reference| reference.path == "shell.exit_code")
                .unwrap();

            let state = panel.conditional_editor_state.as_mut().unwrap();
            let first_branch = state.configuration.branches.first_mut().unwrap();
            let first_group = first_branch.condition.as_mut().unwrap();
            let crate::client::WorkflowConditionNode::Predicate(predicate) =
                first_group.children.first_mut().unwrap()
            else {
                panic!("expected predicate");
            };
            predicate.lhs = Some(exit_code_ref);
            predicate.operator = Some(crate::client::WorkflowComparisonOperator::Neq);
            state
                .rhs_editors
                .get("branch-0.0")
                .unwrap()
                .update(cx, |editor, cx| {
                    editor.set_text("0", _window, cx);
                });

            panel.apply_pending_node_edits(cx);

            let workflow = panel.workflow.as_ref().unwrap();
            let node = workflow
                .nodes
                .iter()
                .find(|node| node.id == "condition")
                .unwrap();
            assert_eq!(node.configuration["branches"][0]["output_id"], "if_1");
            assert_eq!(
                node.configuration["branches"][0]["condition"]["children"][0]["lhs"]["path"],
                "shell.exit_code"
            );
            assert_eq!(
                node.configuration["branches"][0]["condition"]["children"][0]["operator"],
                "neq"
            );
            assert_eq!(
                node.configuration["branches"][0]["condition"]["children"][0]["rhs"],
                0
            );
        });
    }

    #[gpui::test]
    async fn test_apply_pending_node_edits_persists_globals_configuration(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx);

        let workflow = WorkflowDefinitionRecord {
            id: Uuid::nil(),
            name: "Globals Workflow".into(),
            nodes: vec![crate::client::WorkflowNode {
                id: "globals".into(),
                node_type: WORKFLOW_GLOBALS_NODE_TYPE_ID.into(),
                label: "Globals".into(),
                configuration: serde_json::json!({
                    "variables": [
                        {
                            "key": "owner",
                            "value_type": "string",
                            "default_value": "ops",
                            "allow_runtime_override": false,
                            "allow_task_mutation": false
                        }
                    ]
                }),
                runtime: serde_json::json!({}),
            }],
            edges: vec![],
            node_policies: vec![],
            retry_behavior: RetryBehavior::default(),
            validation_policy_ref: None,
            trigger_metadata: BTreeMap::new(),
        };

        let (panel, cx) = cx.add_window_view(|window, cx| {
            let mut panel = NodeInspectorPanel::new_for_test(WorkflowClient::new(), window, cx);
            panel.node_types = crate::client::editor_node_types(vec![]);
            panel.set_workflow(workflow.clone(), cx);
            panel.set_node(Some("globals".into()), window, cx);
            panel
        });

        panel.update_in(cx, |panel, window, cx| {
            let state = panel.globals_editor_state.as_mut().unwrap();
            state.key_editors[0].update(cx, |editor, cx| {
                editor.set_text("deploy_env", window, cx);
            });
            state.value_editors[0].update(cx, |editor, cx| {
                editor.set_text("prod", window, cx);
            });
            panel.globals_toggle_runtime_override(0, cx);
            panel.globals_toggle_task_mutation(0, cx);
            panel.apply_pending_node_edits(cx);

            let workflow = panel.workflow.as_ref().unwrap();
            let node = workflow
                .nodes
                .iter()
                .find(|node| node.id == "globals")
                .unwrap();
            assert_eq!(node.configuration["variables"][0]["key"], "deploy_env");
            assert_eq!(node.configuration["variables"][0]["default_value"], "prod");
            assert_eq!(
                node.configuration["variables"][0]["allow_runtime_override"],
                true
            );
            assert_eq!(
                node.configuration["variables"][0]["allow_task_mutation"],
                true
            );
        });
    }

    #[gpui::test]
    async fn test_node_inspector_activation_priority_is_stable(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let (panel, cx) = cx.add_window_view(|window, cx| {
            NodeInspectorPanel::new_for_test(WorkflowClient::new(), window, cx)
        });

        panel.update_in(cx, |panel, _window, _cx| {
            assert_eq!(panel.activation_priority(), 4);
        });
    }

    #[gpui::test]
    async fn test_begin_rename_prefills_editor_and_commit_updates_workflow_name(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx);

        let workflows = sample_sidebar_workflows();
        let workflow_to_rename = workflows[0].id;

        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = WorkflowDefsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            );
            view.workflows = workflows.clone();
            view
        });

        view.update_in(cx, |view, window, cx| {
            view.begin_rename_workflow(workflow_to_rename, window, cx);
            assert_eq!(view.renaming_workflow_id, Some(workflow_to_rename));
            assert_eq!(view.rename_editor.read(cx).text(cx), "Alpha");

            view.rename_editor.update(cx, |editor, cx| {
                editor.set_text("Renamed Alpha", window, cx);
            });

            let mut renamed = view
                .workflows
                .iter()
                .find(|workflow| workflow.id == workflow_to_rename)
                .cloned()
                .unwrap();
            renamed.name = "Renamed Alpha".into();
            view.replace_workflow(renamed);
            view.cancel_rename(cx);

            assert_eq!(
                view.workflows
                    .iter()
                    .find(|workflow| workflow.id == workflow_to_rename)
                    .unwrap()
                    .name,
                "Renamed Alpha"
            );
            assert_eq!(view.renaming_workflow_id, None);
        });
    }

    #[gpui::test]
    async fn test_begin_rename_focuses_editor_like_project_panel(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let workflows = sample_sidebar_workflows();
        let workflow_to_rename = workflows[0].id;

        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = WorkflowDefsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            );
            view.workflows = workflows.clone();
            view
        });

        view.update_in(cx, |view, window, cx| {
            view.begin_rename_workflow(workflow_to_rename, window, cx);
        });
        cx.run_until_parked();

        view.update_in(cx, |view, window, cx| {
            assert!(view.rename_editor.read(cx).is_focused(window));
        });
    }

    #[gpui::test]
    async fn test_confirm_rename_clears_inline_rename_state(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let workflows = sample_sidebar_workflows();
        let workflow_to_rename = workflows[0].id;

        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = WorkflowDefsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            );
            view.workflows = workflows.clone();
            view
        });

        view.update_in(cx, |view, window, cx| {
            view.begin_rename_workflow(workflow_to_rename, window, cx);
            view.confirm_rename(cx);

            assert_eq!(view.renaming_workflow_id, None);
            assert!(!view.pending_focus_rename_editor);
        });
    }

    #[gpui::test]
    async fn test_remove_workflow_clears_inline_rename_state(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let workflows = sample_sidebar_workflows();
        let removed_workflow = workflows[0].id;
        let remaining_workflow = workflows[1].id;

        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = WorkflowDefsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            );
            view.workflows = workflows.clone();
            view
        });

        view.update(cx, |view, cx| {
            view.renaming_workflow_id = Some(removed_workflow);
            view.remove_workflow(removed_workflow);
            assert_eq!(view.renaming_workflow_id, None);
            assert_eq!(view.workflows.len(), 1);
            assert_eq!(view.workflows[0].id, remaining_workflow);
            cx.notify();
        });
    }

    #[gpui::test]
    async fn test_workflow_defs_view_updates_when_cache_is_upserted(cx: &mut gpui::TestAppContext) {
        init_test(cx);

        let workflows = sample_sidebar_workflows();
        cx.update(|cx| {
            replace_workflow_defs_cache(vec![workflows[0].clone()], cx);
        });

        let added_workflow = workflows[1].clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            WorkflowDefsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            )
        });

        view.read_with(cx, |view, _cx| {
            assert_eq!(view.workflows.len(), 1);
            assert_eq!(view.workflows[0].id, workflows[0].id);
        });

        cx.update(|_, cx| {
            upsert_workflow_def_cache(added_workflow.clone(), cx);
        });
        cx.run_until_parked();

        view.read_with(cx, |view, _cx| {
            assert_eq!(view.workflows.len(), 2);
            assert!(
                view.workflows
                    .iter()
                    .any(|workflow| workflow.id == added_workflow.id)
            );
        });
    }

    #[gpui::test]
    async fn test_workflow_defs_view_filters_workflows_by_search_query(
        cx: &mut gpui::TestAppContext,
    ) {
        init_test(cx);

        let workflows = sample_sidebar_workflows();

        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut view = WorkflowDefsView::new_for_test(
                WorkflowClient::with_base_url("http://localhost:9".into()),
                window,
                cx,
            );
            view.workflows = workflows.clone();
            view
        });

        view.update(cx, |view, _cx| {
            view.set_search_query("beta");

            let filtered = view
                .filtered_workflows()
                .into_iter()
                .map(|workflow| workflow.name.clone())
                .collect::<Vec<_>>();

            assert_eq!(filtered, vec!["Beta".to_string()]);
        });
    }
}
