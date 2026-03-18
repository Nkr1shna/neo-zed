use std::rc::Rc;

use gpui::{App, BorrowAppContext, Global, WeakEntity, Window};
use ui::ContextMenu;

use crate::{Workspace, dock::PanelHandle, item::ItemHandle};

type ItemTabContextMenuHookFn = Rc<
    dyn Fn(
        ContextMenu,
        WeakEntity<Workspace>,
        &dyn ItemHandle,
        &mut Window,
        &mut App,
    ) -> ContextMenu,
>;
type PanelOverflowContextMenuHookFn = Rc<
    dyn Fn(
        ContextMenu,
        WeakEntity<Workspace>,
        &dyn PanelHandle,
        &mut Window,
        &mut App,
    ) -> ContextMenu,
>;

#[derive(Clone)]
struct RegisteredItemTabContextMenuHook {
    id: &'static str,
    handler: ItemTabContextMenuHookFn,
}

#[derive(Clone)]
struct RegisteredPanelOverflowContextMenuHook {
    id: &'static str,
    handler: PanelOverflowContextMenuHookFn,
}

#[derive(Default)]
struct GlobalItemTabContextMenuHooks(Vec<RegisteredItemTabContextMenuHook>);

impl Global for GlobalItemTabContextMenuHooks {}

#[derive(Default)]
struct GlobalPanelOverflowContextMenuHooks(Vec<RegisteredPanelOverflowContextMenuHook>);

impl Global for GlobalPanelOverflowContextMenuHooks {}

pub struct ItemTabContextMenuHooks;

impl ItemTabContextMenuHooks {
    pub fn set_named(
        cx: &mut App,
        id: &'static str,
        hook: impl Fn(
            ContextMenu,
            WeakEntity<Workspace>,
            &dyn ItemHandle,
            &mut Window,
            &mut App,
        ) -> ContextMenu
        + 'static,
    ) {
        let hook = RegisteredItemTabContextMenuHook {
            id,
            handler: Rc::new(hook),
        };
        cx.update_default_global(|hooks: &mut GlobalItemTabContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
            hooks.0.push(hook);
        });
    }

    pub fn clear_named(cx: &mut App, id: &str) {
        if !cx.has_global::<GlobalItemTabContextMenuHooks>() {
            return;
        }

        cx.update_global(|hooks: &mut GlobalItemTabContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
        });
    }

    pub fn apply(
        menu: ContextMenu,
        workspace: WeakEntity<Workspace>,
        item: &dyn ItemHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> ContextMenu {
        let Some(hooks) = cx
            .try_global::<GlobalItemTabContextMenuHooks>()
            .map(|hooks| hooks.0.clone())
        else {
            return menu;
        };

        hooks.iter().fold(menu, |menu, hook| {
            (hook.handler)(menu, workspace.clone(), item, window, cx)
        })
    }
}

pub struct PanelOverflowContextMenuHooks;

impl PanelOverflowContextMenuHooks {
    pub fn set_named(
        cx: &mut App,
        id: &'static str,
        hook: impl Fn(
            ContextMenu,
            WeakEntity<Workspace>,
            &dyn PanelHandle,
            &mut Window,
            &mut App,
        ) -> ContextMenu
        + 'static,
    ) {
        let hook = RegisteredPanelOverflowContextMenuHook {
            id,
            handler: Rc::new(hook),
        };
        cx.update_default_global(|hooks: &mut GlobalPanelOverflowContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
            hooks.0.push(hook);
        });
    }

    pub fn clear_named(cx: &mut App, id: &str) {
        if !cx.has_global::<GlobalPanelOverflowContextMenuHooks>() {
            return;
        }

        cx.update_global(|hooks: &mut GlobalPanelOverflowContextMenuHooks, _cx| {
            hooks.0.retain(|existing| existing.id != id);
        });
    }

    pub fn apply(
        menu: ContextMenu,
        workspace: WeakEntity<Workspace>,
        panel: &dyn PanelHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> ContextMenu {
        let Some(hooks) = cx
            .try_global::<GlobalPanelOverflowContextMenuHooks>()
            .map(|hooks| hooks.0.clone())
        else {
            return menu;
        };

        hooks.iter().fold(menu, |menu, hook| {
            (hook.handler)(menu, workspace.clone(), panel, window, cx)
        })
    }
}
