use std::{rc::Rc, sync::Arc};

use gpui::{App, BorrowAppContext, Global, WeakEntity, Window};
use ui::ContextMenu;
use workspace::{SelectedEntry, Workspace};

pub struct ProjectPanelContextMenuTarget {
    pub selected_entry: SelectedEntry,
    pub marked_entries: Arc<[SelectedEntry]>,
    pub is_dir: bool,
    pub is_root: bool,
    pub is_local: bool,
    pub is_remote: bool,
}

type ProjectPanelContextMenuHookFn = Rc<
    dyn Fn(
        ContextMenu,
        WeakEntity<Workspace>,
        &ProjectPanelContextMenuTarget,
        &mut Window,
        &mut App,
    ) -> ContextMenu,
>;

#[derive(Clone)]
struct RegisteredProjectPanelContextMenuHook {
    id: &'static str,
    handler: ProjectPanelContextMenuHookFn,
}

#[derive(Default)]
struct GlobalProjectPanelContextMenuHooks(Vec<RegisteredProjectPanelContextMenuHook>);

impl Global for GlobalProjectPanelContextMenuHooks {}

pub struct ProjectPanelContextMenuHooks;

impl ProjectPanelContextMenuHooks {
    pub fn set_named(
        cx: &mut App,
        id: &'static str,
        hook: impl Fn(
            ContextMenu,
            WeakEntity<Workspace>,
            &ProjectPanelContextMenuTarget,
            &mut Window,
            &mut App,
        ) -> ContextMenu
        + 'static,
    ) {
        let hook = RegisteredProjectPanelContextMenuHook {
            id,
            handler: Rc::new(hook),
        };
        cx.update_default_global(|hooks: &mut GlobalProjectPanelContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
            hooks.0.push(hook);
        });
    }

    pub fn clear_named(cx: &mut App, id: &str) {
        if !cx.has_global::<GlobalProjectPanelContextMenuHooks>() {
            return;
        }

        cx.update_global(|hooks: &mut GlobalProjectPanelContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
        });
    }

    pub fn apply(
        menu: ContextMenu,
        workspace: WeakEntity<Workspace>,
        target: &ProjectPanelContextMenuTarget,
        window: &mut Window,
        cx: &mut App,
    ) -> ContextMenu {
        let Some(hooks) = cx
            .try_global::<GlobalProjectPanelContextMenuHooks>()
            .map(|hooks| hooks.0.clone())
        else {
            return menu;
        };

        hooks.iter().fold(menu, |menu, hook| {
            (hook.handler)(menu, workspace.clone(), target, window, cx)
        })
    }
}
