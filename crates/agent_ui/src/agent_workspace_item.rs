use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, SharedString, Styled, Window,
};
use ui::v_flex;
use workspace::{Item, Workspace, item::ItemEvent};

use crate::AiWorkspace;

pub struct AgentWorkspaceItem {
    ai_workspace: Entity<AiWorkspace>,
    focus_handle: FocusHandle,
}

impl AgentWorkspaceItem {
    pub fn new(ai_workspace: Entity<AiWorkspace>, focus_handle: FocusHandle) -> Self {
        Self {
            ai_workspace,
            focus_handle,
        }
    }

    pub fn deploy_in_workspace(
        ai_workspace: Entity<AiWorkspace>,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Entity<Self> {
        let existing_item = workspace
            .items_of_type::<AgentWorkspaceItem>(cx)
            .find(|item| item.read(cx).ai_workspace == ai_workspace);

        if let Some(existing_item) = existing_item {
            workspace.activate_item(&existing_item, true, true, window, cx);
            existing_item
        } else {
            let item_focus_handle = ai_workspace.read(cx).item_focus_handle();
            let item = cx.new(|_| AgentWorkspaceItem::new(ai_workspace.clone(), item_focus_handle));
            workspace.add_item_to_center(Box::new(item.clone()), window, cx);
            item
        }
    }

    pub fn ai_workspace(&self) -> &Entity<AiWorkspace> {
        &self.ai_workspace
    }
}

impl EventEmitter<ItemEvent> for AgentWorkspaceItem {}

impl Focusable for AgentWorkspaceItem {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for AgentWorkspaceItem {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, cx: &App) -> SharedString {
        self.ai_workspace.read(cx).tab_title(cx)
    }

    fn tab_tooltip_text(&self, cx: &App) -> Option<SharedString> {
        Some(self.tab_content_text(0, cx))
    }

    fn to_item_events(event: &Self::Event, emit: &mut dyn FnMut(ItemEvent)) {
        emit(*event);
    }
}

impl gpui::Render for AgentWorkspaceItem {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.ai_workspace.clone())
    }
}
