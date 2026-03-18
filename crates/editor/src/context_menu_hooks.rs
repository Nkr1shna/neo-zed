use std::rc::Rc;

use gpui::{App, BorrowAppContext, Global, WeakEntity, Window};
use ui::ContextMenu;
use workspace::Workspace;

use crate::{DisplayPoint, Editor};

type EditorContextMenuHookFn = Rc<
    dyn Fn(
        ContextMenu,
        WeakEntity<Workspace>,
        WeakEntity<Editor>,
        DisplayPoint,
        &mut Window,
        &mut App,
    ) -> ContextMenu,
>;

#[derive(Clone)]
struct RegisteredEditorContextMenuHook {
    id: &'static str,
    handler: EditorContextMenuHookFn,
}

#[derive(Default)]
struct GlobalEditorContextMenuHooks(Vec<RegisteredEditorContextMenuHook>);

impl Global for GlobalEditorContextMenuHooks {}

pub struct EditorContextMenuHooks;

impl EditorContextMenuHooks {
    pub fn set_named(
        cx: &mut App,
        id: &'static str,
        hook: impl Fn(
            ContextMenu,
            WeakEntity<Workspace>,
            WeakEntity<Editor>,
            DisplayPoint,
            &mut Window,
            &mut App,
        ) -> ContextMenu
        + 'static,
    ) {
        let hook = RegisteredEditorContextMenuHook {
            id,
            handler: Rc::new(hook),
        };
        cx.update_default_global(|hooks: &mut GlobalEditorContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
            hooks.0.push(hook);
        });
    }

    pub fn clear_named(cx: &mut App, id: &str) {
        if !cx.has_global::<GlobalEditorContextMenuHooks>() {
            return;
        }

        cx.update_global(|hooks: &mut GlobalEditorContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
        });
    }

    pub fn apply(
        menu: ContextMenu,
        workspace: WeakEntity<Workspace>,
        editor: WeakEntity<Editor>,
        point: DisplayPoint,
        window: &mut Window,
        cx: &mut App,
    ) -> ContextMenu {
        let Some(hooks) = cx
            .try_global::<GlobalEditorContextMenuHooks>()
            .map(|hooks| hooks.0.clone())
        else {
            return menu;
        };

        hooks.iter().fold(menu, |menu, hook| {
            (hook.handler)(menu, workspace.clone(), editor.clone(), point, window, cx)
        })
    }
}
