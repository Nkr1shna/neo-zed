use gpui::{
    App, AppContext, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Styled, WeakEntity, Window, actions,
};
use gpui_util::ResultExt;
use orchestration::OrchestrationState;
use ui::{Headline, HeadlineSize, Label, ParentElement as _, v_flex};
use workspace::{Item, Workspace, item::ItemEvent};

actions!(orchestration, [OpenOrchestration]);

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &OpenOrchestration, window, cx| {
            let current_state = workspace.orchestration_state().clone();
            let existing_item = workspace.items_of_type::<OrchestrationItem>(cx).next();
            if let Some(existing_item) = existing_item {
                existing_item.update(cx, |item, cx| item.hydrate_state(current_state, cx));
                workspace.activate_item(&existing_item, true, true, window, cx);
            } else {
                let workspace_handle = cx.entity().downgrade();
                let item = cx.new(|cx| OrchestrationItem::new(workspace_handle, current_state, cx));
                workspace.add_item_to_center(Box::new(item), window, cx);
            }
        });
    })
    .detach();
}

pub struct OrchestrationItem {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    state: OrchestrationState,
}

impl OrchestrationItem {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        state: OrchestrationState,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            workspace,
            state,
        }
    }

    pub fn state(&self) -> &OrchestrationState {
        &self.state
    }

    fn hydrate_state(&mut self, state: OrchestrationState, cx: &mut Context<Self>) {
        self.state = state;
        cx.emit(ItemEvent::UpdateTab);
        cx.notify();
    }

    pub fn set_state(&mut self, state: OrchestrationState, cx: &mut Context<Self>) {
        self.hydrate_state(state.clone(), cx);
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.set_orchestration_state(state, cx)
            })
            .log_err();
    }
}

impl EventEmitter<ItemEvent> for OrchestrationItem {}

impl Focusable for OrchestrationItem {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for OrchestrationItem {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Orchestration".into()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Orchestration Item Opened")
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn to_item_events(event: &Self::Event, emit: &mut dyn FnMut(ItemEvent)) {
        emit(*event);
    }
}

impl Render for OrchestrationItem {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let feature_count = self.state.features.len();
        let project_count = self.state.projects.len();
        let task_count = self.state.tasks.len();

        v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .child(Headline::new("Orchestration").size(HeadlineSize::Large))
            .child(Label::new(
                "Host-native center workflow surface placeholder. Tree navigation will attach here.",
            ))
            .child(Label::new(format!(
                "{project_count} projects, {feature_count} features, {task_count} tasks loaded",
            )))
    }
}
