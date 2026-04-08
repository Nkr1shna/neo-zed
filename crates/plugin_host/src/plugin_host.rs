use anyhow::Result;
use gpui_api::{
    Entity, EntityId, HandlerId, Render, RenderOutput, Runtime, UiEvent, UiNode, Window,
};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub enum HostToPluginMessage {
    DispatchEvent {
        handler_id: HandlerId,
        event: UiEvent,
    },
}

#[derive(Clone, Debug)]
pub enum PluginToHostMessage {
    Rendered { generation: u64, tree: UiNode },
    RerenderRequested { entity_id: EntityId },
    Error { message: String },
}

pub struct PanelSession<T: 'static> {
    runtime: Runtime,
    root: Entity<T>,
    last_render: Option<RenderOutput>,
    pending_messages: VecDeque<PluginToHostMessage>,
}

impl<T: Render + 'static> PanelSession<T> {
    pub fn new(runtime: Runtime, root: Entity<T>) -> Self {
        Self {
            runtime,
            root,
            last_render: None,
            pending_messages: VecDeque::new(),
        }
    }

    pub fn initial_render(&mut self) -> Result<PluginToHostMessage> {
        self.render()
    }

    pub fn handle_message(
        &mut self,
        message: HostToPluginMessage,
    ) -> Result<Option<PluginToHostMessage>> {
        match message {
            HostToPluginMessage::DispatchEvent { handler_id, event } => {
                let mut window = Window::default();
                if let Some(render) = self.last_render.as_ref() {
                    render.dispatch(handler_id, &event, &mut window, &mut self.runtime)?;
                }

                if let Some(error_message) = self.runtime.take_errors().into_iter().next() {
                    return Ok(Some(PluginToHostMessage::Error {
                        message: error_message,
                    }));
                }

                if self.runtime.is_dirty(self.root.entity_id()) {
                    Ok(Some(PluginToHostMessage::RerenderRequested {
                        entity_id: self.root.entity_id(),
                    }))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub fn render_if_dirty(&mut self) -> Result<PluginToHostMessage> {
        self.render()
    }

    pub fn drain_tasks(&mut self) -> Result<usize> {
        let drained = self.runtime.drain_tasks();

        for error_message in self.runtime.take_errors() {
            self.pending_messages.push_back(PluginToHostMessage::Error {
                message: error_message,
            });
        }

        if self.runtime.is_dirty(self.root.entity_id()) {
            self.pending_messages
                .push_back(PluginToHostMessage::RerenderRequested {
                    entity_id: self.root.entity_id(),
                });
        }

        Ok(drained)
    }

    pub fn take_pending_message(&mut self) -> Option<PluginToHostMessage> {
        self.pending_messages.pop_front()
    }

    fn render(&mut self) -> Result<PluginToHostMessage> {
        let render = self.runtime.render_root(&self.root)?;
        let message = PluginToHostMessage::Rendered {
            generation: render.generation(),
            tree: render.tree.clone(),
        };
        self.last_render = Some(render);
        Ok(message)
    }
}

mod host {
    use anyhow::{Context as _, Result};
    #[cfg(target_os = "macos")]
    use core_foundation::{
        base::{CFType, CFTypeRef, OSStatus, TCFType},
        boolean::CFBoolean,
        data::CFData,
        dictionary::{CFDictionaryRef, CFMutableDictionary},
        string::{CFString, CFStringRef},
    };
    use futures::future::{self, Either};
    use futures::io::{BufReader, BufWriter};
    use futures::{AsyncBufReadExt as _, AsyncWriteExt as _};
    use gpui::{
        Action, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
        InteractiveElement, ParentElement, Render, SharedString, StatefulInteractiveElement,
        Styled, Subscription, WeakEntity, Window, div,
    };
    use plugin::{InstalledPlugin, PluginStore, PluginStoreLayout};
    use plugin_protocol::{
        DockPosition as PluginDockPosition, EventHandlerId, HostThemeSnapshot, HostToPlugin,
        INTERACTIVE_PROP_BLOCK_MOUSE_EXCEPT_SCROLL, INTERACTIVE_PROP_FOCUSABLE,
        INTERACTIVE_PROP_GROUP, INTERACTIVE_PROP_KEY_CONTEXT, INTERACTIVE_PROP_OCCLUDE,
        INTERACTIVE_PROP_TAB_GROUP, INTERACTIVE_PROP_TAB_INDEX, INTERACTIVE_PROP_TAB_STOP,
        INTERACTIVE_PROP_WINDOW_CONTROL_AREA, PanelActivation, PanelDescriptor, PanelInstanceId,
        PluginHostRequest, PluginHostResponse, PluginId, PluginToHost, SerializedActionEvent,
        SerializedClickEvent, SerializedKeyDownEvent, SerializedKeyUpEvent,
        SerializedModifiersChangedEvent, SerializedMouseDownEvent, SerializedMouseMoveEvent,
        SerializedMousePressureEvent, SerializedMouseUpEvent, SerializedPinchEvent,
        SerializedScrollWheelEvent, StyleValue, TitlebarWidgetDescriptor, TitlebarWidgetSide,
        UiEvent, UiEventKind, UiEventPhase, UiNode, UiNodeKind, apply_ui_patches,
    };
    use serde::Deserialize;
    #[cfg(target_os = "windows")]
    use smol::Unblock;
    use smol::channel;
    #[cfg(not(target_os = "windows"))]
    use smol::process::Command;
    #[cfg(target_os = "linux")]
    use std::ffi::CString;
    #[cfg(target_os = "linux")]
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStrExt as _;
    #[cfg(target_os = "linux")]
    use std::os::unix::process::CommandExt as _;
    #[cfg(target_os = "windows")]
    use std::os::windows::{
        ffi::OsStrExt as _,
        io::{FromRawHandle as _, OwnedHandle, RawHandle},
        process::ExitStatusExt as _,
    };
    #[cfg(target_os = "macos")]
    use std::ptr;
    use std::{
        any::TypeId,
        collections::{BTreeMap, BTreeSet},
        env,
        ffi::{OsStr, OsString},
        io,
        path::{Path, PathBuf},
        process::Stdio,
        time::Duration,
    };
    use theme::GlobalTheme;
    use title_bar::TitleBar;
    use ui::{
        Button, Color, ContextMenu, Divider, Icon, IconButton, IconName, IconSize, Indicator,
        Label, PopoverMenu, ProgressBar, Tab, prelude::*,
    };
    use workspace::{
        MultiWorkspace, Workspace,
        dock::{DockPosition, Panel, PanelEvent},
    };

    #[derive(Clone, Debug, PartialEq, Eq, Deserialize, gpui::Action)]
    #[action(namespace = plugin_host, no_json, no_register)]
    pub struct ToggleRemotePluginPanel {
        pub panel_entity_id: u64,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Deserialize, gpui::Action)]
    #[action(namespace = plugin_host, no_json, no_register)]
    struct DispatchPluginClick {
        handler_id: String,
    }

    pub fn init(cx: &mut App) {
        PluginHostRegistry::init_global(cx);
        cx.observe_global::<GlobalTheme>({
            move |cx| {
                let registry = PluginHostRegistry::global(cx);
                if let Err(error) =
                    registry.update(cx, |registry, cx| registry.sync_active_theme(cx))
                {
                    log::error!("failed to sync plugin themes after theme change: {error:#}");
                }
            }
        })
        .detach();
        cx.observe_new(|workspace: &mut Workspace, window, cx| {
            workspace.register_action(|workspace, action: &ToggleRemotePluginPanel, window, cx| {
                workspace.toggle_panel_by_id(action.panel_entity_id.into(), window, cx);
            });

            let Some(window) = window else {
                return;
            };

            let registry = PluginHostRegistry::global(cx);
            if let Some(titlebar) = workspace
                .titlebar_item()
                .and_then(|item| item.downcast::<TitleBar>().ok())
            {
                let left_strip = cx.new(|cx| {
                    PluginTitlebarStrip::new(
                        TitlebarStripSide::Left,
                        workspace.weak_handle(),
                        registry.clone(),
                        cx,
                    )
                });
                let right_strip = cx.new(|cx| {
                    PluginTitlebarStrip::new(
                        TitlebarStripSide::Right,
                        workspace.weak_handle(),
                        registry.clone(),
                        cx,
                    )
                });
                titlebar.update(cx, |titlebar, cx| {
                    titlebar.set_contributed_items(
                        Some(left_strip.into()),
                        Some(right_strip.into()),
                        cx,
                    );
                });
            }

            if let Err(error) =
                sync_workspace_panels_for_workspace(&registry, workspace, window, cx)
            {
                log::error!("failed to sync plugin panels for workspace startup: {error:#}");
            }
        })
        .detach();
    }

    pub fn refresh_catalog(cx: &mut App) {
        let registry = PluginHostRegistry::global(cx);
        if let Err(error) = registry.update(cx, |registry, cx| {
            registry.refresh_catalog(cx);
            Ok::<(), anyhow::Error>(())
        }) {
            log::error!("failed to refresh plugin catalog: {error:#}");
        }

        for window_handle in workspace::local_workspace_windows(cx) {
            let registry = registry.clone();
            let result =
                window_handle.update(cx, |multi_workspace: &mut MultiWorkspace, window, cx| {
                    let workspaces = multi_workspace.workspaces().to_vec();
                    for workspace in workspaces {
                        workspace.update(cx, |workspace, workspace_cx| {
                            sync_workspace_panels_for_workspace(
                                &registry,
                                workspace,
                                window,
                                workspace_cx,
                            )
                        })?;
                    }

                    Ok::<(), anyhow::Error>(())
                });

            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) | Err(error) => {
                    log::error!("failed to refresh plugin panels in workspace window: {error:#}");
                }
            }
        }
    }

    pub fn open_panel_in_workspace(
        plugin_id: &str,
        panel_id: &str,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<Entity<RemotePluginPanel>> {
        let registry = PluginHostRegistry::global(cx);
        ensure_panel_in_workspace(&registry, plugin_id, panel_id, true, workspace, window, cx)
    }

    fn toggle_or_open_panel_in_workspace(
        plugin_id: &str,
        panel_id: &str,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<()> {
        let registry = PluginHostRegistry::global(cx);
        if let Some(panel_entity_id) = registry
            .read(cx)
            .existing_panel_entity_id(workspace, plugin_id, panel_id)
        {
            workspace.toggle_panel_by_id(panel_entity_id, window, cx);
        } else {
            open_panel_in_workspace(plugin_id, panel_id, workspace, window, cx)?;
        }

        Ok(())
    }

    struct GlobalPluginHost(Entity<PluginHostRegistry>);

    impl Global for GlobalPluginHost {}

    #[derive(Clone)]
    struct RemotePanelBinding {
        plugin_id: PluginId,
        panel_id: String,
        workspace: WeakEntity<Workspace>,
        panel: WeakEntity<RemotePluginPanel>,
        panel_entity_id: gpui::EntityId,
    }

    #[derive(Clone)]
    struct TitlebarWidgetBinding {
        plugin_id: PluginId,
        descriptor: TitlebarWidgetDescriptor,
        workspace: WeakEntity<Workspace>,
        widget: Entity<RemotePluginTitlebarWidget>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RegisteredTitlebarWidget {
        plugin_id: PluginId,
        descriptor: TitlebarWidgetDescriptor,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TitlebarStripSide {
        Left,
        Right,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum RemotePluginStartupState {
        Building,
        Starting,
        Cancelling,
    }

    impl RemotePluginStartupState {
        fn message(self) -> &'static str {
            match self {
                Self::Building => "Building plugin...",
                Self::Starting => "Starting plugin...",
                Self::Cancelling => "Cancelling plugin startup...",
            }
        }

        fn can_cancel(self) -> bool {
            matches!(self, Self::Building | Self::Starting)
        }
    }

    pub struct PluginTitlebarStrip {
        side: TitlebarStripSide,
        workspace: WeakEntity<Workspace>,
        registry: Entity<PluginHostRegistry>,
        _subscription: Subscription,
    }

    impl PluginTitlebarStrip {
        fn new(
            side: TitlebarStripSide,
            workspace: WeakEntity<Workspace>,
            registry: Entity<PluginHostRegistry>,
            cx: &mut Context<Self>,
        ) -> Self {
            let subscription = cx.observe(&registry, |_, _, cx| {
                cx.notify();
            });

            Self {
                side,
                workspace,
                registry,
                _subscription: subscription,
            }
        }
    }

    impl Render for PluginTitlebarStrip {
        fn render(
            &mut self,
            _window: &mut Window,
            cx: &mut Context<Self>,
        ) -> impl gpui::IntoElement {
            let widgets = self
                .registry
                .update(cx, |registry, cx| {
                    registry.titlebar_widget_views(self.side, self.workspace.clone(), cx)
                })
                .unwrap_or_else(|error| {
                    log::error!("failed to render plugin titlebar strip: {error:#}");
                    Vec::new()
                });

            h_flex()
                .h_full()
                .flex_none()
                .items_center()
                .children(widgets)
        }
    }

    pub struct RemotePluginTitlebarWidget {
        plugin_id: PluginId,
        descriptor: TitlebarWidgetDescriptor,
        panel_instance_id: PanelInstanceId,
        tree: Option<UiNode>,
        render_generation: Option<u64>,
        startup_state: Option<RemotePluginStartupState>,
        error_message: Option<SharedString>,
        event_sender: channel::Sender<PluginHostEvent>,
        workspace: WeakEntity<Workspace>,
        registry: WeakEntity<PluginHostRegistry>,
        widget_entity_id: u64,
    }

    impl RemotePluginTitlebarWidget {
        fn new(
            plugin_id: PluginId,
            descriptor: TitlebarWidgetDescriptor,
            panel_instance_id: PanelInstanceId,
            workspace: WeakEntity<Workspace>,
            registry: WeakEntity<PluginHostRegistry>,
            event_sender: channel::Sender<PluginHostEvent>,
            cx: &mut Context<Self>,
        ) -> Self {
            Self {
                plugin_id,
                descriptor,
                panel_instance_id,
                tree: None,
                render_generation: None,
                startup_state: None,
                error_message: None,
                event_sender,
                workspace,
                registry,
                widget_entity_id: cx.entity().entity_id().as_u64(),
            }
        }

        fn update_tree(&mut self, tree: UiNode, generation: Option<u64>, cx: &mut Context<Self>) {
            self.tree = Some(tree);
            self.render_generation = generation;
            self.startup_state = None;
            self.error_message = None;
            cx.notify();
        }

        fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
            self.tree = None;
            self.render_generation = None;
            self.startup_state = None;
            self.error_message = Some(message.into().into());
            cx.notify();
        }

        fn prepare_for_open(
            &mut self,
            startup_state: Option<RemotePluginStartupState>,
            cx: &mut Context<Self>,
        ) {
            self.tree = None;
            self.render_generation = None;
            self.startup_state = startup_state;
            self.error_message = None;
            cx.notify();
        }

        fn set_startup_state(
            &mut self,
            startup_state: Option<RemotePluginStartupState>,
            cx: &mut Context<Self>,
        ) {
            self.startup_state = startup_state;
            if startup_state.is_some() {
                self.tree = None;
                self.render_generation = None;
                self.error_message = None;
            }
            cx.notify();
        }

        fn dispatch_event(
            &mut self,
            handler_id: EventHandlerId,
            kind: UiEventKind,
            payload: Option<serde_json::Value>,
            cx: &mut Context<Self>,
        ) {
            let event = UiEvent {
                panel_instance_id: self.panel_instance_id.clone(),
                generation: self.render_generation,
                handler_id,
                kind,
                payload,
            };

            let result = self
                .registry
                .update(cx, |registry, _cx| registry.dispatch_event(event));
            if let Err(error) = result {
                self.set_error(error.to_string(), cx);
            }
        }

        fn toggle_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            let Some(panel_id) = self.descriptor.opens_panel_id.clone() else {
                return;
            };

            let result = self.workspace.update(cx, |workspace, workspace_cx| {
                toggle_or_open_panel_in_workspace(
                    self.plugin_id.as_str(),
                    &panel_id,
                    workspace,
                    window,
                    workspace_cx,
                )
            });

            if let Err(error) = result {
                self.set_error(error.to_string(), cx);
            }
        }
    }

    impl Drop for RemotePluginTitlebarWidget {
        fn drop(&mut self) {
            self.event_sender
                .try_send(PluginHostEvent::ViewDetached {
                    panel_instance_id: self.panel_instance_id.clone(),
                })
                .ok();
        }
    }

    impl Render for RemotePluginTitlebarWidget {
        fn render(
            &mut self,
            _window: &mut Window,
            cx: &mut Context<Self>,
        ) -> impl gpui::IntoElement {
            let content = if let Some(error_message) = self.error_message.clone() {
                Label::new(error_message)
                    .color(Color::Error)
                    .into_any_element()
            } else if let Some(tree) = self.tree.clone() {
                render_remote_node(self, &tree, &[], cx)
            } else if let Some(startup_state) = self.startup_state {
                Label::new(startup_state.message())
                    .color(Color::Muted)
                    .into_any_element()
            } else {
                Label::new(self.descriptor.title.clone())
                    .color(Color::Muted)
                    .into_any_element()
            };

            if self.descriptor.opens_panel_id.is_some() {
                div()
                    .id(format!(
                        "plugin-titlebar-widget-{}-{}",
                        self.plugin_id, self.descriptor.id
                    ))
                    .child(content)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_panel(window, cx);
                    }))
                    .into_any_element()
            } else {
                content
            }
        }
    }

    pub struct PluginHostRegistry {
        layout: PluginStoreLayout,
        processes: BTreeMap<PluginId, PluginProcess>,
        panels: BTreeMap<PanelInstanceId, RemotePanelBinding>,
        titlebar_widgets: BTreeMap<PanelInstanceId, TitlebarWidgetBinding>,
        next_panel_activation_priority: u32,
        next_process_instance_id: u64,
        event_sender: channel::Sender<PluginHostEvent>,
        _event_task: Option<gpui::Task<()>>,
    }

    impl PluginHostRegistry {
        pub fn init_global(cx: &mut App) -> Entity<Self> {
            if cx.has_global::<GlobalPluginHost>() {
                return cx.global::<GlobalPluginHost>().0.clone();
            }

            let layout = default_layout();
            let (event_sender, event_receiver) = channel::unbounded();
            let registry = cx.new(|_cx| Self {
                layout,
                processes: BTreeMap::default(),
                panels: BTreeMap::default(),
                titlebar_widgets: BTreeMap::default(),
                next_panel_activation_priority: 10_000,
                next_process_instance_id: 1,
                event_sender,
                _event_task: None,
            });

            let weak_registry = registry.downgrade();
            let event_task = cx.spawn(async move |cx| {
                while let Ok(event) = event_receiver.recv().await {
                    let Ok(()) = weak_registry.update(cx, |registry, cx| {
                        registry.handle_event(event, cx);
                    }) else {
                        break;
                    };
                }
            });

            registry.update(cx, |registry, _cx| {
                registry._event_task = Some(event_task);
            });

            cx.set_global(GlobalPluginHost(registry.clone()));
            registry
        }

        pub fn global(cx: &App) -> Entity<Self> {
            cx.global::<GlobalPluginHost>().0.clone()
        }

        pub fn list_plugins(&self) -> Result<Vec<InstalledPlugin>> {
            self.store().list()
        }

        pub fn install_from_directory(&self, source_directory: PathBuf) -> Result<InstalledPlugin> {
            let mut store = self.store();
            store.install_from_directory(source_directory)
        }

        pub fn register_development_plugin(
            &self,
            source_directory: PathBuf,
        ) -> Result<InstalledPlugin> {
            let mut store = self.store();
            store.register_development_plugin(source_directory)
        }

        pub fn remove_plugin(&mut self, plugin_id: &str) -> Result<bool> {
            let plugin_id = PluginId::new(plugin_id);
            if let Some(process) = self.processes.get(&plugin_id) {
                send_message(&process.sender, &HostToPlugin::Shutdown)?;
            }
            let mut store = self.store();
            let removed = store.remove(plugin_id.as_str())?;
            if removed {
                self.processes.remove(&plugin_id);
            }
            Ok(removed)
        }

        pub fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
            if let Err(error) = self.prune_titlebar_widget_bindings(cx) {
                log::error!("failed to prune plugin titlebar widgets: {error:#}");
            }
            cx.notify();
        }

        fn sync_active_theme(&mut self, cx: &mut Context<Self>) -> Result<()> {
            let theme = current_theme_snapshot(cx);
            for process in self.processes.values() {
                send_message(
                    &process.sender,
                    &HostToPlugin::ThemeChanged {
                        theme: theme.clone(),
                    },
                )?;
            }
            Ok(())
        }

        fn allocate_panel_activation_priority(&mut self) -> u32 {
            let activation_priority = self.next_panel_activation_priority;
            self.next_panel_activation_priority =
                self.next_panel_activation_priority.saturating_add(1);
            activation_priority
        }

        fn panel_descriptor(&self, plugin_id: &str, panel_id: &str) -> Result<PanelDescriptor> {
            let plugin = self.installed_plugin(plugin_id)?;
            plugin
                .manifest
                .panels
                .iter()
                .find(|descriptor| descriptor.id == panel_id)
                .cloned()
                .with_context(|| {
                    format!("panel `{panel_id}` was not found in plugin `{plugin_id}`")
                })
        }

        fn titlebar_widget_descriptors(
            &self,
            side: TitlebarStripSide,
        ) -> Result<Vec<RegisteredTitlebarWidget>> {
            let mut widgets = self
                .list_plugins()?
                .into_iter()
                .flat_map(|plugin| {
                    let plugin_id = plugin.manifest.id.clone();
                    plugin
                        .manifest
                        .titlebar_widgets
                        .into_iter()
                        .filter(move |descriptor| {
                            matches!(
                                (side, &descriptor.side),
                                (TitlebarStripSide::Left, TitlebarWidgetSide::Left)
                                    | (TitlebarStripSide::Right, TitlebarWidgetSide::Right)
                            )
                        })
                        .map(move |descriptor| RegisteredTitlebarWidget {
                            plugin_id: plugin_id.clone(),
                            descriptor,
                        })
                })
                .collect::<Vec<_>>();

            widgets.sort_by(|left, right| {
                left.descriptor
                    .priority
                    .cmp(&right.descriptor.priority)
                    .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
                    .then_with(|| left.descriptor.id.cmp(&right.descriptor.id))
            });

            Ok(widgets)
        }

        fn existing_panel_binding(
            &self,
            workspace: WeakEntity<Workspace>,
            plugin_id: &str,
            panel_id: &str,
        ) -> Option<RemotePanelBinding> {
            self.panels
                .values()
                .find(|binding| {
                    binding.plugin_id.as_str() == plugin_id
                        && binding.panel_id == panel_id
                        && binding.workspace == workspace
                        && binding.panel.upgrade().is_some()
                })
                .cloned()
        }

        fn existing_panel_entity_id(
            &self,
            workspace: &Workspace,
            plugin_id: &str,
            panel_id: &str,
        ) -> Option<gpui::EntityId> {
            self.existing_panel_binding(workspace.weak_handle(), plugin_id, panel_id)
                .map(|binding| binding.panel_entity_id)
        }

        fn titlebar_widget_views(
            &mut self,
            side: TitlebarStripSide,
            workspace: WeakEntity<Workspace>,
            cx: &mut Context<Self>,
        ) -> Result<Vec<Entity<RemotePluginTitlebarWidget>>> {
            self.prune_titlebar_widget_bindings(cx)?;

            let mut widgets = Vec::new();
            for RegisteredTitlebarWidget {
                plugin_id,
                descriptor,
            } in self.titlebar_widget_descriptors(side)?
            {
                let view_instance_id =
                    titlebar_widget_instance_id(&plugin_id, &descriptor.id, workspace.entity_id());
                if let Some(binding) = self.titlebar_widgets.get(&view_instance_id).cloned() {
                    widgets.push(binding.widget);
                    continue;
                }

                let registry = cx.entity().downgrade();
                let event_sender = self.event_sender.clone();
                let widget: Entity<RemotePluginTitlebarWidget> =
                    cx.new(|widget_cx: &mut Context<RemotePluginTitlebarWidget>| {
                        RemotePluginTitlebarWidget::new(
                            plugin_id.clone(),
                            descriptor.clone(),
                            view_instance_id.clone(),
                            workspace.clone(),
                            registry.clone(),
                            event_sender.clone(),
                            widget_cx,
                        )
                    });

                self.attach_titlebar_widget(
                    &plugin_id,
                    &descriptor,
                    view_instance_id,
                    workspace.clone(),
                    widget.clone(),
                    cx,
                )?;
                widgets.push(widget);
            }

            Ok(widgets)
        }

        fn attach_panel(
            &mut self,
            plugin_id: &str,
            panel_id: &str,
            panel_instance_id: PanelInstanceId,
            workspace: WeakEntity<Workspace>,
            panel: WeakEntity<RemotePluginPanel>,
            panel_entity_id: gpui::EntityId,
            cx: &mut Context<Self>,
        ) -> Result<()> {
            let plugin = self.installed_plugin(plugin_id)?;
            let sender = self.ensure_process(&plugin, cx)?;
            self.panels.insert(
                panel_instance_id.clone(),
                RemotePanelBinding {
                    plugin_id: plugin.manifest.id.clone(),
                    panel_id: panel_id.to_string(),
                    workspace,
                    panel: panel.clone(),
                    panel_entity_id,
                },
            );
            if let Some(process) = self.processes.get_mut(&plugin.manifest.id) {
                process.view_instances.insert(panel_instance_id.clone());
            }

            let startup_state = self
                .processes
                .get(&plugin.manifest.id)
                .and_then(|process| initial_startup_state_for_plugin(&plugin, process.registered));

            panel.update(cx, |panel, cx| {
                panel.prepare_for_open(startup_state, cx);
            })?;

            send_message(
                &sender,
                &HostToPlugin::OpenPanel {
                    panel_id: panel_id.to_string(),
                    panel_instance_id,
                    theme: current_theme_snapshot(cx),
                },
            )?;

            if let Some(process) = self.processes.get_mut(&plugin.manifest.id) {
                process.view_entity_ids.insert(panel_entity_id);
            }

            Ok(())
        }

        fn attach_titlebar_widget(
            &mut self,
            plugin_id: &PluginId,
            descriptor: &TitlebarWidgetDescriptor,
            view_instance_id: PanelInstanceId,
            workspace: WeakEntity<Workspace>,
            widget: Entity<RemotePluginTitlebarWidget>,
            cx: &mut Context<Self>,
        ) -> Result<()> {
            let plugin = self.installed_plugin(plugin_id.as_str())?;
            let sender = self.ensure_process(&plugin, cx)?;
            self.titlebar_widgets.insert(
                view_instance_id.clone(),
                TitlebarWidgetBinding {
                    plugin_id: plugin_id.clone(),
                    descriptor: descriptor.clone(),
                    workspace,
                    widget: widget.clone(),
                },
            );
            if let Some(process) = self.processes.get_mut(&plugin.manifest.id) {
                process.view_instances.insert(view_instance_id.clone());
                process.view_entity_ids.insert(widget.entity_id());
            }

            let startup_state = self
                .processes
                .get(&plugin.manifest.id)
                .and_then(|process| initial_startup_state_for_plugin(&plugin, process.registered));

            widget.update(cx, |widget, cx| {
                widget.prepare_for_open(startup_state, cx);
            });

            send_message(
                &sender,
                &HostToPlugin::OpenPanel {
                    panel_id: descriptor.id.clone(),
                    panel_instance_id: view_instance_id,
                    theme: current_theme_snapshot(cx),
                },
            )?;

            Ok(())
        }

        fn detach_panel_binding(
            &mut self,
            panel_instance_id: &PanelInstanceId,
        ) -> Result<Option<RemotePanelBinding>> {
            let Some(binding) = self.panels.remove(panel_instance_id) else {
                return Ok(None);
            };

            if let Some(process) = self.processes.get_mut(&binding.plugin_id) {
                process.view_instances.remove(panel_instance_id);
                process.view_entity_ids.remove(&binding.panel_entity_id);
            }

            if let Err(error) = self.close_remote_view(&binding.plugin_id, panel_instance_id) {
                log::error!("failed to close plugin panel view {panel_instance_id}: {error:#}");
            }
            self.terminate_process_if_idle(&binding.plugin_id);
            Ok(Some(binding))
        }

        fn detach_titlebar_widget_binding(
            &mut self,
            panel_instance_id: &PanelInstanceId,
        ) -> Result<Option<TitlebarWidgetBinding>> {
            let Some(binding) = self.titlebar_widgets.remove(panel_instance_id) else {
                return Ok(None);
            };

            if let Some(process) = self.processes.get_mut(&binding.plugin_id) {
                process.view_instances.remove(panel_instance_id);
                process.view_entity_ids.remove(&binding.widget.entity_id());
            }

            if let Err(error) = self.close_remote_view(&binding.plugin_id, panel_instance_id) {
                log::error!(
                    "failed to close plugin titlebar widget view {panel_instance_id}: {error:#}"
                );
            }
            self.terminate_process_if_idle(&binding.plugin_id);
            Ok(Some(binding))
        }

        fn prune_titlebar_widget_bindings(&mut self, _cx: &mut Context<Self>) -> Result<()> {
            let valid_widget_keys = self
                .list_plugins()?
                .into_iter()
                .flat_map(|plugin| {
                    let plugin_id = plugin.manifest.id.clone();
                    plugin
                        .manifest
                        .titlebar_widgets
                        .into_iter()
                        .map(move |descriptor| (plugin_id.clone(), descriptor.id))
                })
                .collect::<BTreeSet<_>>();

            let stale_instance_ids = self
                .titlebar_widgets
                .iter()
                .filter_map(|(panel_instance_id, binding)| {
                    let widget_key = (binding.plugin_id.clone(), binding.descriptor.id.clone());
                    if binding.workspace.upgrade().is_none()
                        || !valid_widget_keys.contains(&widget_key)
                    {
                        Some(panel_instance_id.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            for panel_instance_id in stale_instance_ids {
                self.detach_titlebar_widget_binding(&panel_instance_id)?;
            }

            Ok(())
        }

        fn dispatch_event(&mut self, event: UiEvent) -> Result<()> {
            let plugin_id = self
                .processes
                .iter()
                .find_map(|(plugin_id, process)| {
                    process
                        .view_instances
                        .contains(&event.panel_instance_id)
                        .then_some(plugin_id.clone())
                })
                .with_context(|| {
                    format!(
                        "remote view session `{}` is not attached to a running plugin process",
                        event.panel_instance_id
                    )
                })?;
            let process = self
                .processes
                .get(&plugin_id)
                .context("plugin process disappeared")?;
            send_message(&process.sender, &HostToPlugin::DispatchEvent { event })
        }

        fn handle_event(&mut self, event: PluginHostEvent, cx: &mut Context<Self>) {
            match event {
                PluginHostEvent::Message {
                    plugin_id,
                    process_instance_id,
                    message,
                } => {
                    if !self.has_current_process(&plugin_id, process_instance_id) {
                        return;
                    }
                    if let Err(error) = self.apply_message(plugin_id, message, cx) {
                        log::error!("plugin host message error: {error:#}");
                    }
                }
                PluginHostEvent::RegistrationTimedOut {
                    plugin_id,
                    process_instance_id,
                } => {
                    let Some(process) = self.processes.get(&plugin_id) else {
                        return;
                    };
                    if process.instance_id != process_instance_id || process.registered {
                        return;
                    }

                    if let Err(error) = process
                        .terminate_sender
                        .try_send(ProcessTermination::RegistrationTimedOut)
                    {
                        log::error!("failed to terminate timed out plugin `{plugin_id}`: {error}");
                    }
                }
                PluginHostEvent::ViewDetached { panel_instance_id } => {
                    if let Err(error) = self.detach_panel_binding(&panel_instance_id) {
                        log::error!(
                            "failed to detach dropped plugin panel `{panel_instance_id}`: {error:#}"
                        );
                        return;
                    }

                    if let Err(error) = self.detach_titlebar_widget_binding(&panel_instance_id) {
                        log::error!(
                            "failed to detach dropped plugin titlebar widget `{panel_instance_id}`: {error:#}"
                        );
                    }
                }
                PluginHostEvent::Exited {
                    plugin_id,
                    process_instance_id,
                    exit_status,
                    error_message,
                    suppress_ui_error,
                } => {
                    if !self.has_current_process(&plugin_id, process_instance_id) {
                        return;
                    }

                    if let Some(process) = self.processes.remove(&plugin_id) {
                        let message = error_message.unwrap_or_else(|| {
                            format!(
                                "plugin `{}` exited{}",
                                plugin_id,
                                exit_status
                                    .map(|status| format!(" with status {status}"))
                                    .unwrap_or_default()
                            )
                        });
                        for panel_instance_id in process.view_instances {
                            if let Some(binding) = self.panels.remove(&panel_instance_id) {
                                if !suppress_ui_error {
                                    binding
                                        .panel
                                        .update(cx, |panel, cx| {
                                            panel.set_error(message.clone(), cx);
                                        })
                                        .ok();
                                }
                            }
                            if let Some(widget) = self.titlebar_widgets.remove(&panel_instance_id) {
                                if !suppress_ui_error {
                                    widget.widget.update(cx, |widget, cx| {
                                        widget.set_error(message.clone(), cx);
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        fn apply_message(
            &mut self,
            plugin_id: PluginId,
            message: PluginToHost,
            cx: &mut Context<Self>,
        ) -> Result<()> {
            if !matches!(&message, PluginToHost::Register { .. }) {
                let Some(process) = self.processes.get(&plugin_id) else {
                    return Ok(());
                };
                if !process.registered {
                    request_process_termination(
                        &process.terminate_sender,
                        ProcessTermination::ProtocolViolation {
                            message: format!(
                                "plugin `{plugin_id}` sent a {} message before registering with the host",
                                plugin_to_host_message_kind(&message)
                            ),
                        },
                        "message before plugin registration",
                    );
                    return Ok(());
                }
            }

            match message {
                PluginToHost::Register { plugin } => {
                    if plugin.id != plugin_id {
                        if let Some(process) = self.processes.get(&plugin_id) {
                            request_process_termination(
                                &process.terminate_sender,
                                ProcessTermination::ProtocolViolation {
                                    message: format!(
                                        "plugin `{plugin_id}` registered itself as `{}`",
                                        plugin.id
                                    ),
                                },
                                "plugin registration identity mismatch",
                            );
                        }
                        return Ok(());
                    }
                    if let Some(process) = self.processes.get_mut(&plugin_id) {
                        process.registered = true;
                    }
                    self.set_plugin_view_startup_state(
                        &plugin_id,
                        Some(RemotePluginStartupState::Starting),
                        None,
                        cx,
                    );
                }
                PluginToHost::HostRequest {
                    request_id,
                    request,
                } => {
                    let Some(process_sender) = self
                        .processes
                        .get(&plugin_id)
                        .map(|process| process.sender.clone())
                    else {
                        return Ok(());
                    };
                    let (response, error) = match handle_plugin_host_request(&plugin_id, request) {
                        Ok(response) => (Some(response), None),
                        Err(error) => (None, Some(format!("{error:#}"))),
                    };
                    send_message(
                        &process_sender,
                        &HostToPlugin::HostResponse {
                            request_id,
                            response,
                            error,
                        },
                    )?;
                }
                PluginToHost::Render {
                    panel_instance_id,
                    generation,
                    root,
                    ..
                } => {
                    if let Err(error) = validate_ui_tree_limits(&root) {
                        let message = error.to_string();
                        if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                            binding
                                .panel
                                .update(cx, |panel, cx| panel.set_error(message.clone(), cx))
                                .ok();
                        } else if let Some(widget) =
                            self.titlebar_widgets.get(&panel_instance_id).cloned()
                        {
                            widget
                                .widget
                                .update(cx, |widget, cx| widget.set_error(message.clone(), cx));
                        }
                        return Ok(());
                    }

                    if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                        match binding
                            .panel
                            .update(cx, |panel, cx| panel.update_tree(root, generation, cx))
                        {
                            Ok(()) => {}
                            Err(_) => {
                                self.detach_panel_binding(&panel_instance_id)?;
                            }
                        }
                    } else if let Some(widget) =
                        self.titlebar_widgets.get(&panel_instance_id).cloned()
                    {
                        widget
                            .widget
                            .update(cx, |widget, cx| widget.update_tree(root, generation, cx));
                    } else {
                        self.ignore_stale_view_message(&plugin_id, &panel_instance_id, "render");
                    }
                }
                PluginToHost::RenderDelta {
                    panel_instance_id,
                    generation,
                    patches,
                    ..
                } => {
                    let updated_root =
                        if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                            let mut root: Option<UiNode> = binding
                                .panel
                                .read_with(cx, |panel, _| panel.tree.clone())
                                .unwrap_or_default();
                            let Some(mut root) = root.take() else {
                                self.close_remote_view(&plugin_id, &panel_instance_id)?;
                                return Ok(());
                            };
                            apply_ui_patches(&mut root, &patches).map_err(anyhow::Error::msg)?;
                            Some((root, true))
                        } else if let Some(widget) =
                            self.titlebar_widgets.get(&panel_instance_id).cloned()
                        {
                            let Some(mut root) = widget.widget.read(cx).tree.clone() else {
                                self.close_remote_view(&plugin_id, &panel_instance_id)?;
                                return Ok(());
                            };
                            apply_ui_patches(&mut root, &patches).map_err(anyhow::Error::msg)?;
                            Some((root, false))
                        } else {
                            self.ignore_stale_view_message(
                                &plugin_id,
                                &panel_instance_id,
                                "render_delta",
                            );
                            None
                        };

                    let Some((root, is_panel_binding)) = updated_root else {
                        return Ok(());
                    };

                    if let Err(error) = validate_ui_tree_limits(&root) {
                        let message = error.to_string();
                        if is_panel_binding {
                            if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                                binding
                                    .panel
                                    .update(cx, |panel, cx| panel.set_error(message.clone(), cx))
                                    .ok();
                            }
                        } else if let Some(widget) =
                            self.titlebar_widgets.get(&panel_instance_id).cloned()
                        {
                            widget
                                .widget
                                .update(cx, |widget, cx| widget.set_error(message.clone(), cx));
                        }
                        return Ok(());
                    }

                    if is_panel_binding {
                        if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                            match binding
                                .panel
                                .update(cx, |panel, cx| panel.update_tree(root, generation, cx))
                            {
                                Ok(()) => {}
                                Err(_) => {
                                    self.detach_panel_binding(&panel_instance_id)?;
                                }
                            }
                        }
                    } else if let Some(widget) =
                        self.titlebar_widgets.get(&panel_instance_id).cloned()
                    {
                        widget
                            .widget
                            .update(cx, |widget, cx| widget.update_tree(root, generation, cx));
                    }
                }
                PluginToHost::ClosePanel { panel_instance_id } => {
                    if let Some(binding) = self.detach_panel_binding(&panel_instance_id)? {
                        binding
                            .panel
                            .update(cx, |panel, cx| {
                                panel.set_error("Plugin panel closed".to_string(), cx);
                            })
                            .ok();
                    }
                    if let Some(widget) = self.detach_titlebar_widget_binding(&panel_instance_id)? {
                        widget.widget.update(cx, |widget, cx| {
                            widget.set_error("Plugin titlebar widget closed".to_string(), cx);
                        });
                    }
                }
                PluginToHost::ReportError {
                    panel_instance_id,
                    message,
                } => {
                    if let Some(panel_instance_id) = panel_instance_id {
                        if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                            binding
                                .panel
                                .update(cx, |panel, cx| {
                                    panel.set_error(message.clone(), cx);
                                })
                                .ok();
                        } else if let Some(widget) =
                            self.titlebar_widgets.get(&panel_instance_id).cloned()
                        {
                            widget.widget.update(cx, |widget, cx| {
                                widget.set_error(message.clone(), cx);
                            });
                        } else {
                            log::error!("plugin view error for {}: {}", panel_instance_id, message);
                        }
                    } else {
                        log::error!("plugin `{}` error: {}", plugin_id, message);
                    }
                }
            }

            Ok(())
        }

        fn close_remote_view(
            &self,
            plugin_id: &PluginId,
            panel_instance_id: &PanelInstanceId,
        ) -> Result<()> {
            if let Some(process) = self.processes.get(plugin_id) {
                send_message(
                    &process.sender,
                    &HostToPlugin::ClosePanel {
                        panel_instance_id: panel_instance_id.clone(),
                    },
                )?;
            }
            Ok(())
        }

        fn ignore_stale_view_message(
            &self,
            plugin_id: &PluginId,
            panel_instance_id: &PanelInstanceId,
            message_kind: &str,
        ) {
            log::debug!(
                "ignored stale plugin {message_kind} for unattached view `{panel_instance_id}` from `{plugin_id}`"
            );
        }

        fn ensure_process(
            &mut self,
            plugin: &InstalledPlugin,
            cx: &mut Context<Self>,
        ) -> Result<channel::Sender<String>> {
            if let Some(process) = self.processes.get(&plugin.manifest.id) {
                return Ok(process.sender.clone());
            }

            let plugin_id = plugin.manifest.id.clone();
            let process_instance_id = self.next_process_instance_id;
            self.next_process_instance_id = self.next_process_instance_id.saturating_add(1);
            let spawned = spawn_process(
                plugin,
                self.event_sender.clone(),
                cx.to_async(),
                process_instance_id,
            )?;
            self.processes.insert(
                plugin_id,
                PluginProcess {
                    sender: spawned.sender.clone(),
                    terminate_sender: spawned.terminate_sender,
                    view_instances: BTreeSet::default(),
                    view_entity_ids: BTreeSet::default(),
                    instance_id: process_instance_id,
                    registered: false,
                },
            );
            Ok(spawned.sender)
        }

        fn installed_plugin(&self, plugin_id: &str) -> Result<InstalledPlugin> {
            self.list_plugins()?
                .into_iter()
                .find(|plugin| plugin.manifest.id.as_str() == plugin_id)
                .with_context(|| format!("plugin `{plugin_id}` is not installed"))
        }

        fn store(&self) -> PluginStore {
            PluginStore::new(self.layout.clone())
        }

        fn terminate_process_if_idle(&self, plugin_id: &PluginId) {
            let Some(process) = self.processes.get(plugin_id) else {
                return;
            };
            if !process.view_instances.is_empty() {
                return;
            }

            if let Err(error) = process.terminate_sender.try_send(ProcessTermination::Idle) {
                log::error!("failed to terminate idle plugin `{plugin_id}`: {error}");
            }
        }

        fn has_current_process(&self, plugin_id: &PluginId, process_instance_id: u64) -> bool {
            self.processes
                .get(plugin_id)
                .map(|process| process.instance_id == process_instance_id)
                .unwrap_or(false)
        }

        fn set_plugin_view_startup_state(
            &mut self,
            plugin_id: &PluginId,
            startup_state: Option<RemotePluginStartupState>,
            excluded_entity_id: Option<u64>,
            cx: &mut Context<Self>,
        ) {
            let panel_bindings = self
                .panels
                .values()
                .filter(|binding| &binding.plugin_id == plugin_id)
                .filter(|binding| {
                    excluded_entity_id
                        .map(|entity_id| binding.panel_entity_id.as_u64() != entity_id)
                        .unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<_>>();
            for binding in panel_bindings {
                binding
                    .panel
                    .update(cx, |panel, cx| {
                        panel.set_startup_state(startup_state, cx);
                    })
                    .ok();
            }

            let widget_bindings = self
                .titlebar_widgets
                .values()
                .filter(|binding| &binding.plugin_id == plugin_id)
                .filter(|binding| {
                    excluded_entity_id
                        .map(|entity_id| binding.widget.entity_id().as_u64() != entity_id)
                        .unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<_>>();
            for binding in widget_bindings {
                binding.widget.update(cx, |widget, cx| {
                    widget.set_startup_state(startup_state, cx);
                });
            }
        }

        fn cancel_plugin_startup(
            &mut self,
            plugin_id: &PluginId,
            excluded_entity_id: Option<u64>,
            cx: &mut Context<Self>,
        ) -> Result<()> {
            let Some(process) = self.processes.get(plugin_id) else {
                anyhow::bail!("plugin `{plugin_id}` is not running");
            };

            process
                .terminate_sender
                .try_send(ProcessTermination::StartupCancelled)
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .with_context(|| format!("failed to cancel startup for plugin `{plugin_id}`"))?;

            self.set_plugin_view_startup_state(
                plugin_id,
                Some(RemotePluginStartupState::Cancelling),
                excluded_entity_id,
                cx,
            );

            Ok(())
        }
    }

    struct PluginProcess {
        sender: channel::Sender<String>,
        terminate_sender: channel::Sender<ProcessTermination>,
        view_instances: BTreeSet<PanelInstanceId>,
        view_entity_ids: BTreeSet<gpui::EntityId>,
        instance_id: u64,
        registered: bool,
    }

    struct SpawnedPluginProcess {
        sender: channel::Sender<String>,
        terminate_sender: channel::Sender<ProcessTermination>,
    }

    const MAX_PLUGIN_STDIO_LINE_BYTES: usize = 1_048_576;
    const HOST_SECURE_STORAGE_SERVICE: &str = "neo-zed.plugin-host";

    #[cfg(target_os = "macos")]
    mod secure_storage {
        #![allow(non_upper_case_globals)]

        use super::*;

        #[link(name = "Security", kind = "framework")]
        unsafe extern "C" {
            static kSecClass: CFStringRef;
            static kSecClassGenericPassword: CFStringRef;
            static kSecAttrService: CFStringRef;
            static kSecAttrAccount: CFStringRef;
            static kSecValueData: CFStringRef;
            static kSecReturnData: CFStringRef;

            fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
            fn SecItemUpdate(query: CFDictionaryRef, attributes: CFDictionaryRef) -> OSStatus;
            fn SecItemDelete(query: CFDictionaryRef) -> OSStatus;
            fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
        }

        const errSecSuccess: OSStatus = 0;
        const errSecItemNotFound: OSStatus = -25300;

        unsafe fn base_query(
            service: &CFString,
            account: &CFString,
        ) -> CFMutableDictionary<*const core::ffi::c_void, *const core::ffi::c_void> {
            let mut attrs = CFMutableDictionary::with_capacity(3);
            unsafe {
                attrs.set(kSecClass as *const _, kSecClassGenericPassword as *const _);
                attrs.set(kSecAttrService as *const _, service.as_CFTypeRef());
                attrs.set(kSecAttrAccount as *const _, account.as_CFTypeRef());
            }
            attrs
        }

        pub fn load(service: &str, account: &str) -> Result<Option<String>> {
            let service = CFString::from(service);
            let account = CFString::from(account);
            let cf_true = CFBoolean::true_value().as_CFTypeRef();

            unsafe {
                let mut query = base_query(&service, &account);
                query.set(kSecReturnData as *const _, cf_true);

                let mut result = CFTypeRef::from(ptr::null());
                let status = SecItemCopyMatching(query.as_concrete_TypeRef(), &mut result);
                match status {
                    errSecSuccess => {}
                    errSecItemNotFound => return Ok(None),
                    _ => anyhow::bail!("reading keychain item failed: {status}"),
                }

                let data = CFType::wrap_under_create_rule(result)
                    .downcast::<CFData>()
                    .context("keychain item payload was not data")?;
                let value = String::from_utf8(data.bytes().to_vec())
                    .context("keychain item payload was not valid utf-8")?;
                Ok(Some(value))
            }
        }

        pub fn store(service: &str, account: &str, value: &str) -> Result<()> {
            let service = CFString::from(service);
            let account = CFString::from(account);
            let value = CFData::from_buffer(value.as_bytes());

            unsafe {
                let query = base_query(&service, &account);

                let mut update = CFMutableDictionary::with_capacity(1);
                update.set(kSecValueData as *const _, value.as_CFTypeRef());

                let mut status =
                    SecItemUpdate(query.as_concrete_TypeRef(), update.as_concrete_TypeRef());
                if status == errSecItemNotFound {
                    let mut attrs = base_query(&service, &account);
                    attrs.set(kSecValueData as *const _, value.as_CFTypeRef());
                    status = SecItemAdd(attrs.as_concrete_TypeRef(), ptr::null_mut());
                }

                anyhow::ensure!(
                    status == errSecSuccess,
                    "writing keychain item failed: {status}"
                );
                Ok(())
            }
        }

        pub fn clear(service: &str, account: &str) -> Result<bool> {
            let service = CFString::from(service);
            let account = CFString::from(account);

            unsafe {
                let query = base_query(&service, &account);
                let status = SecItemDelete(query.as_concrete_TypeRef());
                match status {
                    errSecSuccess => Ok(true),
                    errSecItemNotFound => Ok(false),
                    _ => anyhow::bail!("deleting keychain item failed: {status}"),
                }
            }
        }
    }

    #[derive(Clone, Debug)]
    enum ProcessTermination {
        Idle,
        StartupCancelled,
        RegistrationTimedOut,
        ProtocolViolation { message: String },
    }

    impl ProcessTermination {
        fn suppresses_ui_error(&self) -> bool {
            matches!(self, Self::StartupCancelled)
        }

        fn error_message(self, plugin_id: &PluginId) -> Option<String> {
            match self {
                Self::Idle => None,
                Self::StartupCancelled => None,
                Self::RegistrationTimedOut => Some(format!(
                    "plugin `{plugin_id}` did not register with the host before the startup timeout"
                )),
                Self::ProtocolViolation { message } => Some(message),
            }
        }
    }

    fn is_cargo_backed_plugin(plugin: &InstalledPlugin) -> bool {
        plugin.installation.root.join("Cargo.toml").exists()
    }

    fn initial_startup_state_for_plugin(
        plugin: &InstalledPlugin,
        registered: bool,
    ) -> Option<RemotePluginStartupState> {
        (!registered && is_cargo_backed_plugin(plugin))
            .then_some(RemotePluginStartupState::Building)
    }

    async fn read_bounded_line<R>(reader: &mut R, max_bytes: usize) -> Result<Option<String>>
    where
        R: futures::io::AsyncBufRead + Unpin,
    {
        let mut line = Vec::new();

        loop {
            let available = reader.fill_buf().await?;
            if available.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                break;
            }

            let newline_found = available.iter().position(|byte| *byte == b'\n');
            let bytes_to_take = newline_found
                .map(|newline_index| newline_index + 1)
                .unwrap_or(available.len());

            if line.len().saturating_add(bytes_to_take) > max_bytes {
                anyhow::bail!("protocol line exceeded {max_bytes} bytes");
            }

            line.extend_from_slice(&available[..bytes_to_take]);
            reader.consume_unpin(bytes_to_take);

            if newline_found.is_some() {
                break;
            }
        }

        if line.ends_with(b"\n") {
            line.pop();
        }
        if line.ends_with(b"\r") {
            line.pop();
        }

        Ok(Some(
            String::from_utf8(line).context("protocol line was not valid UTF-8")?,
        ))
    }

    fn request_process_termination(
        terminate_sender: &channel::Sender<ProcessTermination>,
        reason: ProcessTermination,
        context: &str,
    ) {
        if let Err(error) = terminate_sender.try_send(reason) {
            log::error!("failed to request plugin termination after {context}: {error}");
        }
    }

    fn plugin_to_host_message_kind(message: &PluginToHost) -> &'static str {
        match message {
            PluginToHost::Register { .. } => "register",
            PluginToHost::HostRequest { .. } => "host request",
            PluginToHost::Render { .. } => "render",
            PluginToHost::RenderDelta { .. } => "render delta",
            PluginToHost::ClosePanel { .. } => "close panel",
            PluginToHost::ReportError { .. } => "report error",
        }
    }

    async fn forward_plugin_stdout<R>(
        mut reader: R,
        event_sender: channel::Sender<PluginHostEvent>,
        terminate_sender: channel::Sender<ProcessTermination>,
        plugin_id: PluginId,
        process_instance_id: u64,
    ) -> Result<()>
    where
        R: futures::io::AsyncBufRead + Unpin,
    {
        loop {
            let line = match read_bounded_line(&mut reader, MAX_PLUGIN_STDIO_LINE_BYTES).await {
                Ok(line) => line,
                Err(error) => {
                    request_process_termination(
                        &terminate_sender,
                        ProcessTermination::ProtocolViolation {
                            message: format!(
                                "plugin `{}` sent an invalid stdout protocol message: {error:#}",
                                plugin_id
                            ),
                        },
                        "invalid stdout protocol message",
                    );
                    return Ok(());
                }
            };
            let Some(line) = line else {
                break;
            };

            let message: PluginToHost = match serde_json::from_str(line.trim())
                .context("failed to decode plugin stdout message")
            {
                Ok(message) => message,
                Err(error) => {
                    request_process_termination(
                        &terminate_sender,
                        ProcessTermination::ProtocolViolation {
                            message: format!(
                                "plugin `{}` sent an invalid stdout protocol message: {error:#}",
                                plugin_id
                            ),
                        },
                        "invalid stdout protocol message",
                    );
                    return Ok(());
                }
            };

            if let Err(error) = event_sender
                .send(PluginHostEvent::Message {
                    plugin_id: plugin_id.clone(),
                    process_instance_id,
                    message,
                })
                .await
            {
                log::error!("failed to forward plugin stdout message: {error}");
                return Ok(());
            }
        }

        Ok(())
    }

    async fn log_plugin_stderr<R>(
        mut reader: R,
        terminate_sender: channel::Sender<ProcessTermination>,
        plugin_id: PluginId,
    ) -> Result<()>
    where
        R: futures::io::AsyncBufRead + Unpin,
    {
        loop {
            let line = match read_bounded_line(&mut reader, MAX_PLUGIN_STDIO_LINE_BYTES).await {
                Ok(line) => line,
                Err(error) => {
                    request_process_termination(
                        &terminate_sender,
                        ProcessTermination::ProtocolViolation {
                            message: format!(
                                "plugin `{}` wrote an invalid stderr line: {error:#}",
                                plugin_id
                            ),
                        },
                        "invalid stderr output",
                    );
                    return Ok(());
                }
            };
            let Some(line) = line else {
                break;
            };
            log::warn!("plugin stderr: {}", line.trim());
        }

        Ok(())
    }

    enum PluginHostEvent {
        Message {
            plugin_id: PluginId,
            process_instance_id: u64,
            message: PluginToHost,
        },
        RegistrationTimedOut {
            plugin_id: PluginId,
            process_instance_id: u64,
        },
        ViewDetached {
            panel_instance_id: PanelInstanceId,
        },
        Exited {
            plugin_id: PluginId,
            process_instance_id: u64,
            exit_status: Option<i32>,
            error_message: Option<String>,
            suppress_ui_error: bool,
        },
    }

    pub struct RemotePluginPanel {
        plugin_id: PluginId,
        descriptor: PanelDescriptor,
        panel_instance_id: PanelInstanceId,
        activation_priority: u32,
        tree: Option<UiNode>,
        render_generation: Option<u64>,
        startup_state: Option<RemotePluginStartupState>,
        error_message: Option<SharedString>,
        event_sender: channel::Sender<PluginHostEvent>,
        focus_handle: FocusHandle,
        registry: WeakEntity<PluginHostRegistry>,
        panel_entity_id: u64,
    }

    impl RemotePluginPanel {
        fn new(
            plugin_id: impl Into<String>,
            descriptor: PanelDescriptor,
            panel_instance_id: PanelInstanceId,
            activation_priority: u32,
            registry: WeakEntity<PluginHostRegistry>,
            event_sender: channel::Sender<PluginHostEvent>,
            cx: &mut Context<Self>,
        ) -> Self {
            Self {
                plugin_id: PluginId::new(plugin_id),
                descriptor,
                panel_instance_id,
                activation_priority,
                tree: None,
                render_generation: None,
                startup_state: None,
                error_message: None,
                event_sender,
                focus_handle: cx.focus_handle(),
                registry,
                panel_entity_id: cx.entity_id().as_u64(),
            }
        }

        fn update_tree(&mut self, tree: UiNode, generation: Option<u64>, cx: &mut Context<Self>) {
            self.tree = Some(tree);
            self.render_generation = generation;
            self.startup_state = None;
            self.error_message = None;
            cx.notify();
        }

        fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
            self.tree = None;
            self.render_generation = None;
            self.startup_state = None;
            self.error_message = Some(message.into().into());
            cx.notify();
        }

        fn prepare_for_open(
            &mut self,
            startup_state: Option<RemotePluginStartupState>,
            cx: &mut Context<Self>,
        ) {
            self.tree = None;
            self.render_generation = None;
            self.startup_state = startup_state;
            self.error_message = None;
            cx.notify();
        }

        fn set_startup_state(
            &mut self,
            startup_state: Option<RemotePluginStartupState>,
            cx: &mut Context<Self>,
        ) {
            self.startup_state = startup_state;
            if startup_state.is_some() {
                self.tree = None;
                self.render_generation = None;
                self.error_message = None;
            }
            cx.notify();
        }

        fn cancel_startup(&mut self, cx: &mut Context<Self>) {
            self.set_startup_state(Some(RemotePluginStartupState::Cancelling), cx);
            let result = self.registry.update(cx, |registry, registry_cx| {
                registry.cancel_plugin_startup(
                    &self.plugin_id,
                    Some(self.panel_entity_id),
                    registry_cx,
                )
            });
            if let Err(error) = result {
                self.set_error(error.to_string(), cx);
            }
        }

        fn dispatch_event(
            &mut self,
            handler_id: EventHandlerId,
            kind: UiEventKind,
            payload: Option<serde_json::Value>,
            cx: &mut Context<Self>,
        ) {
            let event = UiEvent {
                panel_instance_id: self.panel_instance_id.clone(),
                generation: self.render_generation,
                handler_id,
                kind,
                payload,
            };
            let result = self
                .registry
                .update(cx, |registry, _cx| registry.dispatch_event(event));
            if let Err(error) = result {
                self.set_error(error.to_string(), cx);
            }
        }

        fn dispatch_click(&mut self, handler_id: EventHandlerId, cx: &mut Context<Self>) {
            let click_event = ClickEvent::default();
            self.dispatch_event(
                handler_id,
                UiEventKind::Click,
                serialize_payload(SerializedClickEvent::from(&click_event)),
                cx,
            );
        }

        fn collect_menu_items(tree: &UiNode) -> Vec<&UiNode> {
            let mut items = Vec::new();
            for child in &tree.children {
                if child.kind == UiNodeKind::MenuItem {
                    items.push(child);
                }
                items.extend(Self::collect_menu_items(child));
            }
            items
        }

        fn render_header(&self, cx: &mut Context<Self>) -> impl gpui::IntoElement {
            let menu_items: Vec<UiNode> = self
                .tree
                .as_ref()
                .map(|tree| {
                    Self::collect_menu_items(tree)
                        .into_iter()
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

            let has_menu_items = !menu_items.is_empty();
            let panel_instance_id = self.panel_instance_id.clone();

            h_flex()
                .h(Tab::container_height(cx))
                .w_full()
                .flex_none()
                .justify_between()
                .px_2()
                .bg(cx.theme().colors().tab_bar_background)
                .border_b_1()
                .border_color(cx.theme().colors().border)
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(
                            Icon::new(
                                icon_name_from_descriptor(self.descriptor.icon_name.as_deref())
                                    .unwrap_or(IconName::Box),
                            )
                            .size(IconSize::Small)
                            .color(Color::Muted),
                        )
                        .child(Label::new(self.descriptor.title.clone()).size(LabelSize::Small)),
                )
                .when(has_menu_items, |header: gpui::Div| {
                    header.child(
                        PopoverMenu::new(format!("plugin-panel-menu-{}", panel_instance_id))
                            .trigger(
                                IconButton::new(
                                    format!("plugin-panel-menu-trigger-{}", panel_instance_id),
                                    IconName::Ellipsis,
                                )
                                .icon_size(IconSize::Small),
                            )
                            .anchor(gpui::Corner::TopRight)
                            .menu({
                                let menu_items = menu_items.clone();
                                move |window, cx| {
                                    Some(ContextMenu::build(window, cx, {
                                        let menu_items = menu_items.clone();
                                        move |mut menu, _window, _| {
                                            for item in &menu_items {
                                                let label = item
                                                    .text
                                                    .clone()
                                                    .unwrap_or_else(|| "Action".to_string());
                                                let is_disabled = matches!(
                                                    item.props.get("disabled"),
                                                    Some(StyleValue::Bool(true))
                                                );
                                                if is_disabled {
                                                    menu = menu.label(label);
                                                } else if let Some(click_event) = item
                                                    .events
                                                    .iter()
                                                    .find(|event| event.event == UiEventKind::Click)
                                                {
                                                    let handler_id = click_event.handler_id.clone();
                                                    menu = menu.entry(label, None, {
                                                        let handler_id = handler_id.clone();
                                                        move |window, cx| {
                                                            window.dispatch_action(
                                                                DispatchPluginClick {
                                                                    handler_id: handler_id
                                                                        .to_string(),
                                                                }
                                                                .boxed_clone(),
                                                                cx,
                                                            );
                                                        }
                                                    });
                                                }
                                            }
                                            menu
                                        }
                                    }))
                                }
                            }),
                    )
                })
        }
    }

    impl Drop for RemotePluginPanel {
        fn drop(&mut self) {
            self.event_sender
                .try_send(PluginHostEvent::ViewDetached {
                    panel_instance_id: self.panel_instance_id.clone(),
                })
                .ok();
        }
    }

    impl EventEmitter<PanelEvent> for RemotePluginPanel {}

    impl Focusable for RemotePluginPanel {
        fn focus_handle(&self, _cx: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    impl Panel for RemotePluginPanel {
        fn persistent_name() -> &'static str {
            "Plugin Panel"
        }

        fn panel_key() -> &'static str {
            "plugin_panel"
        }

        fn panel_key_for_persistence(&self) -> SharedString {
            plugin_panel_persistence_key(self.plugin_id.as_str(), &self.descriptor.id).into()
        }

        fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
            dock_position_from_descriptor(self.descriptor.dock)
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
            self.descriptor.dock = match position {
                DockPosition::Left => PluginDockPosition::Left,
                DockPosition::Right => PluginDockPosition::Right,
                DockPosition::Bottom => PluginDockPosition::Bottom,
            };
            cx.notify();
        }

        fn default_size(&self, _window: &Window, _cx: &App) -> gpui::Pixels {
            gpui::px(320.)
        }

        fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
            icon_name_from_descriptor(self.descriptor.icon_name.as_deref()).or(Some(IconName::Box))
        }

        fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
            Some("Plugin Panel")
        }

        fn toggle_action(&self) -> Box<dyn gpui::Action> {
            ToggleRemotePluginPanel {
                panel_entity_id: self.panel_entity_id,
            }
            .boxed_clone()
        }

        fn activation_priority(&self) -> u32 {
            self.activation_priority
        }
    }

    impl Render for RemotePluginPanel {
        fn render(
            &mut self,
            _window: &mut Window,
            cx: &mut Context<Self>,
        ) -> impl gpui::IntoElement {
            let mut body = div()
                .flex()
                .flex_col()
                .size_full()
                .on_action(cx.listener(|this, action: &DispatchPluginClick, _, cx| {
                    this.dispatch_click(EventHandlerId::new(action.handler_id.clone()), cx);
                }))
                .child(self.render_header(cx));

            if let Some(error_message) = self.error_message.clone() {
                body = body.child(
                    div()
                        .px_2()
                        .py_1()
                        .child(Label::new(error_message).color(Color::Error)),
                );
            }

            let content = if let Some(tree) = self.tree.clone() {
                render_remote_node(self, &tree, &[], cx)
            } else if let Some(startup_state) = self.startup_state {
                let mut startup = v_flex()
                    .id(format!("plugin-panel-startup-{}", self.panel_instance_id))
                    .gap_2()
                    .items_center()
                    .justify_center()
                    .size_full()
                    .child(Label::new(startup_state.message()).color(Color::Muted));

                if startup_state.can_cancel() {
                    startup = startup.child(
                        Button::new(
                            format!("cancel-plugin-startup-{}", self.panel_instance_id),
                            "Cancel startup",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_startup(cx);
                        })),
                    );
                }

                startup.into_any_element()
            } else if self.error_message.is_some() {
                div().size_full().into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_full()
                    .child(Label::new("Loading...").color(Color::Muted))
                    .into_any_element()
            };

            body.child(
                div()
                    .id(format!("plugin-panel-content-{}", self.panel_instance_id))
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(content),
            )
        }
    }

    trait RemoteEventDispatcher: Sized {
        fn dispatch_plugin_event(
            &mut self,
            handler_id: EventHandlerId,
            kind: UiEventKind,
            payload: Option<serde_json::Value>,
            cx: &mut Context<Self>,
        );

        fn remote_panel_instance_id(&self) -> &PanelInstanceId;

        fn remote_entity_id(&self) -> u64;

        fn remote_div_id_prefix(&self) -> &'static str;

        fn remote_button_id_prefix(&self) -> &'static str;
    }

    impl RemoteEventDispatcher for RemotePluginTitlebarWidget {
        fn dispatch_plugin_event(
            &mut self,
            handler_id: EventHandlerId,
            kind: UiEventKind,
            payload: Option<serde_json::Value>,
            cx: &mut Context<Self>,
        ) {
            self.dispatch_event(handler_id, kind, payload, cx);
        }

        fn remote_panel_instance_id(&self) -> &PanelInstanceId {
            &self.panel_instance_id
        }

        fn remote_entity_id(&self) -> u64 {
            self.widget_entity_id
        }

        fn remote_div_id_prefix(&self) -> &'static str {
            "plugin-titlebar-div"
        }

        fn remote_button_id_prefix(&self) -> &'static str {
            "plugin-titlebar-button"
        }
    }

    #[derive(Clone)]
    struct RemoteActionBinding {
        action_type: TypeId,
        action_name: String,
        handler_id: EventHandlerId,
        phase: UiEventPhase,
    }

    struct RemoteActionBindingsElement<T> {
        child: gpui::AnyElement,
        bindings: Vec<RemoteActionBinding>,
        entity: gpui::WeakEntity<T>,
    }

    impl<T> gpui::IntoElement for RemoteActionBindingsElement<T>
    where
        T: RemoteEventDispatcher + 'static,
    {
        type Element = Self;

        fn into_element(self) -> Self::Element {
            self
        }
    }

    impl<T> gpui::Element for RemoteActionBindingsElement<T>
    where
        T: RemoteEventDispatcher + 'static,
    {
        type RequestLayoutState = ();
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _id: Option<&gpui::GlobalElementId>,
            _inspector_id: Option<&gpui::InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (gpui::LayoutId, Self::RequestLayoutState) {
            (self.child.request_layout(window, cx), ())
        }

        fn prepaint(
            &mut self,
            _id: Option<&gpui::GlobalElementId>,
            _inspector_id: Option<&gpui::InspectorElementId>,
            _bounds: gpui::Bounds<gpui::Pixels>,
            _request_layout: &mut Self::RequestLayoutState,
            window: &mut Window,
            cx: &mut App,
        ) -> Self::PrepaintState {
            self.child.prepaint(window, cx);
        }

        fn paint(
            &mut self,
            _id: Option<&gpui::GlobalElementId>,
            _inspector_id: Option<&gpui::InspectorElementId>,
            _bounds: gpui::Bounds<gpui::Pixels>,
            _request_layout: &mut Self::RequestLayoutState,
            _prepaint: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            self.child.paint(window, cx);
            for binding in self.bindings.clone() {
                let entity = self.entity.clone();
                window.on_action(
                    binding.action_type,
                    move |_action, phase, _window, cx| match (binding.phase, phase) {
                        (UiEventPhase::Bubble, gpui::DispatchPhase::Bubble)
                        | (UiEventPhase::Capture, gpui::DispatchPhase::Capture) => {
                            let payload = serialize_payload(SerializedActionEvent {
                                name: binding.action_name.clone(),
                                payload: Some(serde_json::json!({})),
                            });
                            if let Err(error) = entity.update(cx, |this, cx| {
                                this.dispatch_plugin_event(
                                    binding.handler_id.clone(),
                                    UiEventKind::Action,
                                    payload,
                                    cx,
                                );
                            }) {
                                log::error!("failed to dispatch remote action event: {error:?}");
                            }
                        }
                        (UiEventPhase::Capture, gpui::DispatchPhase::Bubble) => cx.propagate(),
                        _ => {}
                    },
                );
            }
        }
    }

    impl RemoteEventDispatcher for RemotePluginPanel {
        fn dispatch_plugin_event(
            &mut self,
            handler_id: EventHandlerId,
            kind: UiEventKind,
            payload: Option<serde_json::Value>,
            cx: &mut Context<Self>,
        ) {
            self.dispatch_event(handler_id, kind, payload, cx);
        }

        fn remote_panel_instance_id(&self) -> &PanelInstanceId {
            &self.panel_instance_id
        }

        fn remote_entity_id(&self) -> u64 {
            self.panel_entity_id
        }

        fn remote_div_id_prefix(&self) -> &'static str {
            "plugin-panel-div"
        }

        fn remote_button_id_prefix(&self) -> &'static str {
            "plugin-button"
        }
    }

    fn serialize_payload<T: serde::Serialize>(value: T) -> Option<serde_json::Value> {
        serde_json::to_value(value).ok()
    }

    fn current_theme_snapshot(cx: &App) -> HostThemeSnapshot {
        let theme = cx.theme();
        HostThemeSnapshot {
            id: theme.id.clone(),
            name: theme.name.to_string(),
            appearance: theme.appearance,
            colors: <theme::ThemeColors as refineable::Refineable>::subtract(
                &theme.styles.colors,
                &theme::ThemeColorsRefinement::default(),
            ),
            status: <theme::StatusColors as refineable::Refineable>::subtract(
                &theme.styles.status,
                &theme::StatusColorsRefinement::default(),
            ),
        }
    }

    const MAX_REMOTE_UI_NODE_COUNT: usize = 4_096;
    const MAX_REMOTE_UI_TREE_DEPTH: usize = 128;

    fn validate_ui_tree_limits(root: &UiNode) -> Result<()> {
        let mut node_count = 0usize;
        let mut stack = vec![(root, 1usize)];

        while let Some((node, depth)) = stack.pop() {
            node_count = node_count.saturating_add(1);
            if node_count > MAX_REMOTE_UI_NODE_COUNT {
                anyhow::bail!(
                    "plugin render tree exceeded the host node limit ({MAX_REMOTE_UI_NODE_COUNT})"
                );
            }
            if depth > MAX_REMOTE_UI_TREE_DEPTH {
                anyhow::bail!(
                    "plugin render tree exceeded the host depth limit ({MAX_REMOTE_UI_TREE_DEPTH})"
                );
            }

            for child in node.children.iter().rev() {
                stack.push((child, depth.saturating_add(1)));
            }
        }

        Ok(())
    }

    fn render_remote_node<T: RemoteEventDispatcher + 'static>(
        dispatcher: &T,
        node: &UiNode,
        node_path: &[usize],
        cx: &mut Context<T>,
    ) -> gpui::AnyElement {
        match node.kind {
            UiNodeKind::Empty => gpui::Empty.into_any_element(),
            UiNodeKind::Div => {
                let element = apply_styles(div(), &node.styles).children(
                    node.children.iter().enumerate().map(|(index, child)| {
                        let mut child_path = Vec::with_capacity(node_path.len().saturating_add(1));
                        child_path.extend_from_slice(node_path);
                        child_path.push(index);
                        render_remote_node(dispatcher, child, &child_path, cx)
                    }),
                );
                let fallback_div_identity = node
                    .events
                    .first()
                    .map(|event| event.handler_id.to_string())
                    .unwrap_or_else(|| format_remote_node_path(node_path));
                let div_id = remote_host_element_id(
                    dispatcher.remote_div_id_prefix(),
                    dispatcher.remote_panel_instance_id(),
                    node,
                    fallback_div_identity,
                );
                let has_interactivity_props = node_has_div_interactivity_props(node);
                let element = if node.events.is_empty() && !has_interactivity_props {
                    if remote_node_element_id(node).is_some() {
                        element.id(div_id).into_any_element()
                    } else {
                        element.into_any_element()
                    }
                } else {
                    let element = apply_div_interactivity(element.id(div_id), node);
                    let element = if node.events.is_empty() {
                        element
                    } else {
                        apply_div_events(element, node, cx)
                    };
                    element.into_any_element()
                };
                wrap_action_bindings(element, node, cx)
            }
            UiNodeKind::Label => {
                let mut label = Label::new(node.text.clone().unwrap_or_default());
                if let Some(size) = style_text(&node.props, "label_size")
                    .as_deref()
                    .and_then(label_size_from_prop)
                {
                    label = label.size(size);
                }
                if let Some(weight) = style_number(&node.props, "font_weight").map(gpui::FontWeight)
                {
                    label = label.weight(weight);
                }
                if let Some(color) = style_text(&node.props, "label_color")
                    .as_deref()
                    .and_then(color_from_prop)
                {
                    label = label.color(color);
                }
                if let Some(line_height_style) = style_text(&node.props, "line_height_style")
                    .as_deref()
                    .and_then(line_height_style_from_prop)
                {
                    label = label.line_height_style(line_height_style);
                }
                if matches!(
                    node.props.get("strikethrough"),
                    Some(StyleValue::Bool(true))
                ) {
                    label = label.strikethrough();
                }
                if matches!(node.props.get("italic"), Some(StyleValue::Bool(true))) {
                    label = label.italic();
                }
                if matches!(node.props.get("underline"), Some(StyleValue::Bool(true))) {
                    label = label.underline();
                }
                if let Some(alpha) = style_number(&node.props, "alpha") {
                    label = label.alpha(alpha);
                }
                label.into_any_element()
            }
            UiNodeKind::Button => {
                let fallback_button_identity = node
                    .events
                    .first()
                    .map(|event| event.handler_id.to_string())
                    .unwrap_or_else(|| {
                        format!(
                            "{}-{}",
                            dispatcher.remote_entity_id(),
                            format_remote_node_path(node_path)
                        )
                    });
                let button_id = remote_host_element_id(
                    dispatcher.remote_button_id_prefix(),
                    dispatcher.remote_panel_instance_id(),
                    node,
                    fallback_button_identity,
                );
                render_button_node(node, button_id, cx)
            }
            UiNodeKind::Divider => render_divider_node(node),
            UiNodeKind::Indicator => render_indicator_node(node),
            UiNodeKind::Icon => {
                let mut icon = Icon::new(
                    icon_name_from_descriptor(node.text.as_deref()).unwrap_or(IconName::AiOpenAi),
                );
                if let Some(size) = style_text(&node.props, "icon_size")
                    .as_deref()
                    .and_then(icon_size_from_prop)
                {
                    icon = icon.size(size);
                } else {
                    icon = icon.size(IconSize::Small);
                }
                if let Some(color) = style_text(&node.props, "icon_color")
                    .as_deref()
                    .and_then(color_from_prop)
                {
                    icon = icon.color(color);
                }
                icon.into_any_element()
            }
            UiNodeKind::ProgressBar => {
                render_progress_bar_node(node, dispatcher.remote_panel_instance_id(), node_path, cx)
            }
            UiNodeKind::MenuItem => gpui::Empty.into_any_element(),
        }
    }

    fn format_remote_node_path(node_path: &[usize]) -> String {
        if node_path.is_empty() {
            return "root".to_string();
        }

        let mut formatted = String::new();
        for (index, segment) in node_path.iter().enumerate() {
            if index > 0 {
                formatted.push('-');
            }
            formatted.push_str(&segment.to_string());
        }
        formatted
    }

    fn remote_node_element_id(node: &UiNode) -> Option<&str> {
        node.element_id.as_deref().or_else(|| {
            node.props.get("element_id").and_then(|value| match value {
                StyleValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
        })
    }

    fn remote_host_element_id(
        prefix: &str,
        panel_instance_id: &PanelInstanceId,
        node: &UiNode,
        fallback_identity: String,
    ) -> String {
        let identity = remote_node_element_id(node)
            .map(ToOwned::to_owned)
            .unwrap_or(fallback_identity);
        format!("{prefix}-{panel_instance_id}-{identity}")
    }

    fn remote_progress_bar_id(
        node: &UiNode,
        panel_instance_id: &PanelInstanceId,
        node_path: &[usize],
    ) -> String {
        remote_node_element_id(node)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "plugin-progress-{}-{}",
                    panel_instance_id,
                    format_remote_node_path(node_path)
                )
            })
    }

    fn wrap_action_bindings<T: RemoteEventDispatcher + 'static>(
        child: gpui::AnyElement,
        node: &UiNode,
        cx: &mut Context<T>,
    ) -> gpui::AnyElement {
        let bindings = node
            .events
            .iter()
            .filter(|event| event.event == UiEventKind::Action)
            .filter_map(|event| {
                let action_name = event.action_name.clone()?;
                let action = cx.build_action(action_name.as_str(), None).ok()?;
                Some(RemoteActionBinding {
                    action_type: action.as_any().type_id(),
                    action_name,
                    handler_id: event.handler_id.clone(),
                    phase: event.phase,
                })
            })
            .collect::<Vec<_>>();

        if bindings.is_empty() {
            child
        } else {
            RemoteActionBindingsElement {
                child,
                bindings,
                entity: cx.entity().downgrade(),
            }
            .into_any_element()
        }
    }

    fn mouse_button_from_metadata(value: Option<&str>) -> Option<gpui::MouseButton> {
        match value {
            Some("left") => Some(gpui::MouseButton::Left),
            Some("right") => Some(gpui::MouseButton::Right),
            Some("middle") => Some(gpui::MouseButton::Middle),
            Some("back") => Some(gpui::MouseButton::Navigate(gpui::NavigationDirection::Back)),
            Some("forward") => Some(gpui::MouseButton::Navigate(
                gpui::NavigationDirection::Forward,
            )),
            _ => None,
        }
    }

    fn node_has_div_interactivity_props(node: &UiNode) -> bool {
        node.props.contains_key(INTERACTIVE_PROP_GROUP)
            || node.props.contains_key(INTERACTIVE_PROP_TAB_STOP)
            || node.props.contains_key(INTERACTIVE_PROP_TAB_INDEX)
            || node.props.contains_key(INTERACTIVE_PROP_TAB_GROUP)
            || node.props.contains_key(INTERACTIVE_PROP_FOCUSABLE)
            || node.props.contains_key(INTERACTIVE_PROP_KEY_CONTEXT)
            || node
                .props
                .contains_key(INTERACTIVE_PROP_WINDOW_CONTROL_AREA)
            || node.props.contains_key(INTERACTIVE_PROP_OCCLUDE)
            || node
                .props
                .contains_key(INTERACTIVE_PROP_BLOCK_MOUSE_EXCEPT_SCROLL)
    }

    fn window_control_area_from_prop(value: &str) -> Option<gpui::WindowControlArea> {
        match value {
            "drag" => Some(gpui::WindowControlArea::Drag),
            "close" => Some(gpui::WindowControlArea::Close),
            "max" => Some(gpui::WindowControlArea::Max),
            "min" => Some(gpui::WindowControlArea::Min),
            _ => None,
        }
    }

    fn apply_div_interactivity(
        mut element: gpui::Stateful<gpui::Div>,
        node: &UiNode,
    ) -> gpui::Stateful<gpui::Div> {
        if let Some(group_name) = style_text(&node.props, INTERACTIVE_PROP_GROUP) {
            element = element.group(group_name);
        }
        if style_bool(&node.props, INTERACTIVE_PROP_TAB_GROUP).unwrap_or(false) {
            element = element.tab_group();
        }
        if let Some(tab_index) = style_isize(&node.props, INTERACTIVE_PROP_TAB_INDEX) {
            element = element.tab_index(tab_index);
        }
        if let Some(tab_stop) = style_bool(&node.props, INTERACTIVE_PROP_TAB_STOP) {
            element = element.tab_stop(tab_stop);
        }
        if style_bool(&node.props, INTERACTIVE_PROP_FOCUSABLE).unwrap_or(false) {
            element = element.focusable();
        }
        if let Some(key_context) =
            style_text(&node.props, INTERACTIVE_PROP_KEY_CONTEXT).or_else(|| {
                style_bool(&node.props, INTERACTIVE_PROP_KEY_CONTEXT)
                    .filter(|enabled| *enabled)
                    .map(|_| "Workspace".to_string())
            })
        {
            element = element.key_context(key_context.as_str());
        }
        if let Some(window_control_area) =
            style_text(&node.props, INTERACTIVE_PROP_WINDOW_CONTROL_AREA)
                .as_deref()
                .and_then(window_control_area_from_prop)
        {
            element = element.window_control_area(window_control_area);
        }
        if style_bool(&node.props, INTERACTIVE_PROP_OCCLUDE).unwrap_or(false) {
            element = element.occlude();
        }
        if style_bool(&node.props, INTERACTIVE_PROP_BLOCK_MOUSE_EXCEPT_SCROLL).unwrap_or(false) {
            element = element.block_mouse_except_scroll();
        }
        element
    }

    fn apply_div_events<T: RemoteEventDispatcher + 'static>(
        mut element: gpui::Stateful<gpui::Div>,
        node: &UiNode,
        cx: &mut Context<T>,
    ) -> gpui::Stateful<gpui::Div> {
        for event in &node.events {
            let handler_id = event.handler_id.clone();
            let mouse_button = mouse_button_from_metadata(event.mouse_button.as_deref());

            element = match (event.event, event.phase) {
                (UiEventKind::Click, _) => {
                    element.on_click(cx.listener(move |this, click, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::Click,
                            serialize_payload(SerializedClickEvent::from(click)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::AuxClick, _) => {
                    element.on_aux_click(cx.listener(move |this, click, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::AuxClick,
                            serialize_payload(SerializedClickEvent::from(click)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::Hover, _) => {
                    element.on_hover(cx.listener(move |this, hovered, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::Hover,
                            serialize_payload(*hovered),
                            cx,
                        );
                    }))
                }
                (UiEventKind::MouseDown, UiEventPhase::Bubble) => match mouse_button {
                    Some(button) => element.on_mouse_down(
                        button,
                        cx.listener(move |this, mouse_event, _, cx| {
                            this.dispatch_plugin_event(
                                handler_id.clone(),
                                UiEventKind::MouseDown,
                                serialize_payload(SerializedMouseDownEvent::from(mouse_event)),
                                cx,
                            );
                        }),
                    ),
                    None => {
                        element.on_any_mouse_down(cx.listener(move |this, mouse_event, _, cx| {
                            this.dispatch_plugin_event(
                                handler_id.clone(),
                                UiEventKind::MouseDown,
                                serialize_payload(SerializedMouseDownEvent::from(mouse_event)),
                                cx,
                            );
                        }))
                    }
                },
                (UiEventKind::MouseDown, UiEventPhase::Capture) => {
                    let expected_button = mouse_button;
                    element.capture_any_mouse_down(cx.listener(
                        move |this, mouse_event: &gpui::MouseDownEvent, _, cx| {
                            if expected_button.is_none_or(|button| button == mouse_event.button) {
                                this.dispatch_plugin_event(
                                    handler_id.clone(),
                                    UiEventKind::MouseDown,
                                    serialize_payload(SerializedMouseDownEvent::from(mouse_event)),
                                    cx,
                                );
                            }
                        },
                    ))
                }
                (UiEventKind::MouseUp, UiEventPhase::Bubble) => match mouse_button {
                    Some(button) => element.on_mouse_up(
                        button,
                        cx.listener(move |this, mouse_event, _, cx| {
                            this.dispatch_plugin_event(
                                handler_id.clone(),
                                UiEventKind::MouseUp,
                                serialize_payload(SerializedMouseUpEvent::from(mouse_event)),
                                cx,
                            );
                        }),
                    ),
                    None => element.capture_any_mouse_up(cx.listener(
                        move |this, mouse_event, _, cx| {
                            this.dispatch_plugin_event(
                                handler_id.clone(),
                                UiEventKind::MouseUp,
                                serialize_payload(SerializedMouseUpEvent::from(mouse_event)),
                                cx,
                            );
                        },
                    )),
                },
                (UiEventKind::MouseUp, UiEventPhase::Capture) => {
                    let expected_button = mouse_button;
                    element.capture_any_mouse_up(cx.listener(
                        move |this, mouse_event: &gpui::MouseUpEvent, _, cx| {
                            if expected_button.is_none_or(|button| button == mouse_event.button) {
                                this.dispatch_plugin_event(
                                    handler_id.clone(),
                                    UiEventKind::MouseUp,
                                    serialize_payload(SerializedMouseUpEvent::from(mouse_event)),
                                    cx,
                                );
                            }
                        },
                    ))
                }
                (UiEventKind::MouseMove, _) => {
                    element.on_mouse_move(cx.listener(move |this, mouse_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::MouseMove,
                            serialize_payload(SerializedMouseMoveEvent::from(mouse_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::MousePressure, UiEventPhase::Capture) => element
                    .capture_mouse_pressure(cx.listener(move |this, mouse_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::MousePressure,
                            serialize_payload(SerializedMousePressureEvent::from(mouse_event)),
                            cx,
                        );
                    })),
                (UiEventKind::MousePressure, _) => {
                    element.on_mouse_pressure(cx.listener(move |this, mouse_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::MousePressure,
                            serialize_payload(SerializedMousePressureEvent::from(mouse_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::KeyDown, UiEventPhase::Capture) => {
                    element.capture_key_down(cx.listener(move |this, key_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::KeyDown,
                            serialize_payload(SerializedKeyDownEvent::from(key_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::KeyDown, _) => {
                    element.on_key_down(cx.listener(move |this, key_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::KeyDown,
                            serialize_payload(SerializedKeyDownEvent::from(key_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::KeyUp, UiEventPhase::Capture) => {
                    element.capture_key_up(cx.listener(move |this, key_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::KeyUp,
                            serialize_payload(SerializedKeyUpEvent::from(key_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::KeyUp, _) => {
                    element.on_key_up(cx.listener(move |this, key_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::KeyUp,
                            serialize_payload(SerializedKeyUpEvent::from(key_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::ModifiersChanged, _) => {
                    element.on_modifiers_changed(cx.listener(move |this, modifier_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::ModifiersChanged,
                            serialize_payload(SerializedModifiersChangedEvent::from(
                                modifier_event,
                            )),
                            cx,
                        );
                    }))
                }
                (UiEventKind::ScrollWheel, _) => {
                    element.on_scroll_wheel(cx.listener(move |this, scroll_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::ScrollWheel,
                            serialize_payload(SerializedScrollWheelEvent::from(scroll_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::Pinch, UiEventPhase::Capture) => {
                    element.capture_pinch(cx.listener(move |this, pinch_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::Pinch,
                            serialize_payload(SerializedPinchEvent::from(pinch_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::Pinch, _) => {
                    element.on_pinch(cx.listener(move |this, pinch_event, _, cx| {
                        this.dispatch_plugin_event(
                            handler_id.clone(),
                            UiEventKind::Pinch,
                            serialize_payload(SerializedPinchEvent::from(pinch_event)),
                            cx,
                        );
                    }))
                }
                (UiEventKind::Action, _) => element,
            };
        }

        element
    }

    fn render_button_node<T: RemoteEventDispatcher + 'static>(
        node: &UiNode,
        button_id: String,
        cx: &mut Context<T>,
    ) -> gpui::AnyElement {
        let disabled = matches!(node.props.get("disabled"), Some(StyleValue::Bool(true)));
        let selected = matches!(node.props.get("selected"), Some(StyleValue::Bool(true)));
        let button_style = style_text(&node.props, "button_style")
            .as_deref()
            .and_then(button_style_from_prop);
        let selected_style = style_text(&node.props, "selected_button_style")
            .as_deref()
            .and_then(button_style_from_prop);
        let button_size = style_text(&node.props, "button_size")
            .as_deref()
            .and_then(button_size_from_prop);
        let label_color = style_text(&node.props, "label_color")
            .as_deref()
            .and_then(color_from_prop);
        let label_size = style_text(&node.props, "label_size")
            .as_deref()
            .and_then(label_size_from_prop);
        let button_layer = style_text(&node.props, "button_layer")
            .as_deref()
            .and_then(elevation_from_prop);
        let button_alpha = style_number(&node.props, "alpha");
        let tab_index = style_number(&node.props, "tab_index").map(|value| value as isize);
        let icon_name = style_text(&node.props, "button_icon")
            .as_deref()
            .and_then(|name| icon_name_from_descriptor(Some(name)));
        let icon_color = style_text(&node.props, "button_icon_color")
            .as_deref()
            .and_then(color_from_prop);
        let icon_size = style_text(&node.props, "icon_size")
            .as_deref()
            .and_then(icon_size_from_prop);
        let indicator_kind = style_text(&node.props, "indicator_kind");
        let indicator_color = style_text(&node.props, "indicator_color")
            .as_deref()
            .and_then(color_from_prop);
        let indicator_border_color = style_text(&node.props, "indicator_border_color")
            .as_deref()
            .and_then(color_from_prop);
        let indicator_stroke_color = style_text(&node.props, "indicator_stroke_color")
            .as_deref()
            .and_then(hsla_from_prop);

        if let Some(shape) = style_text(&node.props, "icon_button_shape")
            .as_deref()
            .and_then(icon_button_shape_from_prop)
        {
            let selected_icon = style_text(&node.props, "selected_button_icon")
                .as_deref()
                .and_then(|name| icon_name_from_descriptor(Some(name)));
            let selected_icon_color = style_text(&node.props, "selected_button_icon_color")
                .as_deref()
                .and_then(color_from_prop);
            let mut button = IconButton::new(
                button_id,
                icon_name.or(selected_icon).unwrap_or(IconName::Box),
            )
            .shape(shape)
            .disabled(disabled);

            if let Some(style) = button_style {
                button = button.style(style);
            }
            if let Some(size) = button_size {
                button = button.size(size);
            }
            if let Some(layer) = button_layer {
                button = button.layer(layer);
            }
            if let Some(size) = icon_size {
                button = button.icon_size(size);
            }
            if let Some(color) = icon_color {
                button = button.icon_color(color);
            }
            if let Some(icon) = selected_icon {
                button = button.selected_icon(Some(icon));
            }
            if let Some(color) = selected_icon_color {
                button = button.selected_icon_color(Some(color));
            }
            if let Some(alpha) = button_alpha {
                button = button.alpha(alpha);
            }
            if let Some(kind) = indicator_kind.as_deref() {
                let mut indicator = match kind {
                    "bar" => Indicator::bar(),
                    "icon" => Indicator::icon(Icon::new(
                        style_text(&node.props, "indicator_icon")
                            .as_deref()
                            .and_then(|name| icon_name_from_descriptor(Some(name)))
                            .unwrap_or(IconName::Circle),
                    )),
                    _ => Indicator::dot(),
                };
                if let Some(color) = indicator_color {
                    indicator = indicator.color(color);
                }
                if let Some(color) = indicator_border_color {
                    indicator = indicator.border_color(color);
                }
                button = button.indicator(indicator);
            }
            if let Some(color) = indicator_stroke_color {
                button = button.indicator_border_color(Some(color));
            }
            if selected && let Some(style) = selected_style {
                button = button.toggle_state(true).selected_style(style);
            }
            if let Some(tab_index) = tab_index {
                button = button.tab_index(tab_index);
            }
            if let Some(click_event) = node
                .events
                .iter()
                .find(|event| event.event == UiEventKind::Click)
            {
                let handler_id = click_event.handler_id.clone();
                button = button.on_click(cx.listener(move |this, click, _, cx| {
                    this.dispatch_plugin_event(
                        handler_id.clone(),
                        UiEventKind::Click,
                        serialize_payload(SerializedClickEvent::from(click)),
                        cx,
                    );
                }));
            }

            return wrap_action_bindings(button.into_any_element(), node, cx);
        }

        let button_label = if selected {
            style_text(&node.props, "selected_label")
                .or_else(|| node.text.clone())
                .unwrap_or_default()
        } else {
            node.text.clone().unwrap_or_default()
        };

        let mut button = Button::new(button_id, button_label).disabled(disabled);
        if let Some(style) = button_style {
            button = button.style(style);
        }
        if let Some(size) = button_size {
            button = button.size(size);
        }
        if let Some(color) = label_color {
            button = button.color(color);
        }
        if let Some(size) = label_size {
            button = button.label_size(Some(size));
        }
        if let Some(layer) = button_layer {
            button = button.layer(layer);
        }
        if selected && let Some(style) = selected_style {
            button = button.toggle_state(true).selected_style(style);
        }
        if let Some(alpha) = button_alpha {
            button = button.alpha(alpha);
        }
        if matches!(node.props.get("truncate"), Some(StyleValue::Bool(true))) {
            button = button.truncate(true);
        }
        if matches!(node.props.get("loading"), Some(StyleValue::Bool(true))) {
            button = button.loading(true);
        }
        if let Some(tab_index) = tab_index {
            button = button.tab_index(tab_index);
        }
        if let Some(icon_name) = icon_name {
            let mut icon = Icon::new(icon_name);
            if let Some(size) = icon_size {
                icon = icon.size(size);
            }
            if let Some(color) = icon_color {
                icon = icon.color(color);
            }
            match style_text(&node.props, "button_icon_position")
                .as_deref()
                .and_then(icon_position_from_prop)
                .unwrap_or(ui::IconPosition::Start)
            {
                ui::IconPosition::Start => {
                    button = button.start_icon(icon);
                }
                ui::IconPosition::End => {
                    button = button.end_icon(icon);
                }
            }
        }
        if let Some(click_event) = node
            .events
            .iter()
            .find(|event| event.event == UiEventKind::Click)
        {
            let handler_id = click_event.handler_id.clone();
            button = button.on_click(cx.listener(move |this, click, _, cx| {
                this.dispatch_plugin_event(
                    handler_id.clone(),
                    UiEventKind::Click,
                    serialize_payload(SerializedClickEvent::from(click)),
                    cx,
                );
            }));
        }

        wrap_action_bindings(button.into_any_element(), node, cx)
    }

    fn render_indicator_node(node: &UiNode) -> gpui::AnyElement {
        let mut indicator = match style_text(&node.props, "indicator_kind").as_deref() {
            Some("bar") => Indicator::bar(),
            Some("icon") => Indicator::icon(Icon::new(
                icon_name_from_descriptor(node.text.as_deref()).unwrap_or(IconName::Circle),
            )),
            _ => Indicator::dot(),
        };
        if let Some(color) = style_text(&node.props, "indicator_color")
            .as_deref()
            .and_then(color_from_prop)
        {
            indicator = indicator.color(color);
        }
        if let Some(color) = style_text(&node.props, "indicator_border_color")
            .as_deref()
            .and_then(color_from_prop)
        {
            indicator = indicator.border_color(color);
        }
        indicator.into_any_element()
    }

    fn render_divider_node(node: &UiNode) -> gpui::AnyElement {
        let orientation = node
            .props
            .get("orientation")
            .and_then(|value| match value {
                StyleValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
            .unwrap_or("horizontal");
        let dashed = matches!(node.props.get("dashed"), Some(StyleValue::Bool(true)));
        let mut divider = match (orientation, dashed) {
            ("vertical", true) => Divider::vertical_dashed(),
            ("vertical", false) => Divider::vertical(),
            (_, true) => Divider::horizontal_dashed(),
            _ => Divider::horizontal(),
        };
        if matches!(node.props.get("inset"), Some(StyleValue::Bool(true))) {
            divider = divider.inset();
        }
        if let Some(color) = style_text(&node.props, "divider_color")
            .as_deref()
            .and_then(divider_color_from_prop)
        {
            divider = divider.color(color);
        }
        divider.into_any_element()
    }

    fn render_progress_bar_node(
        node: &UiNode,
        panel_instance_id: &PanelInstanceId,
        node_path: &[usize],
        cx: &mut App,
    ) -> gpui::AnyElement {
        let progress_id = remote_progress_bar_id(node, panel_instance_id, node_path);
        let value = style_number(&node.props, "value").unwrap_or_default();
        let max_value = style_number(&node.props, "max_value").unwrap_or(100.);
        let mut progress = ProgressBar::new(progress_id, value, max_value, cx);
        if let Some(color) = style_text(&node.props, "bg_color")
            .as_deref()
            .and_then(hsla_from_prop)
        {
            progress = progress.bg_color(color);
        }
        if let Some(color) = style_text(&node.props, "fg_color")
            .as_deref()
            .and_then(hsla_from_prop)
        {
            progress = progress.fg_color(color);
        }
        if let Some(color) = style_text(&node.props, "over_color")
            .as_deref()
            .and_then(hsla_from_prop)
        {
            progress = progress.over_color(color);
        }
        progress.into_any_element()
    }

    fn default_layout() -> PluginStoreLayout {
        let root = paths::plugins_dir();
        PluginStoreLayout {
            installed_root: root.join("installed"),
            development_root: root.join("development"),
        }
    }

    fn dock_position_from_descriptor(position: PluginDockPosition) -> DockPosition {
        match position {
            PluginDockPosition::Left => DockPosition::Left,
            PluginDockPosition::Right => DockPosition::Right,
            PluginDockPosition::Bottom => DockPosition::Bottom,
        }
    }

    fn icon_name_from_descriptor(name: Option<&str>) -> Option<IconName> {
        let name = name?;
        name.parse::<IconName>().ok().or_else(|| match name {
            "ai_open_ai" => Some(IconName::AiOpenAi),
            "box" => Some(IconName::Box),
            "box_open" => Some(IconName::BoxOpen),
            _ => None,
        })
    }

    fn label_size_from_prop(value: &str) -> Option<ui::LabelSize> {
        match value {
            "default" => Some(ui::LabelSize::Default),
            "large" => Some(ui::LabelSize::Large),
            "small" => Some(ui::LabelSize::Small),
            "x_small" => Some(ui::LabelSize::XSmall),
            _ => None,
        }
    }

    fn line_height_style_from_prop(value: &str) -> Option<ui::LineHeightStyle> {
        match value {
            "text_label" => Some(ui::LineHeightStyle::TextLabel),
            "ui_label" => Some(ui::LineHeightStyle::UiLabel),
            _ => None,
        }
    }

    fn button_style_from_prop(value: &str) -> Option<ui::ButtonStyle> {
        match value {
            "filled" => Some(ui::ButtonStyle::Filled),
            "outlined" => Some(ui::ButtonStyle::Outlined),
            "outlined_ghost" => Some(ui::ButtonStyle::OutlinedGhost),
            "subtle" => Some(ui::ButtonStyle::Subtle),
            "transparent" => Some(ui::ButtonStyle::Transparent),
            "tinted:accent" => Some(ui::ButtonStyle::Tinted(ui::TintColor::Accent)),
            "tinted:error" => Some(ui::ButtonStyle::Tinted(ui::TintColor::Error)),
            "tinted:warning" => Some(ui::ButtonStyle::Tinted(ui::TintColor::Warning)),
            "tinted:success" => Some(ui::ButtonStyle::Tinted(ui::TintColor::Success)),
            "outlined_custom" => Some(ui::ButtonStyle::Outlined),
            _ => None,
        }
    }

    fn button_size_from_prop(value: &str) -> Option<ui::ButtonSize> {
        match value {
            "large" => Some(ui::ButtonSize::Large),
            "medium" => Some(ui::ButtonSize::Medium),
            "default" => Some(ui::ButtonSize::Default),
            "compact" => Some(ui::ButtonSize::Compact),
            "none" => Some(ui::ButtonSize::None),
            _ => None,
        }
    }

    fn elevation_from_prop(value: &str) -> Option<ui::ElevationIndex> {
        match value {
            "background" => Some(ui::ElevationIndex::Background),
            "surface" => Some(ui::ElevationIndex::Surface),
            "editor_surface" => Some(ui::ElevationIndex::EditorSurface),
            "elevated_surface" => Some(ui::ElevationIndex::ElevatedSurface),
            "modal_surface" => Some(ui::ElevationIndex::ModalSurface),
            _ => None,
        }
    }

    fn icon_position_from_prop(value: &str) -> Option<ui::IconPosition> {
        match value {
            "start" => Some(ui::IconPosition::Start),
            "end" => Some(ui::IconPosition::End),
            _ => None,
        }
    }

    fn icon_button_shape_from_prop(value: &str) -> Option<ui::IconButtonShape> {
        match value {
            "square" => Some(ui::IconButtonShape::Square),
            "wide" => Some(ui::IconButtonShape::Wide),
            _ => None,
        }
    }

    fn icon_size_from_prop(value: &str) -> Option<ui::IconSize> {
        match value {
            "indicator" => Some(ui::IconSize::Indicator),
            "x_small" => Some(ui::IconSize::XSmall),
            "small" => Some(ui::IconSize::Small),
            "medium" => Some(ui::IconSize::Medium),
            "x_large" => Some(ui::IconSize::XLarge),
            _ => None,
        }
    }

    fn divider_color_from_prop(value: &str) -> Option<ui::DividerColor> {
        match value {
            "border" => Some(ui::DividerColor::Border),
            "border_faded" => Some(ui::DividerColor::BorderFaded),
            "border_variant" => Some(ui::DividerColor::BorderVariant),
            _ => None,
        }
    }

    fn color_from_prop(value: &str) -> Option<ui::Color> {
        match value {
            "default" => Some(ui::Color::Default),
            "accent" => Some(ui::Color::Accent),
            "conflict" => Some(ui::Color::Conflict),
            "created" => Some(ui::Color::Created),
            "debugger" => Some(ui::Color::Debugger),
            "deleted" => Some(ui::Color::Deleted),
            "disabled" => Some(ui::Color::Disabled),
            "error" => Some(ui::Color::Error),
            "hidden" => Some(ui::Color::Hidden),
            "hint" => Some(ui::Color::Hint),
            "ignored" => Some(ui::Color::Ignored),
            "info" => Some(ui::Color::Info),
            "modified" => Some(ui::Color::Modified),
            "muted" => Some(ui::Color::Muted),
            "placeholder" => Some(ui::Color::Placeholder),
            "selected" => Some(ui::Color::Selected),
            "success" => Some(ui::Color::Success),
            "version_control_added" => Some(ui::Color::VersionControlAdded),
            "version_control_conflict" => Some(ui::Color::VersionControlConflict),
            "version_control_deleted" => Some(ui::Color::VersionControlDeleted),
            "version_control_ignored" => Some(ui::Color::VersionControlIgnored),
            "version_control_modified" => Some(ui::Color::VersionControlModified),
            "warning" => Some(ui::Color::Warning),
            _ => None,
        }
    }

    fn hsla_from_prop(value: &str) -> Option<gpui::Hsla> {
        let mut parts = value.split(',');
        let h = parts.next()?.parse().ok()?;
        let s = parts.next()?.parse().ok()?;
        let l = parts.next()?.parse().ok()?;
        let a = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(gpui::hsla(h, s, l, a))
    }

    fn style_number(props: &BTreeMap<String, StyleValue>, key: &str) -> Option<f32> {
        props.get(key).and_then(|value| match value {
            StyleValue::Number(value) => Some(*value),
            _ => None,
        })
    }

    fn style_isize(props: &BTreeMap<String, StyleValue>, key: &str) -> Option<isize> {
        let value = style_number(props, key)?;
        if !value.is_finite() {
            return None;
        }
        let rounded = value.round();
        if (rounded - value).abs() > f32::EPSILON {
            return None;
        }
        isize::try_from(rounded as i64).ok()
    }

    fn style_bool(props: &BTreeMap<String, StyleValue>, key: &str) -> Option<bool> {
        props.get(key).and_then(|value| match value {
            StyleValue::Bool(value) => Some(*value),
            _ => None,
        })
    }

    fn style_text(props: &BTreeMap<String, StyleValue>, key: &str) -> Option<String> {
        props.get(key).and_then(|value| match value {
            StyleValue::Text(value) => Some(value.clone()),
            _ => None,
        })
    }

    fn titlebar_widget_instance_id(
        plugin_id: &PluginId,
        widget_id: &str,
        workspace_id: gpui::EntityId,
    ) -> PanelInstanceId {
        PanelInstanceId::new(format!(
            "titlebar::{}::{plugin_id}::{widget_id}",
            workspace_id.as_u64()
        ))
    }

    fn plugin_panel_persistence_key(plugin_id: &str, panel_id: &str) -> String {
        format!("plugin_panel::{plugin_id}::{panel_id}")
    }

    fn ensure_panel_in_workspace(
        registry: &Entity<PluginHostRegistry>,
        plugin_id: &str,
        panel_id: &str,
        reveal: bool,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<Entity<RemotePluginPanel>> {
        if let Some(binding) =
            registry
                .read(cx)
                .existing_panel_binding(workspace.weak_handle(), plugin_id, panel_id)
        {
            if let Some(panel) = binding.panel.upgrade() {
                if reveal {
                    workspace.reveal_panel_by_id(binding.panel_entity_id, window, cx);
                }
                return Ok(panel);
            }
        }

        if let Some(restored_panel) = workspace
            .panel_for_serialized_key(&plugin_panel_persistence_key(plugin_id, panel_id), cx)
            .and_then(|panel| panel.to_any().downcast::<RemotePluginPanel>().ok())
        {
            if reveal {
                workspace.reveal_panel_by_id(restored_panel.entity_id(), window, cx);
            }

            registry.update(cx, |registry, registry_cx| {
                let panel_instance_id = restored_panel.read(registry_cx).panel_instance_id.clone();
                registry.attach_panel(
                    plugin_id,
                    panel_id,
                    panel_instance_id,
                    workspace.weak_handle(),
                    restored_panel.downgrade(),
                    restored_panel.entity_id(),
                    registry_cx,
                )
            })?;

            return Ok(restored_panel);
        }

        let instance_id = PanelInstanceId::new(uuid::Uuid::new_v4().to_string());
        let (descriptor, activation_priority, event_sender) =
            registry.update(cx, |registry, _cx| {
                Ok::<(PanelDescriptor, u32, channel::Sender<PluginHostEvent>), anyhow::Error>((
                    registry.panel_descriptor(plugin_id, panel_id)?,
                    registry.allocate_panel_activation_priority(),
                    registry.event_sender.clone(),
                ))
            })?;
        let panel = cx.new(|panel_cx| {
            RemotePluginPanel::new(
                plugin_id,
                descriptor.clone(),
                instance_id.clone(),
                activation_priority,
                registry.downgrade(),
                event_sender.clone(),
                panel_cx,
            )
        });

        workspace.add_panel(panel.clone(), window, cx);
        if reveal {
            workspace.reveal_panel_by_id(panel.entity_id(), window, cx);
        }

        registry.update(cx, |registry, registry_cx| {
            registry.attach_panel(
                plugin_id,
                panel_id,
                instance_id,
                workspace.weak_handle(),
                panel.downgrade(),
                panel.entity_id(),
                registry_cx,
            )
        })?;

        Ok(panel)
    }

    fn sync_workspace_panels_for_workspace(
        registry: &Entity<PluginHostRegistry>,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Result<()> {
        let workspace_handle = workspace.weak_handle();
        let (installed_panel_keys, startup_panels, stale_bindings) = {
            let registry_state = registry.read(cx);
            let installed_plugins = registry_state.list_plugins()?;
            let installed_panel_keys = installed_plugins
                .iter()
                .flat_map(|plugin| {
                    let plugin_id = plugin.manifest.id.clone();
                    plugin
                        .manifest
                        .panels
                        .iter()
                        .map(move |descriptor| (plugin_id.clone(), descriptor.id.clone()))
                })
                .collect::<BTreeSet<_>>();
            let startup_panels = installed_plugins
                .iter()
                .flat_map(|plugin| {
                    let plugin_id = plugin.manifest.id.clone();
                    plugin
                        .manifest
                        .panels
                        .iter()
                        .filter(|descriptor| descriptor.activation == PanelActivation::OnStartup)
                        .map(move |descriptor| (plugin_id.clone(), descriptor.id.clone()))
                })
                .collect::<Vec<_>>();
            let stale_bindings = registry_state
                .panels
                .iter()
                .filter_map(|(panel_instance_id, binding)| {
                    let binding_key = (binding.plugin_id.clone(), binding.panel_id.clone());
                    if binding.workspace == workspace_handle
                        && (!installed_panel_keys.contains(&binding_key)
                            || binding.panel.upgrade().is_none())
                    {
                        Some((panel_instance_id.clone(), binding.panel.upgrade()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            (installed_panel_keys, startup_panels, stale_bindings)
        };

        for (panel_instance_id, panel) in stale_bindings {
            if let Some(panel) = panel {
                workspace.remove_panel(&panel, window, cx);
            }
            registry.update(cx, |registry, _| {
                registry.detach_panel_binding(&panel_instance_id)?;
                Ok::<(), anyhow::Error>(())
            })?;
        }

        let bound_panel_entity_ids = registry
            .read(cx)
            .panels
            .values()
            .filter(|binding| binding.workspace == workspace_handle)
            .map(|binding| binding.panel_entity_id)
            .collect::<BTreeSet<_>>();

        let mut panels_by_key = BTreeMap::<String, Vec<WorkspaceRemotePanel>>::new();
        for panel in workspace_remote_panels(workspace, cx) {
            panels_by_key
                .entry(panel.panel_key.clone())
                .or_default()
                .push(panel);
        }

        let mut panels_to_remove = Vec::new();
        let mut restored_panels_to_attach = Vec::new();
        let mut present_panel_keys = BTreeSet::new();

        for panels in panels_by_key.into_values() {
            let Some(first_panel) = panels.first() else {
                continue;
            };
            let panel_key = (first_panel.plugin_id.clone(), first_panel.panel_id.clone());

            if !installed_panel_keys.contains(&panel_key) {
                panels_to_remove.extend(
                    panels
                        .into_iter()
                        .map(|panel| (panel.panel, panel.panel_instance_id)),
                );
                continue;
            }

            let panel_to_keep_index = panels
                .iter()
                .position(|panel| bound_panel_entity_ids.contains(&panel.entity_id))
                .unwrap_or(0);
            let panel_to_keep = &panels[panel_to_keep_index];
            present_panel_keys.insert(panel_key.clone());

            if !bound_panel_entity_ids.contains(&panel_to_keep.entity_id) {
                restored_panels_to_attach.push(panel_key);
            }

            panels_to_remove.extend(panels.into_iter().enumerate().filter_map(|(index, panel)| {
                (index != panel_to_keep_index).then_some((panel.panel, panel.panel_instance_id))
            }));
        }

        for (panel, panel_instance_id) in panels_to_remove {
            workspace.remove_panel(&panel, window, cx);
            registry.update(cx, |registry, _| {
                registry.detach_panel_binding(&panel_instance_id)?;
                Ok::<(), anyhow::Error>(())
            })?;
        }

        for (plugin_id, panel_id) in restored_panels_to_attach {
            ensure_panel_in_workspace(
                registry,
                plugin_id.as_str(),
                &panel_id,
                false,
                workspace,
                window,
                cx,
            )?;
        }

        for (plugin_id, panel_id) in startup_panels {
            let panel_key = (plugin_id.clone(), panel_id.clone());
            if !installed_panel_keys.contains(&panel_key) || present_panel_keys.contains(&panel_key)
            {
                continue;
            }

            let already_attached = registry
                .read(cx)
                .existing_panel_binding(workspace_handle.clone(), plugin_id.as_str(), &panel_id)
                .is_some();
            if !already_attached {
                ensure_panel_in_workspace(
                    registry,
                    plugin_id.as_str(),
                    &panel_id,
                    false,
                    workspace,
                    window,
                    cx,
                )?;
            }
        }

        Ok(())
    }

    #[derive(Clone)]
    struct WorkspaceRemotePanel {
        panel: Entity<RemotePluginPanel>,
        plugin_id: PluginId,
        panel_id: String,
        panel_instance_id: PanelInstanceId,
        panel_key: String,
        entity_id: gpui::EntityId,
    }

    fn workspace_remote_panels(workspace: &Workspace, cx: &App) -> Vec<WorkspaceRemotePanel> {
        let mut panels = Vec::new();
        let mut seen_entity_ids = BTreeSet::new();

        for dock in [
            workspace.left_dock(),
            workspace.right_dock(),
            workspace.bottom_dock(),
        ] {
            for panel_handle in dock.read(cx).panels() {
                let Ok(panel) = panel_handle.to_any().downcast::<RemotePluginPanel>() else {
                    continue;
                };
                if !seen_entity_ids.insert(panel.entity_id()) {
                    continue;
                }

                let snapshot = panel.read(cx);
                panels.push(WorkspaceRemotePanel {
                    panel: panel.clone(),
                    plugin_id: snapshot.plugin_id.clone(),
                    panel_id: snapshot.descriptor.id.clone(),
                    panel_instance_id: snapshot.panel_instance_id.clone(),
                    panel_key: snapshot.panel_key_for_persistence().to_string(),
                    entity_id: panel.entity_id(),
                });
            }
        }

        panels
    }

    fn apply_styles<E: Styled>(mut element: E, styles: &gpui::StyleRefinement) -> E {
        *element.style() = styles.clone();
        element
    }

    fn send_message(sender: &channel::Sender<String>, message: &HostToPlugin) -> Result<()> {
        sender
            .try_send(serde_json::to_string(message)?)
            .context("failed to queue plugin host message")
    }

    fn secure_storage_account(plugin_id: &PluginId, key: &str) -> String {
        format!("plugin:{}:{key}", plugin_id.as_str())
    }

    fn handle_plugin_host_request(
        plugin_id: &PluginId,
        request: PluginHostRequest,
    ) -> Result<PluginHostResponse> {
        match request {
            PluginHostRequest::SecureStorageLoad { key } => {
                let account = secure_storage_account(plugin_id, &key);
                let value = match read_secure_storage_value(&account) {
                    Ok(Some(value)) => {
                        log::warn!(
                            "plugin host secure storage load succeeded for plugin `{plugin_id}` account `{account}`"
                        );
                        Some(value)
                    }
                    Ok(None) => {
                        log::warn!(
                            "plugin host secure storage returned no entry for plugin `{plugin_id}` account `{account}`"
                        );
                        None
                    }
                    Err(error) => {
                        log::warn!(
                            "plugin host secure storage load failed for plugin `{plugin_id}` account `{account}`: {error}"
                        );
                        return Err(error.context(format!(
                            "failed to read secure storage key `{key}` for plugin `{plugin_id}`"
                        )));
                    }
                };
                Ok(PluginHostResponse::SecureStorageLoad { value })
            }
            PluginHostRequest::SecureStorageStore { key, value } => {
                let account = secure_storage_account(plugin_id, &key);
                write_secure_storage_value(&account, &value).with_context(|| {
                    format!("failed to write secure storage key `{key}` for plugin `{plugin_id}`")
                })?;
                log::warn!(
                    "plugin host secure storage store succeeded for plugin `{plugin_id}` account `{account}`"
                );
                Ok(PluginHostResponse::SecureStorageStore)
            }
            PluginHostRequest::SecureStorageClear { key } => {
                let account = secure_storage_account(plugin_id, &key);
                match clear_secure_storage_value(&account) {
                    Ok(true) => {
                        log::warn!(
                            "plugin host secure storage clear succeeded for plugin `{plugin_id}` account `{account}`"
                        );
                    }
                    Ok(false) => {
                        log::warn!(
                            "plugin host secure storage clear found no entry for plugin `{plugin_id}` account `{account}`"
                        );
                    }
                    Err(error) => {
                        log::warn!(
                            "plugin host secure storage clear failed for plugin `{plugin_id}` account `{account}`: {error}"
                        );
                        return Err(error.context(format!(
                            "failed to clear secure storage key `{key}` for plugin `{plugin_id}`"
                        )));
                    }
                }
                Ok(PluginHostResponse::SecureStorageClear)
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn read_secure_storage_value(account: &str) -> Result<Option<String>> {
        secure_storage::load(HOST_SECURE_STORAGE_SERVICE, account)
            .with_context(|| format!("failed to initialize secure storage for `{account}`"))
    }

    #[cfg(not(target_os = "macos"))]
    fn read_secure_storage_value(account: &str) -> Result<Option<String>> {
        let entry = keyring::Entry::new(HOST_SECURE_STORAGE_SERVICE, account)
            .map_err(anyhow::Error::new)
            .with_context(|| format!("failed to initialize secure storage for `{account}`"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow::Error::new(error)),
        }
    }

    #[cfg(target_os = "macos")]
    fn write_secure_storage_value(account: &str, value: &str) -> Result<()> {
        secure_storage::store(HOST_SECURE_STORAGE_SERVICE, account, value)
            .with_context(|| format!("failed to initialize secure storage for `{account}`"))
    }

    #[cfg(not(target_os = "macos"))]
    fn write_secure_storage_value(account: &str, value: &str) -> Result<()> {
        let entry = keyring::Entry::new(HOST_SECURE_STORAGE_SERVICE, account)
            .map_err(anyhow::Error::new)
            .with_context(|| format!("failed to initialize secure storage for `{account}`"))?;
        entry.set_password(value).map_err(anyhow::Error::new)
    }

    #[cfg(target_os = "macos")]
    fn clear_secure_storage_value(account: &str) -> Result<bool> {
        secure_storage::clear(HOST_SECURE_STORAGE_SERVICE, account)
            .with_context(|| format!("failed to initialize secure storage for `{account}`"))
    }

    #[cfg(not(target_os = "macos"))]
    fn clear_secure_storage_value(account: &str) -> Result<bool> {
        let entry = keyring::Entry::new(HOST_SECURE_STORAGE_SERVICE, account)
            .map_err(anyhow::Error::new)
            .with_context(|| format!("failed to initialize secure storage for `{account}`"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(anyhow::Error::new(error)),
        }
    }

    type PluginProcessReader = Box<dyn futures::AsyncRead + Unpin + Send>;
    type PluginProcessWriter = Box<dyn futures::AsyncWrite + Unpin + Send>;

    enum PluginChild {
        Async(smol::process::Child),
        #[cfg(target_os = "windows")]
        Windows(WindowsPluginChild),
    }

    impl PluginChild {
        fn from_async(inner: smol::process::Child) -> Self {
            Self::Async(inner)
        }

        fn take_stdin(&mut self) -> Option<PluginProcessWriter> {
            match self {
                Self::Async(child) => child
                    .stdin
                    .take()
                    .map(|stdin| Box::new(stdin) as PluginProcessWriter),
                #[cfg(target_os = "windows")]
                Self::Windows(child) => child
                    .stdin
                    .take()
                    .map(|stdin| Box::new(stdin) as PluginProcessWriter),
            }
        }

        fn take_stdout(&mut self) -> Option<PluginProcessReader> {
            match self {
                Self::Async(child) => child
                    .stdout
                    .take()
                    .map(|stdout| Box::new(stdout) as PluginProcessReader),
                #[cfg(target_os = "windows")]
                Self::Windows(child) => child
                    .stdout
                    .take()
                    .map(|stdout| Box::new(stdout) as PluginProcessReader),
            }
        }

        fn take_stderr(&mut self) -> Option<PluginProcessReader> {
            match self {
                Self::Async(child) => child
                    .stderr
                    .take()
                    .map(|stderr| Box::new(stderr) as PluginProcessReader),
                #[cfg(target_os = "windows")]
                Self::Windows(child) => child
                    .stderr
                    .take()
                    .map(|stderr| Box::new(stderr) as PluginProcessReader),
            }
        }

        async fn wait_status(&mut self) -> io::Result<std::process::ExitStatus> {
            match self {
                Self::Async(child) => child.status().await,
                #[cfg(target_os = "windows")]
                Self::Windows(child) => child.wait_status().await,
            }
        }

        fn kill(&mut self) -> io::Result<()> {
            match self {
                Self::Async(child) => child.kill(),
                #[cfg(target_os = "windows")]
                Self::Windows(child) => child.kill(),
            }
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsOwnedHandle(windows::Win32::Foundation::HANDLE);

    #[cfg(target_os = "windows")]
    impl WindowsOwnedHandle {
        fn new(handle: windows::Win32::Foundation::HANDLE) -> Self {
            Self(handle)
        }

        fn raw(&self) -> windows::Win32::Foundation::HANDLE {
            self.0
        }

        fn into_file(self) -> std::fs::File {
            let raw_handle = self.0.0 as RawHandle;
            std::mem::forget(self);
            let owned_handle = unsafe { OwnedHandle::from_raw_handle(raw_handle) };
            std::fs::File::from(owned_handle)
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for WindowsOwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
            }
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsPluginChild {
        process: WindowsOwnedHandle,
        thread: WindowsOwnedHandle,
        stdin: Option<Unblock<std::fs::File>>,
        stdout: Option<Unblock<std::fs::File>>,
        stderr: Option<Unblock<std::fs::File>>,
        job: WindowsJobObject,
    }

    #[cfg(target_os = "windows")]
    impl WindowsPluginChild {
        async fn wait_status(&mut self) -> io::Result<std::process::ExitStatus> {
            use windows::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
            use windows::Win32::System::Threading::{
                GetExitCodeProcess, INFINITE, WaitForSingleObject,
            };

            let process = self.process.raw();
            smol::unblock(move || {
                let wait_result = unsafe { WaitForSingleObject(process, INFINITE) };
                if wait_result == WAIT_FAILED {
                    return Err(io::Error::last_os_error());
                }
                if wait_result != WAIT_OBJECT_0 {
                    return Err(io::Error::other(format!(
                        "unexpected WaitForSingleObject result: {}",
                        wait_result.0
                    )));
                }

                let mut exit_code = 0_u32;
                unsafe {
                    GetExitCodeProcess(process, &mut exit_code)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                }
                Ok(std::process::ExitStatus::from_raw(exit_code))
            })
            .await
        }

        fn kill(&mut self) -> io::Result<()> {
            self.job.terminate()
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsJobObject(Option<WindowsOwnedHandle>);

    #[cfg(target_os = "windows")]
    impl WindowsJobObject {
        fn none() -> Self {
            Self(None)
        }

        fn new_kill_on_close() -> Result<Self> {
            use windows::Win32::System::JobObjects::{
                CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            };

            unsafe {
                let job = CreateJobObjectW(None, None)?;
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;
                Ok(Self(Some(WindowsOwnedHandle::new(job))))
            }
        }

        fn raw(&self) -> Option<windows::Win32::Foundation::HANDLE> {
            self.0.as_ref().map(WindowsOwnedHandle::raw)
        }

        fn terminate(&self) -> io::Result<()> {
            use windows::Win32::System::JobObjects::TerminateJobObject;

            if let Some(job) = self.raw() {
                unsafe {
                    TerminateJobObject(job, 1)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                }
            }
            Ok(())
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsOwnedSid(windows::Win32::Security::PSID);

    #[cfg(target_os = "windows")]
    impl WindowsOwnedSid {
        fn raw(&self) -> windows::Win32::Security::PSID {
            self.0
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for WindowsOwnedSid {
        fn drop(&mut self) {
            if !self.0.is_null() {
                let _ = unsafe {
                    windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
                        self.0.0,
                    )))
                };
            }
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsLocalAllocation(*mut core::ffi::c_void);

    #[cfg(target_os = "windows")]
    impl WindowsLocalAllocation {
        fn new(pointer: *mut core::ffi::c_void) -> Option<Self> {
            (!pointer.is_null()).then_some(Self(pointer))
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for WindowsLocalAllocation {
        fn drop(&mut self) {
            let _ = unsafe {
                windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
                    self.0 as isize,
                )))
            };
        }
    }

    #[cfg(target_os = "windows")]
    struct WindowsProcThreadAttributeList {
        buffer: Vec<u8>,
        raw: windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
    }

    #[cfg(target_os = "windows")]
    impl WindowsProcThreadAttributeList {
        fn new(attribute_count: u32) -> Result<Self> {
            use windows::Win32::System::Threading::InitializeProcThreadAttributeList;

            let mut size = 0_usize;
            unsafe {
                let _ = InitializeProcThreadAttributeList(None, attribute_count, None, &mut size);
            }

            let mut buffer = vec![0_u8; size];
            let raw = buffer.as_mut_ptr()
                as windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST;
            unsafe {
                InitializeProcThreadAttributeList(Some(raw), attribute_count, None, &mut size)?;
            }

            Ok(Self { buffer, raw })
        }

        fn update(
            &mut self,
            attribute: usize,
            value: *const core::ffi::c_void,
            size: usize,
        ) -> Result<()> {
            use windows::Win32::System::Threading::UpdateProcThreadAttribute;

            unsafe {
                UpdateProcThreadAttribute(self.raw, 0, attribute, Some(value), size, None, None)?;
            }
            Ok(())
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for WindowsProcThreadAttributeList {
        fn drop(&mut self) {
            use windows::Win32::System::Threading::DeleteProcThreadAttributeList;

            unsafe {
                DeleteProcThreadAttributeList(self.raw);
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[derive(Clone, Debug)]
    struct WindowsPathAccess {
        path: PathBuf,
        access_mask: u32,
        recursive: bool,
    }

    #[cfg(target_os = "windows")]
    struct WindowsSecurityCapabilities {
        app_container_sid: WindowsOwnedSid,
        capability_sids: Vec<WindowsOwnedSid>,
        sid_and_attributes: Vec<windows::Win32::Security::SID_AND_ATTRIBUTES>,
    }

    #[cfg(target_os = "windows")]
    impl WindowsSecurityCapabilities {
        fn as_raw(&mut self) -> windows::Win32::Security::SECURITY_CAPABILITIES {
            windows::Win32::Security::SECURITY_CAPABILITIES {
                AppContainerSid: self.app_container_sid.raw(),
                Capabilities: self.sid_and_attributes.as_mut_ptr(),
                CapabilityCount: self.sid_and_attributes.len() as u32,
                Reserved: 0,
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn ensure_windows_os_string_has_no_nuls(value: &OsStr) -> Result<()> {
        if value.encode_wide().any(|unit| unit == 0) {
            anyhow::bail!(
                "windows plugin launch values may not contain interior NUL bytes: {}",
                value.to_string_lossy()
            );
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn windows_wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    #[cfg(target_os = "windows")]
    fn append_windows_command_line_arg(
        command_line: &mut Vec<u16>,
        argument: &OsStr,
    ) -> Result<()> {
        ensure_windows_os_string_has_no_nuls(argument)?;
        let mut encoded = argument.encode_wide().peekable();
        // `CreateProcessW` passes this string directly to the child process, so only empty
        // arguments and arguments containing spaces or tabs need quoting for the standard argv
        // parser. Shell metacharacters are not interpreted here.
        let should_quote = encoded.peek().is_none()
            || argument
                .to_string_lossy()
                .chars()
                .any(|character| character == ' ' || character == '\t');

        if should_quote {
            command_line.push('"' as u16);
        }

        let mut trailing_backslashes = 0_usize;
        for unit in argument.encode_wide() {
            if unit == '\\' as u16 {
                trailing_backslashes += 1;
            } else {
                if unit == '"' as u16 {
                    command_line.extend((0..=trailing_backslashes).map(|_| '\\' as u16));
                }
                trailing_backslashes = 0;
            }
            command_line.push(unit);
        }

        if should_quote {
            command_line.extend((0..trailing_backslashes).map(|_| '\\' as u16));
            command_line.push('"' as u16);
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn build_windows_command_line(program: &OsStr, args: &[OsString]) -> Result<Vec<u16>> {
        ensure_windows_os_string_has_no_nuls(program)?;
        let mut command_line = Vec::new();
        command_line.push('"' as u16);
        command_line.extend(program.encode_wide());
        command_line.push('"' as u16);

        for argument in args {
            command_line.push(' ' as u16);
            append_windows_command_line_arg(&mut command_line, argument)?;
        }

        command_line.push(0);
        Ok(command_line)
    }

    #[cfg(target_os = "windows")]
    fn windows_appcontainer_name(plugin: &InstalledPlugin) -> String {
        let suffix = plugin
            .manifest
            .id
            .as_str()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        format!("ZedPlugin_{suffix}")
    }

    #[cfg(target_os = "windows")]
    fn derive_windows_capability_sids(name: &str) -> Result<Vec<WindowsOwnedSid>> {
        use windows::Win32::Security::DeriveCapabilitySidsFromName;

        let mut capability_group_sids = std::ptr::null_mut();
        let mut capability_group_sid_count = 0_u32;
        let mut capability_sids = std::ptr::null_mut();
        let mut capability_sid_count = 0_u32;
        let capability_name = windows_wide_null(OsStr::new(name));

        unsafe {
            DeriveCapabilitySidsFromName(
                windows::core::PCWSTR(capability_name.as_ptr()),
                &mut capability_group_sids,
                &mut capability_group_sid_count,
                &mut capability_sids,
                &mut capability_sid_count,
            )?;
        }

        let mut sids = Vec::new();
        unsafe {
            for index in 0..capability_group_sid_count as usize {
                sids.push(WindowsOwnedSid(*capability_group_sids.add(index)));
            }
            for index in 0..capability_sid_count as usize {
                sids.push(WindowsOwnedSid(*capability_sids.add(index)));
            }
        }

        if !capability_group_sids.is_null() {
            let _ = unsafe {
                windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
                    capability_group_sids as isize,
                )))
            };
        }
        if !capability_sids.is_null() {
            let _ = unsafe {
                windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
                    capability_sids as isize,
                )))
            };
        }

        Ok(sids)
    }

    #[cfg(target_os = "windows")]
    fn windows_security_capabilities(
        plugin: &InstalledPlugin,
    ) -> Result<WindowsSecurityCapabilities> {
        use windows::Win32::Security::Isolation::{
            CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
        };

        let appcontainer_name = windows_appcontainer_name(plugin);
        let appcontainer_name_wide = windows_wide_null(OsStr::new(&appcontainer_name));
        let plugin_name = windows_wide_null(OsStr::new(plugin.manifest.name.as_ref()));
        let app_container_sid = unsafe {
            DeriveAppContainerSidFromAppContainerName(windows::core::PCWSTR(
                appcontainer_name_wide.as_ptr(),
            ))
            .or_else(|_| {
                CreateAppContainerProfile(
                    windows::core::PCWSTR(appcontainer_name_wide.as_ptr()),
                    windows::core::PCWSTR(plugin_name.as_ptr()),
                    windows::core::PCWSTR(plugin_name.as_ptr()),
                    None,
                )
            })
        }?;

        let mut capability_sids = Vec::new();
        for capability in ["internetClient", "privateNetworkClientServer"] {
            capability_sids.extend(derive_windows_capability_sids(capability)?);
        }

        let sid_and_attributes = capability_sids
            .iter()
            .map(|sid| windows::Win32::Security::SID_AND_ATTRIBUTES {
                Sid: sid.raw(),
                Attributes: 0,
            })
            .collect();

        Ok(WindowsSecurityCapabilities {
            app_container_sid: WindowsOwnedSid(app_container_sid),
            capability_sids,
            sid_and_attributes,
        })
    }

    #[cfg(target_os = "windows")]
    fn windows_read_execute_mask() -> u32 {
        use windows::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};

        FILE_GENERIC_READ.0 | FILE_GENERIC_EXECUTE.0
    }

    #[cfg(target_os = "windows")]
    fn windows_read_write_execute_mask() -> u32 {
        use windows::Win32::Storage::FileSystem::{DELETE, FILE_GENERIC_WRITE};

        windows_read_execute_mask() | FILE_GENERIC_WRITE.0 | DELETE.0
    }

    #[cfg(target_os = "windows")]
    fn windows_record_path_access(
        entries: &mut BTreeMap<PathBuf, WindowsPathAccess>,
        path: &Path,
        access_mask: u32,
        recursive: bool,
    ) {
        let entry = entries
            .entry(path.to_path_buf())
            .or_insert_with(|| WindowsPathAccess {
                path: path.to_path_buf(),
                access_mask: 0,
                recursive: false,
            });
        entry.access_mask |= access_mask;
        entry.recursive |= recursive;
    }

    #[cfg(target_os = "windows")]
    fn windows_record_recursive_path_access(
        entries: &mut BTreeMap<PathBuf, WindowsPathAccess>,
        path: &Path,
        access_mask: u32,
    ) {
        windows_record_path_access(entries, path, access_mask, true);
        for ancestor in path.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            windows_record_path_access(entries, ancestor, windows_read_execute_mask(), false);
        }
    }

    #[cfg(target_os = "windows")]
    fn windows_cargo_workspace_paths(plugin_root: &Path) -> Vec<PathBuf> {
        plugin_root
            .ancestors()
            .filter(|ancestor| {
                ancestor.join("Cargo.toml").exists()
                    || ancestor.join("Cargo.lock").exists()
                    || ancestor.join(".cargo").exists()
            })
            .map(Path::to_path_buf)
            .collect()
    }

    #[cfg(target_os = "windows")]
    fn windows_plugin_data_dir(plugin: &InstalledPlugin) -> PathBuf {
        paths::data_dir()
            .join("plugins")
            .join(plugin.manifest.id.as_str())
    }

    #[cfg(target_os = "windows")]
    fn windows_plugin_temp_dir(plugin: &InstalledPlugin) -> PathBuf {
        paths::temp_dir()
            .join("plugins")
            .join(plugin.manifest.id.as_str())
    }

    #[cfg(target_os = "windows")]
    fn windows_access_plan(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
    ) -> BTreeMap<PathBuf, WindowsPathAccess> {
        let mut entries = BTreeMap::new();
        windows_record_recursive_path_access(
            &mut entries,
            &plugin.installation.root,
            windows_read_execute_mask(),
        );
        windows_record_recursive_path_access(
            &mut entries,
            &windows_plugin_data_dir(plugin),
            windows_read_write_execute_mask(),
        );
        windows_record_recursive_path_access(
            &mut entries,
            &windows_plugin_temp_dir(plugin),
            windows_read_write_execute_mask(),
        );

        if let Some(cargo_target_dir) = &launch_spec.cargo_target_dir {
            windows_record_recursive_path_access(
                &mut entries,
                cargo_target_dir,
                windows_read_write_execute_mask(),
            );
            for ancestor in windows_cargo_workspace_paths(&plugin.installation.root) {
                windows_record_recursive_path_access(
                    &mut entries,
                    &ancestor,
                    windows_read_execute_mask(),
                );
            }
            windows_record_recursive_path_access(
                &mut entries,
                &env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".cargo")),
                windows_read_write_execute_mask(),
            );
            windows_record_recursive_path_access(
                &mut entries,
                &env::var_os("RUSTUP_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".rustup")),
                windows_read_write_execute_mask(),
            );
            if let Some(program_parent) = Path::new(&launch_spec.program).parent() {
                windows_record_recursive_path_access(
                    &mut entries,
                    program_parent,
                    windows_read_execute_mask(),
                );
            }
        }

        entries
    }

    #[cfg(target_os = "windows")]
    fn windows_acl_error(
        context: &str,
        error: windows::Win32::Foundation::WIN32_ERROR,
    ) -> anyhow::Error {
        anyhow::anyhow!(
            "{context}: {}",
            io::Error::from_raw_os_error(error.0 as i32)
        )
    }

    #[cfg(target_os = "windows")]
    fn windows_path_has_access(
        path: &Path,
        sid: windows::Win32::Security::PSID,
        access_mask: u32,
    ) -> Result<bool> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::Security::Authorization::{
            BuildTrusteeWithSidW, GetEffectiveRightsFromAclW, GetNamedSecurityInfoW,
            SE_FILE_OBJECT, TRUSTEE_W,
        };
        use windows::Win32::Security::DACL_SECURITY_INFORMATION;

        let wide_path = windows_wide_null(path.as_os_str());
        let mut security_descriptor = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let error = unsafe {
            GetNamedSecurityInfoW(
                windows::core::PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut security_descriptor,
            )
        };
        if error != ERROR_SUCCESS {
            return Err(windows_acl_error(
                &format!("failed to read security descriptor for {}", path.display()),
                error,
            ));
        }
        let _security_descriptor = WindowsLocalAllocation::new(security_descriptor.cast());

        if dacl.is_null() {
            return Ok(false);
        }

        let mut trustee = TRUSTEE_W::default();
        unsafe {
            BuildTrusteeWithSidW(&mut trustee, Some(sid));
        }

        let mut effective_rights = 0_u32;
        let error = unsafe { GetEffectiveRightsFromAclW(dacl, &trustee, &mut effective_rights) };
        if error != ERROR_SUCCESS {
            return Err(windows_acl_error(
                &format!("failed to inspect effective rights for {}", path.display()),
                error,
            ));
        }

        Ok((effective_rights & access_mask) == access_mask)
    }

    #[cfg(target_os = "windows")]
    fn ensure_windows_path_access(
        path: &Path,
        sid: windows::Win32::Security::PSID,
        access_mask: u32,
        recursive: bool,
    ) -> Result<()> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::Security::Authorization::{
            BuildTrusteeWithSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
            ProgressInvokeNever, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW,
            SetNamedSecurityInfoW, TREE_SEC_INFO_SET, TRUSTEE_W, TreeSetNamedSecurityInfoW,
        };
        use windows::Win32::Security::{
            ACL, DACL_SECURITY_INFORMATION, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        };

        if !path.exists() || windows_path_has_access(path, sid, access_mask)? {
            return Ok(());
        }

        let wide_path = windows_wide_null(path.as_os_str());
        let mut security_descriptor = std::ptr::null_mut();
        let mut current_dacl = std::ptr::null_mut();
        let error = unsafe {
            GetNamedSecurityInfoW(
                windows::core::PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut current_dacl),
                None,
                &mut security_descriptor,
            )
        };
        if error != ERROR_SUCCESS {
            return Err(windows_acl_error(
                &format!("failed to load ACL for {}", path.display()),
                error,
            ));
        }
        let _security_descriptor = WindowsLocalAllocation::new(security_descriptor.cast());

        let mut trustee = TRUSTEE_W::default();
        unsafe {
            BuildTrusteeWithSidW(&mut trustee, Some(sid));
        }

        let explicit_access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: access_mask,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: if recursive && path.is_dir() {
                SUB_CONTAINERS_AND_OBJECTS_INHERIT.0
            } else {
                0
            },
            Trustee: trustee,
        };

        let mut updated_dacl = std::ptr::null_mut();
        let error = unsafe {
            SetEntriesInAclW(
                Some(&[explicit_access]),
                if current_dacl.is_null() {
                    None
                } else {
                    Some(current_dacl as *const ACL)
                },
                &mut updated_dacl,
            )
        };
        if error != ERROR_SUCCESS {
            return Err(windows_acl_error(
                &format!("failed to build ACL for {}", path.display()),
                error,
            ));
        }
        let _updated_dacl = WindowsLocalAllocation::new(updated_dacl.cast());

        let error = if recursive && path.is_dir() {
            unsafe {
                TreeSetNamedSecurityInfoW(
                    windows::core::PCWSTR(wide_path.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(updated_dacl as *const ACL),
                    None,
                    TREE_SEC_INFO_SET,
                    None,
                    ProgressInvokeNever,
                    None,
                )
            }
        } else {
            unsafe {
                SetNamedSecurityInfoW(
                    windows::core::PCWSTR(wide_path.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(updated_dacl as *const ACL),
                    None,
                )
            }
        };
        if error != ERROR_SUCCESS {
            return Err(windows_acl_error(
                &format!("failed to apply ACL for {}", path.display()),
                error,
            ));
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn ensure_windows_sandbox_paths(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
        sid: windows::Win32::Security::PSID,
    ) -> Result<()> {
        std::fs::create_dir_all(windows_plugin_data_dir(plugin)).with_context(|| {
            format!(
                "failed to create plugin data directory for {}",
                plugin.manifest.id.as_str()
            )
        })?;
        std::fs::create_dir_all(windows_plugin_temp_dir(plugin)).with_context(|| {
            format!(
                "failed to create plugin temp directory for {}",
                plugin.manifest.id.as_str()
            )
        })?;
        if let Some(cargo_target_dir) = &launch_spec.cargo_target_dir {
            std::fs::create_dir_all(cargo_target_dir).with_context(|| {
                format!(
                    "failed to create cargo target directory {}",
                    cargo_target_dir.display()
                )
            })?;
        }

        for access in windows_access_plan(plugin, launch_spec).into_values() {
            ensure_windows_path_access(&access.path, sid, access.access_mask, access.recursive)?;
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn create_windows_pipe_pair() -> Result<(WindowsOwnedHandle, WindowsOwnedHandle)> {
        use windows::Win32::Security::SECURITY_ATTRIBUTES;
        use windows::Win32::System::Pipes::CreatePipe;

        let mut read_handle = windows::Win32::Foundation::HANDLE::default();
        let mut write_handle = windows::Win32::Foundation::HANDLE::default();
        let security_attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: true.into(),
        };
        unsafe {
            CreatePipe(
                &mut read_handle,
                &mut write_handle,
                Some(&security_attributes),
                0,
            )?;
        }
        Ok((
            WindowsOwnedHandle::new(read_handle),
            WindowsOwnedHandle::new(write_handle),
        ))
    }

    fn spawn_plugin_child(plugin: &InstalledPlugin) -> Result<PluginChild> {
        #[cfg(target_os = "windows")]
        {
            return spawn_plugin_child_windows(plugin);
        }

        #[cfg(not(target_os = "windows"))]
        {
            let mut command = build_command(plugin)?;
            let child = command
                .spawn()
                .with_context(|| format!("failed to spawn plugin process {command:?}"))?;
            Ok(PluginChild::from_async(child))
        }
    }

    #[cfg(target_os = "windows")]
    fn spawn_plugin_child_windows(plugin: &InstalledPlugin) -> Result<PluginChild> {
        use windows::Win32::Foundation::{HANDLE_FLAGS, SetHandleInformation};
        use windows::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, CreateProcessW,
            EXTENDED_STARTUPINFO_PRESENT, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
            PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
        };

        let launch_spec = sandbox_launch_spec(plugin, build_launch_spec(plugin)?)?;
        let mut security_capabilities = windows_security_capabilities(plugin)?;
        ensure_windows_sandbox_paths(
            plugin,
            &launch_spec,
            security_capabilities.app_container_sid.raw(),
        )?;

        let job = WindowsJobObject::new_kill_on_close()?;
        let (child_stdin, parent_stdin) = create_windows_pipe_pair()?;
        let (parent_stdout, child_stdout) = create_windows_pipe_pair()?;
        let (parent_stderr, child_stderr) = create_windows_pipe_pair()?;

        unsafe {
            SetHandleInformation(
                parent_stdin.raw(),
                windows::Win32::Foundation::HANDLE_FLAG_INHERIT.0,
                HANDLE_FLAGS(0),
            )?;
            SetHandleInformation(
                parent_stdout.raw(),
                windows::Win32::Foundation::HANDLE_FLAG_INHERIT.0,
                HANDLE_FLAGS(0),
            )?;
            SetHandleInformation(
                parent_stderr.raw(),
                windows::Win32::Foundation::HANDLE_FLAG_INHERIT.0,
                HANDLE_FLAGS(0),
            )?;
        }

        let inherited_handles = [child_stdin.raw(), child_stdout.raw(), child_stderr.raw()];
        let job_handle = job.raw().ok_or_else(|| {
            anyhow::anyhow!("windows plugin sandbox failed to create a job object")
        })?;
        let mut raw_security_capabilities = security_capabilities.as_raw();
        let mut attribute_list = WindowsProcThreadAttributeList::new(3)?;
        attribute_list.update(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            (&mut raw_security_capabilities
                as *mut windows::Win32::Security::SECURITY_CAPABILITIES)
                .cast(),
            std::mem::size_of::<windows::Win32::Security::SECURITY_CAPABILITIES>(),
        )?;
        attribute_list.update(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited_handles.as_ptr().cast(),
            std::mem::size_of_val(&inherited_handles),
        )?;
        attribute_list.update(
            PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
            (&job_handle as *const windows::Win32::Foundation::HANDLE).cast(),
            std::mem::size_of::<windows::Win32::Foundation::HANDLE>(),
        )?;

        let mut startup_info = STARTUPINFOEXW {
            StartupInfo: windows::Win32::System::Threading::STARTUPINFOW::default(),
            lpAttributeList: attribute_list.raw,
        };
        startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup_info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup_info.StartupInfo.hStdInput = child_stdin.raw();
        startup_info.StartupInfo.hStdOutput = child_stdout.raw();
        startup_info.StartupInfo.hStdError = child_stderr.raw();

        let application_name = windows_wide_null(launch_spec.program.as_os_str());
        let current_dir = windows_wide_null(launch_spec.current_dir.as_os_str());
        let mut command_line =
            build_windows_command_line(launch_spec.program.as_os_str(), &launch_spec.args)?;
        let mut process_information = PROCESS_INFORMATION::default();
        unsafe {
            CreateProcessW(
                windows::core::PCWSTR(application_name.as_ptr()),
                Some(windows::core::PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB | EXTENDED_STARTUPINFO_PRESENT,
                None,
                windows::core::PCWSTR(current_dir.as_ptr()),
                (&startup_info as *const STARTUPINFOEXW).cast(),
                &mut process_information,
            )?;
        }

        drop(child_stdin);
        drop(child_stdout);
        drop(child_stderr);

        Ok(PluginChild::Windows(WindowsPluginChild {
            process: WindowsOwnedHandle::new(process_information.hProcess),
            thread: WindowsOwnedHandle::new(process_information.hThread),
            stdin: Some(Unblock::new(parent_stdin.into_file())),
            stdout: Some(Unblock::new(parent_stdout.into_file())),
            stderr: Some(Unblock::new(parent_stderr.into_file())),
            job,
        }))
    }

    fn spawn_process(
        plugin: &InstalledPlugin,
        event_sender: channel::Sender<PluginHostEvent>,
        async_cx: gpui::AsyncApp,
        process_instance_id: u64,
    ) -> Result<SpawnedPluginProcess> {
        let plugin_id = plugin.manifest.id.clone();
        let mut child = spawn_plugin_child(plugin)?;

        let stdin = child
            .take_stdin()
            .context("plugin process did not expose stdin")?;
        let stdout = child
            .take_stdout()
            .context("plugin process did not expose stdout")?;
        let stderr = child
            .take_stderr()
            .context("plugin process did not expose stderr")?;

        let (sender, receiver) = channel::unbounded::<String>();
        let (terminate_sender, terminate_receiver) = channel::unbounded::<ProcessTermination>();

        async_cx
            .background_spawn(async move {
                let mut writer = BufWriter::new(stdin);
                while let Ok(message) = receiver.recv().await {
                    writer.write_all(message.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                }
                anyhow::Ok(())
            })
            .detach();

        let stdout_sender = event_sender.clone();
        let stdout_plugin_id = plugin_id.clone();
        let stdout_terminate_sender = terminate_sender.clone();
        async_cx
            .background_spawn(async move {
                forward_plugin_stdout(
                    BufReader::new(stdout),
                    stdout_sender,
                    stdout_terminate_sender,
                    stdout_plugin_id,
                    process_instance_id,
                )
                .await
            })
            .detach();

        let stderr_terminate_sender = terminate_sender.clone();
        let stderr_plugin_id = plugin_id.clone();
        async_cx
            .background_spawn(async move {
                log_plugin_stderr(
                    BufReader::new(stderr),
                    stderr_terminate_sender,
                    stderr_plugin_id,
                )
                .await
            })
            .detach();

        let timeout_sender = event_sender.clone();
        let timeout_plugin_id = plugin_id.clone();
        let startup_timeout = registration_timeout(plugin);
        let background_executor = async_cx.background_executor().clone();
        async_cx
            .background_spawn(async move {
                background_executor.timer(startup_timeout).await;
                timeout_sender
                    .send(PluginHostEvent::RegistrationTimedOut {
                        plugin_id: timeout_plugin_id,
                        process_instance_id,
                    })
                    .await
                    .ok();
                anyhow::Ok(())
            })
            .detach();

        let exit_plugin_id = plugin_id.clone();
        async_cx
            .background_spawn(async move {
                enum ExitWaitOutcome {
                    Exited(io::Result<std::process::ExitStatus>),
                    TerminationRequested(Option<ProcessTermination>),
                }

                let mut termination_reason = None;
                let wait_outcome = match future::select(
                    Box::pin(child.wait_status()),
                    Box::pin(terminate_receiver.recv()),
                )
                .await
                {
                    Either::Left((status, _)) => ExitWaitOutcome::Exited(status),
                    Either::Right((termination, child_status)) => {
                        drop(child_status);
                        ExitWaitOutcome::TerminationRequested(termination.ok())
                    }
                };
                let exit_status = match wait_outcome {
                    ExitWaitOutcome::Exited(status) => status
                        .map(|status| status.code())
                        .unwrap_or_else(|error| {
                            log::error!(
                                "failed while waiting for plugin `{exit_plugin_id}` to exit: {error}"
                            );
                            None
                        }),
                    ExitWaitOutcome::TerminationRequested(termination) => {
                        termination_reason = termination;

                        if let Err(error) = child.kill() {
                            log::warn!(
                                "failed to kill plugin `{exit_plugin_id}` after a termination request: {error}"
                            );
                        }

                        child.wait_status()
                            .await
                            .map(|status| status.code())
                            .unwrap_or_else(|error| {
                                log::error!(
                                    "failed while waiting for plugin `{exit_plugin_id}` to exit after termination: {error}"
                                );
                                None
                            })
                    }
                };
                let suppress_ui_error = termination_reason
                    .as_ref()
                    .is_some_and(ProcessTermination::suppresses_ui_error);
                event_sender
                    .send(PluginHostEvent::Exited {
                        plugin_id: exit_plugin_id,
                        process_instance_id,
                        exit_status,
                        error_message: termination_reason
                            .and_then(|reason| reason.error_message(&plugin_id)),
                        suppress_ui_error,
                    })
                    .await
                    .ok();
            })
            .detach();

        Ok(SpawnedPluginProcess {
            sender,
            terminate_sender,
        })
    }

    fn registration_timeout(plugin: &InstalledPlugin) -> Duration {
        registration_timeout_for_plugin(plugin, cfg!(test))
    }

    fn registration_timeout_for_plugin(plugin: &InstalledPlugin, test_mode: bool) -> Duration {
        if test_mode {
            Duration::from_millis(75)
        } else if is_cargo_backed_plugin(plugin) {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(5)
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct PluginLaunchSpec {
        program: OsString,
        args: Vec<OsString>,
        current_dir: PathBuf,
        cargo_target_dir: Option<PathBuf>,
    }

    fn cargo_target_dir(plugin_root: &Path) -> PathBuf {
        plugin_root.join(".zed-plugin-target")
    }

    fn resolve_program_on_path(program: &OsStr) -> Result<PathBuf> {
        let path = env::var_os("PATH").ok_or_else(|| {
            anyhow::anyhow!(
                "failed to resolve `{}` because PATH is not set",
                program.to_string_lossy()
            )
        })?;
        let candidate_paths = env::split_paths(&path);

        #[cfg(target_os = "windows")]
        let suffixes = {
            let pathext = env::var_os("PATHEXT")
                .map(|value| {
                    env::split_paths(&value)
                        .map(|path| path.into_os_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if pathext.is_empty() {
                vec![
                    OsString::from(".exe"),
                    OsString::from(".cmd"),
                    OsString::from(".bat"),
                ]
            } else {
                pathext
            }
        };

        for directory in candidate_paths {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return Ok(candidate);
            }

            #[cfg(target_os = "windows")]
            if candidate.extension().is_none() {
                for suffix in &suffixes {
                    let mut extended_program = OsString::from(program);
                    extended_program.push(suffix);
                    let extended = directory.join(extended_program);
                    if extended.is_file() {
                        return Ok(extended);
                    }
                }
            }
        }

        anyhow::bail!("failed to resolve `{}` on PATH", program.to_string_lossy())
    }

    fn build_launch_spec(plugin: &InstalledPlugin) -> Result<PluginLaunchSpec> {
        let plugin_root = plugin.installation.root.clone();
        let cargo_manifest = plugin_root.join("Cargo.toml");

        if cargo_manifest.exists() {
            return Ok(PluginLaunchSpec {
                program: resolve_program_on_path(OsStr::new("cargo"))?.into_os_string(),
                args: vec![
                    OsString::from("run"),
                    OsString::from("--quiet"),
                    OsString::from("--manifest-path"),
                    cargo_manifest.into_os_string(),
                    OsString::from("--bin"),
                    plugin.manifest.entrypoint.as_os_str().to_os_string(),
                    OsString::from("--features"),
                    OsString::from("mirror"),
                ],
                current_dir: plugin_root.clone(),
                cargo_target_dir: Some(cargo_target_dir(&plugin_root)),
            });
        }

        let executable_path = if plugin.manifest.entrypoint.is_absolute() {
            plugin.manifest.entrypoint.clone()
        } else {
            plugin_root.join(&plugin.manifest.entrypoint)
        };

        Ok(PluginLaunchSpec {
            program: executable_path.into_os_string(),
            args: Vec::new(),
            current_dir: plugin_root,
            cargo_target_dir: None,
        })
    }

    #[cfg(target_os = "macos")]
    fn home_dir_from_environment() -> PathBuf {
        env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths::home_dir().clone())
    }

    #[cfg(target_os = "macos")]
    fn insert_path_with_canonical_alias(paths: &mut BTreeSet<PathBuf>, path: PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }

        paths.insert(path.clone());
        if let Ok(canonical_path) = path.canonicalize() {
            paths.insert(canonical_path);
        }
    }

    #[cfg(target_os = "macos")]
    fn insert_home_runtime_paths(paths: &mut BTreeSet<PathBuf>) {
        insert_path_with_canonical_alias(paths, env::temp_dir());

        if let Some(tmpdir) = env::var_os("TMPDIR").map(PathBuf::from) {
            insert_path_with_canonical_alias(paths, tmpdir);
        }

        let home_dir = home_dir_from_environment();
        if !home_dir.as_os_str().is_empty() {
            insert_path_with_canonical_alias(paths, home_dir.join("Library").join("Keychains"));
            insert_path_with_canonical_alias(paths, home_dir.join("Library").join("Preferences"));
        }
    }

    #[cfg(target_os = "macos")]
    fn sandbox_readable_paths(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
    ) -> Vec<PathBuf> {
        let mut paths = BTreeSet::from([
            PathBuf::from("/System"),
            PathBuf::from("/usr"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/Library"),
            PathBuf::from("/private/etc"),
            PathBuf::from("/private/var"),
            PathBuf::from("/dev"),
            PathBuf::from("/opt/homebrew"),
            plugin.installation.root.clone(),
        ]);
        insert_home_runtime_paths(&mut paths);

        if let Some(cargo_target_dir) = &launch_spec.cargo_target_dir {
            let home_dir = home_dir_from_environment();
            paths.insert(cargo_target_dir.clone());
            paths.extend(cargo_workspace_write_paths(&plugin.installation.root));
            if !home_dir.as_os_str().is_empty() {
                paths.insert(home_dir);
            }
            paths.insert(
                env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".cargo")),
            );
            paths.insert(
                env::var_os("RUSTUP_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".rustup")),
            );
        }

        paths.into_iter().collect()
    }

    #[cfg(target_os = "macos")]
    fn sandbox_writable_paths(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
    ) -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();
        paths.insert(plugin.installation.root.clone());
        paths.insert(
            paths::data_dir()
                .join("plugins")
                .join(plugin.manifest.id.as_str()),
        );
        paths.insert(
            paths::temp_dir()
                .join("plugins")
                .join(plugin.manifest.id.as_str()),
        );
        insert_home_runtime_paths(&mut paths);

        if let Some(cargo_target_dir) = &launch_spec.cargo_target_dir {
            paths.insert(cargo_target_dir.clone());
            paths.extend(cargo_workspace_write_paths(&plugin.installation.root));
            paths.insert(
                env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".cargo")),
            );
            paths.insert(
                env::var_os("RUSTUP_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".rustup")),
            );
        }

        paths.into_iter().collect()
    }

    #[cfg(target_os = "macos")]
    fn cargo_workspace_write_paths(plugin_root: &Path) -> Vec<PathBuf> {
        plugin_root
            .ancestors()
            .filter(|ancestor| {
                ancestor.join("Cargo.toml").exists() || ancestor.join("Cargo.lock").exists()
            })
            .map(Path::to_path_buf)
            .collect()
    }

    #[cfg(target_os = "macos")]
    fn insert_path_and_ancestors(paths: &mut BTreeSet<PathBuf>, path: &Path) {
        if !path.is_absolute() || path.as_os_str().is_empty() {
            return;
        }

        for ancestor in path.ancestors() {
            if ancestor.as_os_str().is_empty() {
                continue;
            }
            paths.insert(ancestor.to_path_buf());
        }
    }

    #[cfg(target_os = "macos")]
    fn sandbox_metadata_paths(
        readable_paths: &[PathBuf],
        writable_paths: &[PathBuf],
        launch_spec: &PluginLaunchSpec,
    ) -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();

        for path in readable_paths {
            insert_path_and_ancestors(&mut paths, path);
        }
        for path in writable_paths {
            insert_path_and_ancestors(&mut paths, path);
        }
        insert_path_and_ancestors(&mut paths, &launch_spec.current_dir);
        insert_path_and_ancestors(&mut paths, Path::new(&launch_spec.program));

        paths.into_iter().collect()
    }

    #[cfg(target_os = "macos")]
    fn sbpl_quote(value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('\"', "\\\"");
        format!("\"{escaped}\"")
    }

    #[cfg(target_os = "macos")]
    fn plugin_sandbox_process_exec_rule(launch_spec: &PluginLaunchSpec) -> String {
        if launch_spec.cargo_target_dir.is_some() {
            return String::from("(allow process-exec*)\n");
        }

        let mut allowed_paths = BTreeSet::from([
            PathBuf::from("/bin/sh"),
            PathBuf::from("/bin/bash"),
            PathBuf::from("/bin/zsh"),
            PathBuf::from("/usr/bin/env"),
            PathBuf::from("/usr/bin/open"),
        ]);

        allowed_paths.insert(PathBuf::from(&launch_spec.program));

        let mut rule = String::from("(allow process-exec\n");
        for path in allowed_paths {
            rule.push_str(&format!(
                "       (literal {})\n",
                sbpl_quote(path.as_os_str().to_string_lossy().as_ref())
            ));
        }
        rule.push_str(")\n");
        rule
    }

    #[cfg(target_os = "macos")]
    fn plugin_sandbox_profile(plugin: &InstalledPlugin, launch_spec: &PluginLaunchSpec) -> String {
        let mut profile = String::from(
            r#"(version 1)
(deny default)
(import "system.sb")
(import "com.apple.corefoundation.sb")
(corefoundation)
(allow process-info* (target self))
"#,
        );

        let readable_paths = sandbox_readable_paths(plugin, launch_spec);
        profile.push_str("(allow file-read*\n");
        for path in &readable_paths {
            profile.push_str(&format!(
                "       (subpath {})\n",
                sbpl_quote(path.as_os_str().to_string_lossy().as_ref())
            ));
        }
        profile.push_str(")\n");

        let writable_paths = sandbox_writable_paths(plugin, launch_spec);
        let metadata_paths = sandbox_metadata_paths(&readable_paths, &writable_paths, launch_spec);
        profile.push_str("(allow file-read-metadata\n");
        for path in metadata_paths {
            profile.push_str(&format!(
                "       (literal {})\n",
                sbpl_quote(path.as_os_str().to_string_lossy().as_ref())
            ));
        }
        profile.push_str(")\n");

        profile.push_str("(allow file-write*\n");
        for path in &writable_paths {
            profile.push_str(&format!(
                "       (subpath {})\n",
                sbpl_quote(path.as_os_str().to_string_lossy().as_ref())
            ));
        }
        profile.push_str(")\n");
        profile.push_str(
            r#"(allow network*)
(allow process-fork)
(allow lsopen)
(allow mach-lookup
       (global-name "com.apple.SecurityServer")
       (global-name "com.apple.SystemConfiguration.configd")
       (global-name "com.apple.coreservices.quarantine-resolver")
       (global-name "com.apple.dnssd.service")
       (global-name "com.apple.lsd.mapdb")
       (global-name "com.apple.securityd.xpc")
       (global-name "com.apple.system.opendirectoryd.api"))
"#,
        );
        profile.push_str(&plugin_sandbox_process_exec_rule(launch_spec));
        profile
    }

    #[cfg(target_os = "macos")]
    fn sandbox_launch_spec(
        plugin: &InstalledPlugin,
        launch_spec: PluginLaunchSpec,
    ) -> Result<PluginLaunchSpec> {
        let sandbox_executable = PathBuf::from("/usr/bin/sandbox-exec");
        if !sandbox_executable.exists() {
            anyhow::bail!(
                "plugin sandbox executable {} is not available",
                sandbox_executable.display()
            );
        }

        let mut args = Vec::with_capacity(launch_spec.args.len() + 3);
        args.push(OsString::from("-p"));
        args.push(OsString::from(plugin_sandbox_profile(plugin, &launch_spec)));
        args.push(launch_spec.program.clone());
        args.extend(launch_spec.args.clone());

        Ok(PluginLaunchSpec {
            program: sandbox_executable.into_os_string(),
            args,
            current_dir: launch_spec.current_dir,
            cargo_target_dir: launch_spec.cargo_target_dir,
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn sandbox_launch_spec(
        _plugin: &InstalledPlugin,
        launch_spec: PluginLaunchSpec,
    ) -> Result<PluginLaunchSpec> {
        Ok(launch_spec)
    }

    #[cfg(target_os = "linux")]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum LinuxLandlockAccess {
        ReadOnly,
        ReadWrite,
    }

    #[cfg(target_os = "linux")]
    #[derive(Debug)]
    struct LinuxLandlockRule {
        fd: OwnedFd,
        access: LinuxLandlockAccess,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct LinuxLandlockRulesetAttr {
        handled_access_fs: u64,
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct LinuxLandlockPathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_CREATE_RULESET_VERSION: usize = 1;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_RULE_PATH_BENEATH: u32 = 1;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
    #[cfg(target_os = "linux")]
    const LINUX_LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;

    #[cfg(target_os = "linux")]
    fn insert_linux_runtime_paths(paths: &mut BTreeSet<PathBuf>) {
        paths.insert(env::temp_dir());

        if let Some(tmpdir) = env::var_os("TMPDIR").map(PathBuf::from) {
            paths.insert(tmpdir);
        }

        if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
            paths.insert(runtime_dir);
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_cargo_workspace_read_paths(plugin_root: &Path) -> Vec<PathBuf> {
        plugin_root
            .ancestors()
            .filter(|ancestor| {
                ancestor.join("Cargo.toml").exists()
                    || ancestor.join("Cargo.lock").exists()
                    || ancestor.join(".cargo").exists()
            })
            .map(Path::to_path_buf)
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn linux_landlock_read_paths(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
    ) -> Vec<PathBuf> {
        let mut paths = BTreeSet::from([
            PathBuf::from("/usr"),
            PathBuf::from("/bin"),
            PathBuf::from("/sbin"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/etc"),
            PathBuf::from("/dev"),
            PathBuf::from("/proc"),
            PathBuf::from("/run"),
            plugin.installation.root.clone(),
        ]);
        if Path::new("/nix/store").exists() {
            paths.insert(PathBuf::from("/nix/store"));
        }
        insert_linux_runtime_paths(&mut paths);

        if let Some(cargo_target_dir) = &launch_spec.cargo_target_dir {
            paths.insert(cargo_target_dir.clone());
            paths.extend(linux_cargo_workspace_read_paths(&plugin.installation.root));
            paths.insert(
                env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".cargo")),
            );
            paths.insert(
                env::var_os("RUSTUP_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".rustup")),
            );
        }

        paths.into_iter().collect()
    }

    #[cfg(target_os = "linux")]
    fn linux_landlock_write_paths(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
    ) -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();
        paths.insert(plugin.installation.root.clone());
        paths.insert(
            paths::data_dir()
                .join("plugins")
                .join(plugin.manifest.id.as_str()),
        );
        paths.insert(
            paths::temp_dir()
                .join("plugins")
                .join(plugin.manifest.id.as_str()),
        );
        insert_linux_runtime_paths(&mut paths);

        if let Some(cargo_target_dir) = &launch_spec.cargo_target_dir {
            paths.insert(cargo_target_dir.clone());
            paths.insert(
                env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".cargo")),
            );
            paths.insert(
                env::var_os("RUSTUP_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| paths::home_dir().join(".rustup")),
            );
        }

        paths.into_iter().collect()
    }

    #[cfg(target_os = "linux")]
    fn open_linux_landlock_path(path: &Path) -> io::Result<OwnedFd> {
        let bytes = path.as_os_str().as_bytes();
        let path = CString::new(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "linux sandbox path contains interior NUL: {}",
                    path.display()
                ),
            )
        })?;

        let fd = unsafe { libc::open(path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    #[cfg(target_os = "linux")]
    fn push_linux_landlock_rules(
        rules: &mut Vec<LinuxLandlockRule>,
        paths: Vec<PathBuf>,
        access: LinuxLandlockAccess,
    ) -> Result<()> {
        for path in paths {
            if !path.exists() {
                continue;
            }

            let fd = open_linux_landlock_path(&path)
                .with_context(|| format!("failed to open linux sandbox path {}", path.display()))?;
            rules.push(LinuxLandlockRule { fd, access });
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn prepare_linux_landlock_rules(
        plugin: &InstalledPlugin,
        launch_spec: &PluginLaunchSpec,
    ) -> Result<Vec<LinuxLandlockRule>> {
        let mut rules = Vec::new();
        push_linux_landlock_rules(
            &mut rules,
            linux_landlock_read_paths(plugin, launch_spec),
            LinuxLandlockAccess::ReadOnly,
        )?;
        push_linux_landlock_rules(
            &mut rules,
            linux_landlock_write_paths(plugin, launch_spec),
            LinuxLandlockAccess::ReadWrite,
        )?;
        Ok(rules)
    }

    #[cfg(target_os = "linux")]
    fn linux_landlock_supported_access_fs(abi_version: i32) -> u64 {
        let mut access = LINUX_LANDLOCK_ACCESS_FS_EXECUTE
            | LINUX_LANDLOCK_ACCESS_FS_WRITE_FILE
            | LINUX_LANDLOCK_ACCESS_FS_READ_FILE
            | LINUX_LANDLOCK_ACCESS_FS_READ_DIR
            | LINUX_LANDLOCK_ACCESS_FS_REMOVE_DIR
            | LINUX_LANDLOCK_ACCESS_FS_REMOVE_FILE
            | LINUX_LANDLOCK_ACCESS_FS_MAKE_DIR
            | LINUX_LANDLOCK_ACCESS_FS_MAKE_REG
            | LINUX_LANDLOCK_ACCESS_FS_MAKE_SOCK
            | LINUX_LANDLOCK_ACCESS_FS_MAKE_FIFO
            | LINUX_LANDLOCK_ACCESS_FS_MAKE_SYM;

        if abi_version >= 2 {
            access |= LINUX_LANDLOCK_ACCESS_FS_REFER;
        }

        if abi_version >= 3 {
            access |= LINUX_LANDLOCK_ACCESS_FS_TRUNCATE;
        }

        access
    }

    #[cfg(target_os = "linux")]
    fn linux_landlock_allowed_access(abi_version: i32, access: LinuxLandlockAccess) -> u64 {
        let mut allowed = LINUX_LANDLOCK_ACCESS_FS_EXECUTE
            | LINUX_LANDLOCK_ACCESS_FS_READ_FILE
            | LINUX_LANDLOCK_ACCESS_FS_READ_DIR;

        if access == LinuxLandlockAccess::ReadWrite {
            allowed |= LINUX_LANDLOCK_ACCESS_FS_WRITE_FILE
                | LINUX_LANDLOCK_ACCESS_FS_REMOVE_DIR
                | LINUX_LANDLOCK_ACCESS_FS_REMOVE_FILE
                | LINUX_LANDLOCK_ACCESS_FS_MAKE_DIR
                | LINUX_LANDLOCK_ACCESS_FS_MAKE_REG
                | LINUX_LANDLOCK_ACCESS_FS_MAKE_SOCK
                | LINUX_LANDLOCK_ACCESS_FS_MAKE_FIFO
                | LINUX_LANDLOCK_ACCESS_FS_MAKE_SYM;

            if abi_version >= 2 {
                allowed |= LINUX_LANDLOCK_ACCESS_FS_REFER;
            }

            if abi_version >= 3 {
                allowed |= LINUX_LANDLOCK_ACCESS_FS_TRUNCATE;
            }
        }

        allowed
    }

    #[cfg(target_os = "linux")]
    fn install_linux_no_new_privs() -> io::Result<()> {
        unsafe {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn install_linux_landlock(rules: &[LinuxLandlockRule]) -> io::Result<()> {
        let abi_version = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<LinuxLandlockRulesetAttr>(),
                0,
                LINUX_LANDLOCK_CREATE_RULESET_VERSION,
            )
        };

        if abi_version < 0 {
            return Err(io::Error::last_os_error());
        }

        let abi_version = abi_version as i32;
        let handled_access_fs = linux_landlock_supported_access_fs(abi_version);
        let ruleset_attr = LinuxLandlockRulesetAttr { handled_access_fs };
        let ruleset_fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &ruleset_attr,
                std::mem::size_of::<LinuxLandlockRulesetAttr>(),
                0,
            )
        };

        if ruleset_fd < 0 {
            return Err(io::Error::last_os_error());
        }

        for rule in rules {
            let path_beneath_attr = LinuxLandlockPathBeneathAttr {
                allowed_access: linux_landlock_allowed_access(abi_version, rule.access),
                parent_fd: rule.fd.as_raw_fd(),
            };
            let result = unsafe {
                libc::syscall(
                    libc::SYS_landlock_add_rule,
                    ruleset_fd,
                    LINUX_LANDLOCK_RULE_PATH_BENEATH,
                    &path_beneath_attr,
                    0,
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                unsafe {
                    libc::close(ruleset_fd as i32);
                }
                return Err(error);
            }
        }

        let restrict_result =
            unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, 0) };
        let restrict_error = if restrict_result < 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        unsafe {
            libc::close(ruleset_fd as i32);
        }

        if let Some(error) = restrict_error {
            return Err(error);
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_AUDIT_ARCH: u32 = if cfg!(target_arch = "x86_64") {
        0xC000_003E
    } else if cfg!(target_arch = "aarch64") {
        0xC000_00B7
    } else {
        0
    };

    #[cfg(target_os = "linux")]
    const LINUX_BLOCKED_SYSCALLS: [libc::c_long; 20] = [
        libc::SYS_bpf,
        libc::SYS_clone3,
        libc::SYS_finit_module,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fsopen,
        libc::SYS_fspick,
        libc::SYS_init_module,
        libc::SYS_kexec_file_load,
        libc::SYS_kexec_load,
        libc::SYS_mount,
        libc::SYS_mount_setattr,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
        libc::SYS_perf_event_open,
        libc::SYS_pivot_root,
        libc::SYS_ptrace,
        libc::SYS_setns,
        libc::SYS_umount2,
        libc::SYS_unshare,
    ];

    #[cfg(target_os = "linux")]
    const LINUX_BPF_LD: u16 = 0x00;
    #[cfg(target_os = "linux")]
    const LINUX_BPF_W: u16 = 0x00;
    #[cfg(target_os = "linux")]
    const LINUX_BPF_ABS: u16 = 0x20;
    #[cfg(target_os = "linux")]
    const LINUX_BPF_JMP: u16 = 0x05;
    #[cfg(target_os = "linux")]
    const LINUX_BPF_JEQ: u16 = 0x10;
    #[cfg(target_os = "linux")]
    const LINUX_BPF_K: u16 = 0x00;
    #[cfg(target_os = "linux")]
    const LINUX_BPF_RET: u16 = 0x06;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_ERRNO_OPERATION_NOT_PERMITTED: u32 = 1;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_ARCH_OFFSET: u32 = 4;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_NR_OFFSET: u32 = 0;
    #[cfg(target_os = "linux")]
    const LINUX_SECCOMP_FILTER_LEN: usize = 5 + (LINUX_BLOCKED_SYSCALLS.len() * 2);

    #[cfg(target_os = "linux")]
    const fn linux_bpf_stmt(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    #[cfg(target_os = "linux")]
    const fn linux_bpf_jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }

    #[cfg(target_os = "linux")]
    fn linux_seccomp_program() -> [libc::sock_filter; LINUX_SECCOMP_FILTER_LEN] {
        let mut program = [linux_bpf_stmt(LINUX_BPF_RET | LINUX_BPF_K, LINUX_SECCOMP_RET_ALLOW);
            LINUX_SECCOMP_FILTER_LEN];
        program[0] = linux_bpf_stmt(
            LINUX_BPF_LD | LINUX_BPF_W | LINUX_BPF_ABS,
            LINUX_SECCOMP_ARCH_OFFSET,
        );
        program[1] = linux_bpf_jump(
            LINUX_BPF_JMP | LINUX_BPF_JEQ | LINUX_BPF_K,
            LINUX_SECCOMP_AUDIT_ARCH,
            1,
            0,
        );
        program[2] = linux_bpf_stmt(LINUX_BPF_RET | LINUX_BPF_K, LINUX_SECCOMP_RET_KILL_PROCESS);
        program[3] = linux_bpf_stmt(
            LINUX_BPF_LD | LINUX_BPF_W | LINUX_BPF_ABS,
            LINUX_SECCOMP_NR_OFFSET,
        );

        let mut instruction_index = 4;
        let deny_result = LINUX_SECCOMP_RET_ERRNO | LINUX_SECCOMP_ERRNO_OPERATION_NOT_PERMITTED;
        for syscall in LINUX_BLOCKED_SYSCALLS {
            program[instruction_index] = linux_bpf_jump(
                LINUX_BPF_JMP | LINUX_BPF_JEQ | LINUX_BPF_K,
                syscall as u32,
                0,
                1,
            );
            program[instruction_index + 1] =
                linux_bpf_stmt(LINUX_BPF_RET | LINUX_BPF_K, deny_result);
            instruction_index += 2;
        }

        program[instruction_index] =
            linux_bpf_stmt(LINUX_BPF_RET | LINUX_BPF_K, LINUX_SECCOMP_RET_ALLOW);
        program
    }

    #[cfg(target_os = "linux")]
    fn install_linux_seccomp() -> io::Result<()> {
        if LINUX_SECCOMP_AUDIT_ARCH == 0 {
            return Err(io::Error::other(
                "plugin seccomp sandbox does not support this linux architecture",
            ));
        }

        let program = linux_seccomp_program();
        let filter_program = libc::sock_fprog {
            len: program.len() as u16,
            filter: program.as_ptr() as *mut libc::sock_filter,
        };

        unsafe {
            if libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &filter_program,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn build_command(plugin: &InstalledPlugin) -> Result<Command> {
        let launch_spec = sandbox_launch_spec(plugin, build_launch_spec(plugin)?)?;
        let landlock_rules = prepare_linux_landlock_rules(plugin, &launch_spec)?;
        let mut std_command = std::process::Command::new(&launch_spec.program);
        std_command.args(&launch_spec.args);

        if let Some(cargo_target_dir) = launch_spec.cargo_target_dir {
            std_command.env("CARGO_TARGET_DIR", cargo_target_dir);
        }

        std_command
            .current_dir(launch_spec.current_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        unsafe {
            std_command.pre_exec(move || {
                install_linux_no_new_privs()?;
                install_linux_landlock(&landlock_rules)?;
                install_linux_seccomp()
            });
        }

        let mut command = Command::from(std_command);
        command.kill_on_drop(true);
        Ok(command)
    }

    #[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
    fn build_command(plugin: &InstalledPlugin) -> Result<Command> {
        let launch_spec = sandbox_launch_spec(plugin, build_launch_spec(plugin)?)?;
        let mut command = Command::new(&launch_spec.program);
        command.args(&launch_spec.args);

        if let Some(cargo_target_dir) = launch_spec.cargo_target_dir {
            command.env("CARGO_TARGET_DIR", cargo_target_dir);
        }

        command
            .current_dir(launch_spec.current_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        Ok(command)
    }

    #[cfg(all(test, unix))]
    mod tests {
        use super::*;
        use ::fs::FakeFs;
        use gpui::TestAppContext;
        use node_runtime::NodeRuntime;
        use plugin_protocol::PluginInstallState;
        use project::Project;
        use session::{AppSession, Session};
        use settings::SettingsStore;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::sync::Arc;
        use tempfile::TempDir;

        fn init_test_app(cx: &mut TestAppContext) {
            cx.update(|cx| {
                let settings_store = SettingsStore::test(cx);
                cx.set_global(settings_store);
                cx.set_global(db::AppDatabase::test_new());
                theme_settings::init(theme::LoadThemes::JustBase, cx);
            });
        }

        fn new_test_registry(
            layout: PluginStoreLayout,
            cx: &mut TestAppContext,
        ) -> Entity<PluginHostRegistry> {
            cx.update(|cx| {
                let (event_sender, event_receiver) = channel::unbounded();
                let registry = cx.new(|_cx| PluginHostRegistry {
                    layout,
                    processes: BTreeMap::default(),
                    panels: BTreeMap::default(),
                    titlebar_widgets: BTreeMap::default(),
                    next_panel_activation_priority: 10_000,
                    next_process_instance_id: 1,
                    event_sender,
                    _event_task: None,
                });

                let weak_registry = registry.downgrade();
                let event_task = cx.spawn(async move |cx| {
                    while let Ok(event) = event_receiver.recv().await {
                        let Ok(()) = weak_registry.update(cx, |registry, cx| {
                            registry.handle_event(event, cx);
                        }) else {
                            break;
                        };
                    }
                });

                registry.update(cx, |registry, _cx| {
                    registry._event_task = Some(event_task);
                });

                cx.set_global(GlobalPluginHost(registry.clone()));
                registry
            })
        }

        fn write_plugin(plugin_root: &Path, manifest: &str, executable_body: &str) -> Result<()> {
            fs::create_dir_all(plugin_root).with_context(|| {
                format!("failed to create fake plugin {}", plugin_root.display())
            })?;
            fs::write(plugin_root.join("plugin.toml"), manifest).with_context(|| {
                format!(
                    "failed to write {}",
                    plugin_root.join("plugin.toml").display()
                )
            })?;

            let executable_path = plugin_root.join("fake-plugin");
            fs::write(&executable_path, executable_body)
                .with_context(|| format!("failed to write {}", executable_path.display()))?;
            let mut permissions = fs::metadata(&executable_path)
                .with_context(|| format!("failed to stat {}", executable_path.display()))?
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&executable_path, permissions)
                .with_context(|| format!("failed to chmod {}", executable_path.display()))?;

            Ok(())
        }

        fn write_fake_plugin(plugin_root: &Path, manifest: &str) -> Result<()> {
            write_plugin(
                plugin_root,
                manifest,
                "#!/bin/sh\nwhile IFS= read -r _line; do\n  :\ndone\n",
            )
        }

        fn write_registering_plugin(
            plugin_root: &Path,
            manifest: &str,
            plugin_id: &str,
            plugin_name: &str,
        ) -> Result<()> {
            let register_message = serde_json::json!({
                "type": "register",
                "plugin": {
                    "id": plugin_id,
                    "name": plugin_name,
                    "version": "0.1.0",
                }
            });
            let executable_body = format!(
                "#!/bin/sh\necho '{}'\nwhile IFS= read -r _line; do\n  :\ndone\n",
                register_message
            );
            write_plugin(plugin_root, manifest, &executable_body)
        }

        fn seed_process(
            registry: &Entity<PluginHostRegistry>,
            plugin_id: &str,
            registered: bool,
            cx: &mut TestAppContext,
        ) -> (
            channel::Receiver<String>,
            channel::Receiver<ProcessTermination>,
        ) {
            let (sender, receiver) = channel::unbounded();
            let (terminate_sender, terminate_receiver) = channel::unbounded();
            let plugin_id = PluginId::new(plugin_id);
            cx.update(|cx| {
                registry.update(cx, |registry, _cx| {
                    registry.processes.insert(
                        plugin_id,
                        PluginProcess {
                            sender,
                            terminate_sender,
                            view_instances: BTreeSet::default(),
                            view_entity_ids: BTreeSet::default(),
                            instance_id: 1,
                            registered,
                        },
                    );
                });
            });
            (receiver, terminate_receiver)
        }

        fn attach_test_panel(
            registry: &Entity<PluginHostRegistry>,
            plugin_id: &str,
            panel_id: &str,
            title: &str,
            cx: &mut TestAppContext,
        ) -> Entity<RemotePluginPanel> {
            let panel_instance_id =
                PanelInstanceId::new(format!("test-panel-{plugin_id}-{panel_id}"));
            let registry_weak = registry.downgrade();
            cx.update(|cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        let descriptor = PanelDescriptor {
                            id: panel_id.to_string(),
                            title: title.to_string(),
                            dock: PluginDockPosition::Right,
                            icon_name: None,
                            tooltip: None,
                            activation: PanelActivation::OnDemand,
                        };
                        let event_sender = registry.event_sender.clone();
                        let panel = registry_cx.new(|panel_cx| {
                            RemotePluginPanel::new(
                                plugin_id,
                                descriptor.clone(),
                                panel_instance_id.clone(),
                                10_000,
                                registry_weak.clone(),
                                event_sender.clone(),
                                panel_cx,
                            )
                        });

                        registry.attach_panel(
                            plugin_id,
                            panel_id,
                            panel_instance_id,
                            WeakEntity::<Workspace>::new_invalid(),
                            panel.downgrade(),
                            panel.entity_id(),
                            registry_cx,
                        )?;

                        Ok::<Entity<RemotePluginPanel>, anyhow::Error>(panel)
                    })
                    .expect("attach test panel")
            })
        }

        fn new_test_workspace(
            project: Entity<Project>,
            window: &mut Window,
            cx: &mut Context<Workspace>,
        ) -> Workspace {
            let client = project.read(cx).client();
            let user_store = project.read(cx).user_store();
            let workspace_store = cx.new(|cx| workspace::WorkspaceStore::new(client.clone(), cx));
            let session = cx.new(|cx| AppSession::new(Session::test(), cx));
            window.activate_window();
            let app_state = Arc::new(workspace::AppState {
                languages: project.read(cx).languages().clone(),
                workspace_store,
                client,
                user_store,
                fs: project.read(cx).fs().clone(),
                build_window_options: |_, _| Default::default(),
                node_runtime: NodeRuntime::unavailable(),
                session,
            });

            workspace::Workspace::new(None, project, app_state, window, cx)
        }

        #[test]
        fn validate_ui_tree_limits_rejects_excessive_node_count() {
            let mut root = UiNode::new(UiNodeKind::Div);
            root.children = (0..=MAX_REMOTE_UI_NODE_COUNT)
                .map(|_| UiNode::new(UiNodeKind::Div))
                .collect();

            let error = validate_ui_tree_limits(&root).expect_err("tree should exceed limits");
            assert!(
                error
                    .to_string()
                    .contains("plugin render tree exceeded the host node limit")
            );
        }

        #[test]
        fn remote_progress_bar_id_uses_node_path_when_no_element_id_is_provided() {
            let node = UiNode::new(UiNodeKind::ProgressBar);

            let left_id = remote_progress_bar_id(&node, &PanelInstanceId::new("panel-a"), &[0, 0]);
            let right_id = remote_progress_bar_id(&node, &PanelInstanceId::new("panel-a"), &[0, 1]);

            assert_ne!(left_id, right_id);
            assert!(left_id.contains("panel-a"));
            assert!(right_id.ends_with("0-1"));
        }

        #[test]
        fn remote_node_element_id_prefers_protocol_field_over_legacy_props() {
            let mut node = UiNode::new(UiNodeKind::Div);
            node.element_id = Some("protocol-id".to_string());
            node.props.insert(
                "element_id".to_string(),
                StyleValue::Text("legacy-prop-id".to_string()),
            );

            assert_eq!(remote_node_element_id(&node), Some("protocol-id"));
        }

        #[test]
        fn remote_host_element_id_uses_protocol_identity_with_panel_prefix() {
            let mut node = UiNode::new(UiNodeKind::Div);
            node.element_id = Some("zoom-target".to_string());

            let host_id = remote_host_element_id(
                "plugin-panel-div",
                &PanelInstanceId::new("panel-a"),
                &node,
                "fallback".to_string(),
            );

            assert_eq!(host_id, "plugin-panel-div-panel-a-zoom-target");
        }

        #[test]
        fn node_has_div_interactivity_props_detects_serialized_canvas_flags() {
            let mut node = UiNode::new(UiNodeKind::Div);
            assert!(!node_has_div_interactivity_props(&node));

            node.props.insert(
                INTERACTIVE_PROP_TAB_STOP.to_string(),
                StyleValue::Bool(true),
            );
            assert!(node_has_div_interactivity_props(&node));
        }

        #[test]
        fn window_control_area_from_prop_parses_protocol_values() {
            assert_eq!(
                window_control_area_from_prop("drag"),
                Some(gpui::WindowControlArea::Drag)
            );
            assert_eq!(
                window_control_area_from_prop("close"),
                Some(gpui::WindowControlArea::Close)
            );
            assert_eq!(
                window_control_area_from_prop("max"),
                Some(gpui::WindowControlArea::Max)
            );
            assert_eq!(
                window_control_area_from_prop("min"),
                Some(gpui::WindowControlArea::Min)
            );
            assert_eq!(window_control_area_from_prop("unknown"), None);
        }

        #[test]
        fn remote_progress_bar_id_uses_protocol_element_id_when_available() {
            let mut node = UiNode::new(UiNodeKind::ProgressBar);
            node.element_id = Some("fixture-progress".to_string());

            let progress_id =
                remote_progress_bar_id(&node, &PanelInstanceId::new("panel-a"), &[0, 0]);

            assert_eq!(progress_id, "fixture-progress");
        }

        #[gpui::test]
        async fn read_bounded_line_rejects_oversized_lines(_cx: &mut TestAppContext) {
            let mut reader =
                futures::io::BufReader::new(futures::io::Cursor::new(b"abcdef\n".to_vec()));

            let line = read_bounded_line(&mut reader, 16)
                .await
                .expect("bounded line should read successfully")
                .expect("line should be present");
            assert_eq!(line, "abcdef");

            let mut oversized_reader =
                futures::io::BufReader::new(futures::io::Cursor::new(b"abcdef\n".to_vec()));

            let error = read_bounded_line(&mut oversized_reader, 4)
                .await
                .expect_err("line should exceed the limit");
            assert!(error.to_string().contains("protocol line exceeded 4 bytes"));
        }

        #[gpui::test]
        async fn sync_active_theme_broadcasts_theme_changed_to_running_processes(
            cx: &mut TestAppContext,
        ) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };

            let registry = new_test_registry(layout, cx);
            let (process_receiver, _terminate_receiver) =
                seed_process(&registry, "theme-plugin", true, cx);

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.sync_active_theme(registry_cx)
                })
            })
            .expect("sync active theme");

            let message = process_receiver
                .recv()
                .await
                .expect("theme message should be sent");
            let decoded: HostToPlugin =
                serde_json::from_str(&message).expect("theme message should decode");

            match decoded {
                HostToPlugin::ThemeChanged { theme } => {
                    cx.update(|cx| {
                        assert_eq!(theme.name, cx.theme().name.to_string());
                        assert_eq!(theme.appearance, cx.theme().appearance);
                    });
                }
                other => panic!("expected theme changed message, got {other:?}"),
            }
        }

        #[gpui::test]
        async fn sync_active_theme_keeps_open_panel_and_titlebar_widget_bindings(
            cx: &mut TestAppContext,
        ) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("theme-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "theme-plugin"
name = "Theme Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"

[[titlebar_widgets]]
id = "usage-widget"
title = "Usage Widget"
side = "right"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (process_receiver, _terminate_receiver) =
                seed_process(&registry, "theme-plugin", true, cx);

            let panel = attach_test_panel(&registry, "theme-plugin", "panel-a", "Panel A", cx);
            let panel_instance_id = cx.update(|cx| panel.read(cx).panel_instance_id.clone());
            let panel_entity_id = panel.entity_id();

            let workspace = WeakEntity::<Workspace>::new_invalid();
            let widget_instance_id = titlebar_widget_instance_id(
                &PluginId::new("theme-plugin"),
                "usage-widget",
                workspace.entity_id(),
            );
            let widget_entity_id = cx.update(|cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        let widgets = registry.titlebar_widget_views(
                            TitlebarStripSide::Right,
                            workspace.clone(),
                            registry_cx,
                        )?;
                        assert_eq!(widgets.len(), 1);
                        Ok::<gpui::EntityId, anyhow::Error>(
                            registry
                                .titlebar_widgets
                                .get(&widget_instance_id)
                                .expect("titlebar binding should exist")
                                .widget
                                .entity_id(),
                        )
                    })
                    .expect("create titlebar widget binding")
            });

            let mut opened_instances = BTreeSet::default();
            for _ in 0..2 {
                let message = process_receiver
                    .recv()
                    .await
                    .expect("open panel message should be sent");
                let decoded: HostToPlugin =
                    serde_json::from_str(&message).expect("open panel message should decode");
                match decoded {
                    HostToPlugin::OpenPanel {
                        panel_instance_id, ..
                    } => {
                        opened_instances.insert(panel_instance_id);
                    }
                    other => panic!("expected open panel message, got {other:?}"),
                }
            }
            assert_eq!(
                opened_instances,
                BTreeSet::from([panel_instance_id.clone(), widget_instance_id.clone()])
            );

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.sync_active_theme(registry_cx)
                })
            })
            .expect("sync active theme");

            let message = process_receiver
                .recv()
                .await
                .expect("theme message should be sent");
            let decoded: HostToPlugin =
                serde_json::from_str(&message).expect("theme message should decode");
            assert!(matches!(decoded, HostToPlugin::ThemeChanged { .. }));

            cx.update(|cx| {
                let registry = registry.read(cx);
                let panel_binding = registry
                    .panels
                    .get(&panel_instance_id)
                    .expect("panel binding should persist after theme sync");
                assert_eq!(panel_binding.panel_entity_id, panel_entity_id);

                let widget_binding = registry
                    .titlebar_widgets
                    .get(&widget_instance_id)
                    .expect("titlebar binding should persist after theme sync");
                assert_eq!(widget_binding.widget.entity_id(), widget_entity_id);
            });
        }

        #[gpui::test]
        async fn sync_workspace_panels_registers_on_startup_panels(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "startup-a"
title = "Startup A"
dock = "right"
activation = "on_startup"

[[panels]]
id = "startup-b"
title = "Startup B"
dock = "right"
activation = "on_startup"

[[panels]]
id = "on-demand"
title = "On Demand"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            workspace.update_in(cx, |workspace, window, workspace_cx| {
                sync_workspace_panels_for_workspace(&registry, workspace, window, workspace_cx)
                    .expect("sync plugin panels");

                let right_dock = workspace.right_dock().read(workspace_cx);
                assert_eq!(right_dock.panels_len(), 2);
                assert!(!right_dock.is_open());

                let panel_ids = registry
                    .read(workspace_cx)
                    .panels
                    .values()
                    .filter(|binding| binding.workspace == workspace.weak_handle())
                    .map(|binding| binding.panel_id.clone())
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    panel_ids,
                    BTreeSet::from([String::from("startup-a"), String::from("startup-b"),])
                );
            });
        }

        #[gpui::test]
        async fn sync_workspace_panels_rebinds_restored_remote_panels_without_duplicates(
            cx: &mut TestAppContext,
        ) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "startup-a"
title = "Startup A"
dock = "right"
activation = "on_startup"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            workspace
                .update_in(cx, |workspace, window, workspace_cx| {
                    let (descriptor, event_sender) = registry.update(workspace_cx, |registry, _| {
                    Ok::<(PanelDescriptor, channel::Sender<PluginHostEvent>), anyhow::Error>((
                        registry.panel_descriptor("test-plugin", "startup-a")?,
                        registry.event_sender.clone(),
                    ))
                })?;
                    let restored_panel = workspace_cx.new(|panel_cx| {
                        RemotePluginPanel::new(
                            "test-plugin",
                            descriptor,
                            PanelInstanceId::new("restored-panel".to_string()),
                            10_000,
                            registry.downgrade(),
                            event_sender,
                            panel_cx,
                        )
                    });

                    workspace.add_panel(restored_panel.clone(), window, workspace_cx);
                    assert_eq!(workspace.right_dock().read(workspace_cx).panels_len(), 1);

                    sync_workspace_panels_for_workspace(&registry, workspace, window, workspace_cx)
                        .expect("sync plugin panels");

                    let right_dock = workspace.right_dock().read(workspace_cx);
                    assert_eq!(right_dock.panels_len(), 1);

                    let bindings = registry
                        .read(workspace_cx)
                        .panels
                        .values()
                        .filter(|binding| binding.workspace == workspace.weak_handle())
                        .cloned()
                        .collect::<Vec<_>>();
                    assert_eq!(bindings.len(), 1);
                    assert_eq!(bindings[0].panel_entity_id, restored_panel.entity_id());

                    Ok::<(), anyhow::Error>(())
                })
                .expect("rebind restored panel");
        }

        #[gpui::test]
        async fn sync_workspace_panels_removes_duplicate_remote_panels(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "startup-a"
title = "Startup A"
dock = "right"
activation = "on_startup"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            workspace
                .update_in(cx, |workspace, window, workspace_cx| {
                    let (descriptor, event_sender) =
                        registry.update(workspace_cx, |registry, _| {
                            Ok::<
                                (PanelDescriptor, channel::Sender<PluginHostEvent>),
                                anyhow::Error,
                            >((
                                registry.panel_descriptor("test-plugin", "startup-a")?,
                                registry.event_sender.clone(),
                            ))
                        })?;

                    let panel_a = workspace_cx.new(|panel_cx| {
                        RemotePluginPanel::new(
                            "test-plugin",
                            descriptor.clone(),
                            PanelInstanceId::new("restored-panel-a".to_string()),
                            10_000,
                            registry.downgrade(),
                            event_sender.clone(),
                            panel_cx,
                        )
                    });
                    let panel_b = workspace_cx.new(|panel_cx| {
                        RemotePluginPanel::new(
                            "test-plugin",
                            descriptor,
                            PanelInstanceId::new("restored-panel-b".to_string()),
                            10_001,
                            registry.downgrade(),
                            event_sender,
                            panel_cx,
                        )
                    });

                    workspace.add_panel(panel_a, window, workspace_cx);
                    workspace.add_panel(panel_b, window, workspace_cx);
                    assert_eq!(workspace.right_dock().read(workspace_cx).panels_len(), 2);

                    sync_workspace_panels_for_workspace(
                        &registry,
                        workspace,
                        window,
                        workspace_cx,
                    )?;

                    let right_dock = workspace.right_dock().read(workspace_cx);
                    assert_eq!(right_dock.panels_len(), 1);

                    let bindings = registry
                        .read(workspace_cx)
                        .panels
                        .values()
                        .filter(|binding| binding.workspace == workspace.weak_handle())
                        .cloned()
                        .collect::<Vec<_>>();
                    assert_eq!(bindings.len(), 1);

                    Ok::<(), anyhow::Error>(())
                })
                .expect("dedupe remote panels");
        }

        #[test]
        fn cargo_backed_plugins_get_longer_registration_timeout() {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let plugin_root = temp_dir.path().join("cargo-plugin");
            fs::create_dir_all(&plugin_root).expect("create plugin root");
            fs::write(
                plugin_root.join("Cargo.toml"),
                r#"
[package]
name = "cargo-plugin"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "fake-plugin"
path = "src/main.rs"
"#,
            )
            .expect("write Cargo.toml");

            let plugin = plugin::InstalledPlugin {
                manifest: plugin::PluginManifest {
                    id: PluginId::new("cargo-plugin"),
                    name: "Cargo Plugin".into(),
                    version: "0.1.0".into(),
                    schema_version: 1,
                    description: None,
                    authors: Vec::new(),
                    repository: None,
                    homepage: None,
                    entrypoint: PathBuf::from("cargo-plugin"),
                    panels: Vec::new(),
                    titlebar_widgets: Vec::new(),
                },
                state: PluginInstallState::Installed,
                installation: plugin::PluginInstallation {
                    root: plugin_root.clone(),
                    source: plugin::PluginInstallSource::Directory(plugin_root),
                },
                error_message: None,
            };

            assert_eq!(
                registration_timeout_for_plugin(&plugin, false),
                Duration::from_secs(180)
            );
            assert_eq!(
                registration_timeout_for_plugin(&plugin, true),
                Duration::from_millis(75)
            );
        }

        #[gpui::test]
        async fn cargo_plugin_panel_shows_startup_state_and_can_cancel(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("cargo-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "cargo-plugin"
name = "Cargo Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");
            fs::write(
                plugin_root.join("Cargo.toml"),
                r#"
[package]
name = "cargo-plugin"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "fake-plugin"
path = "src/main.rs"
"#,
            )
            .expect("write Cargo.toml");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, terminate_receiver) =
                seed_process(&registry, "cargo-plugin", false, cx);

            let panel = attach_test_panel(&registry, "cargo-plugin", "panel-a", "Panel A", cx);

            cx.update(|cx| {
                assert_eq!(
                    panel.read(cx).startup_state,
                    Some(RemotePluginStartupState::Building)
                );
            });

            cx.update(|cx| {
                panel.update(cx, |panel, panel_cx| {
                    panel.cancel_startup(panel_cx);
                });
            });

            assert!(matches!(
                terminate_receiver.try_recv().ok(),
                Some(ProcessTermination::StartupCancelled)
            ));

            cx.update(|cx| {
                assert_eq!(
                    panel.read(cx).startup_state,
                    Some(RemotePluginStartupState::Cancelling)
                );
            });

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::Exited {
                            plugin_id: PluginId::new("cargo-plugin"),
                            process_instance_id: 1,
                            exit_status: None,
                            error_message: None,
                            suppress_ui_error: true,
                        },
                        registry_cx,
                    );
                });

                let panel = panel.read(cx);
                assert_eq!(
                    panel.startup_state,
                    Some(RemotePluginStartupState::Cancelling)
                );
                assert!(panel.error_message.is_none());
                assert!(
                    !registry
                        .read(cx)
                        .panels
                        .contains_key(&PanelInstanceId::new("test-panel-cargo-plugin-panel-a"))
                );
            });
        }

        #[gpui::test]
        async fn register_message_advances_cargo_plugin_startup_state(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("cargo-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "cargo-plugin"
name = "Cargo Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");
            fs::write(
                plugin_root.join("Cargo.toml"),
                r#"
[package]
name = "cargo-plugin"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "fake-plugin"
path = "src/main.rs"
"#,
            )
            .expect("write Cargo.toml");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "cargo-plugin", false, cx);

            let panel = attach_test_panel(&registry, "cargo-plugin", "panel-a", "Panel A", cx);

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::Message {
                            plugin_id: PluginId::new("cargo-plugin"),
                            process_instance_id: 1,
                            message: PluginToHost::Register {
                                plugin: plugin_protocol::PluginMetadata {
                                    id: PluginId::new("cargo-plugin"),
                                    name: "Cargo Plugin".to_string(),
                                    version: "0.1.0".to_string(),
                                    description: None,
                                    panels: Vec::new(),
                                    titlebar_widgets: Vec::new(),
                                },
                            },
                        },
                        registry_cx,
                    );
                });

                assert_eq!(
                    panel.read(cx).startup_state,
                    Some(RemotePluginStartupState::Starting)
                );
                assert!(
                    registry
                        .read(cx)
                        .processes
                        .get(&PluginId::new("cargo-plugin"))
                        .expect("seeded process should exist")
                        .registered
                );
            });
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn linux_seccomp_program_blocks_mount_and_namespace_escape_syscalls() {
            assert!(LINUX_BLOCKED_SYSCALLS.contains(&libc::SYS_mount));
            assert!(LINUX_BLOCKED_SYSCALLS.contains(&libc::SYS_umount2));
            assert!(LINUX_BLOCKED_SYSCALLS.contains(&libc::SYS_unshare));
            assert!(LINUX_BLOCKED_SYSCALLS.contains(&libc::SYS_setns));
            assert!(LINUX_BLOCKED_SYSCALLS.contains(&libc::SYS_bpf));

            let program = linux_seccomp_program();
            assert_eq!(program.len(), LINUX_SECCOMP_FILTER_LEN);
            assert_eq!(program[0].k, LINUX_SECCOMP_ARCH_OFFSET);
            assert_eq!(program[3].k, LINUX_SECCOMP_NR_OFFSET);
            assert_eq!(
                program.last().expect("seccomp program terminator").k,
                LINUX_SECCOMP_RET_ALLOW
            );
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn linux_landlock_write_access_includes_mutation_rights() {
            let abi_v1 = linux_landlock_allowed_access(1, LinuxLandlockAccess::ReadWrite);
            assert_ne!(abi_v1 & LINUX_LANDLOCK_ACCESS_FS_WRITE_FILE, 0);
            assert_ne!(abi_v1 & LINUX_LANDLOCK_ACCESS_FS_MAKE_DIR, 0);
            assert_eq!(abi_v1 & LINUX_LANDLOCK_ACCESS_FS_REFER, 0);
            assert_eq!(abi_v1 & LINUX_LANDLOCK_ACCESS_FS_TRUNCATE, 0);

            let abi_v3 = linux_landlock_allowed_access(3, LinuxLandlockAccess::ReadWrite);
            assert_ne!(abi_v3 & LINUX_LANDLOCK_ACCESS_FS_REFER, 0);
            assert_ne!(abi_v3 & LINUX_LANDLOCK_ACCESS_FS_TRUNCATE, 0);
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn build_command_wraps_plugins_in_sandbox_exec_on_macos() {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let plugin_root = temp_dir.path().join("sandboxed-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "sandboxed-plugin"
name = "Sandboxed Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"
"#,
            )
            .expect("write fake plugin");

            let plugin = plugin::InstalledPlugin {
                manifest: plugin::PluginManifest {
                    id: PluginId::new("sandboxed-plugin"),
                    name: "Sandboxed Plugin".into(),
                    version: "0.1.0".into(),
                    schema_version: 1,
                    description: None,
                    authors: Vec::new(),
                    repository: None,
                    homepage: None,
                    entrypoint: PathBuf::from("fake-plugin"),
                    panels: Vec::new(),
                    titlebar_widgets: Vec::new(),
                },
                state: PluginInstallState::Installed,
                installation: plugin::PluginInstallation {
                    root: plugin_root.clone(),
                    source: plugin::PluginInstallSource::Directory(plugin_root.clone()),
                },
                error_message: None,
            };

            let command = build_command(&plugin).expect("build sandboxed command");
            let args = command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>();

            assert_eq!(
                command.get_program().to_string_lossy(),
                "/usr/bin/sandbox-exec"
            );
            assert_eq!(args[0], "-p");
            assert!(args[1].contains("(allow file-write*"));
            assert!(!args[1].contains("(allow process-exec*)"));
            assert!(args[1].contains("/usr/bin/open"));
            assert!(args[1].contains(plugin_root.to_string_lossy().as_ref()));
            assert_eq!(args[2], plugin_root.join("fake-plugin").to_string_lossy());
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn cargo_backed_plugins_use_sandbox_exec_with_isolated_target_dir() {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let plugin_root = temp_dir.path().join("cargo-plugin");
            fs::create_dir_all(&plugin_root).expect("create plugin root");
            fs::write(
                plugin_root.join("Cargo.toml"),
                r#"
[package]
name = "cargo-plugin"
version = "0.1.0"
edition = "2021"
"#,
            )
            .expect("write Cargo.toml");

            let plugin = plugin::InstalledPlugin {
                manifest: plugin::PluginManifest {
                    id: PluginId::new("cargo-plugin"),
                    name: "Cargo Plugin".into(),
                    version: "0.1.0".into(),
                    schema_version: 1,
                    description: None,
                    authors: Vec::new(),
                    repository: None,
                    homepage: None,
                    entrypoint: PathBuf::from("cargo-plugin"),
                    panels: Vec::new(),
                    titlebar_widgets: Vec::new(),
                },
                state: PluginInstallState::Development,
                installation: plugin::PluginInstallation {
                    root: plugin_root.clone(),
                    source: plugin::PluginInstallSource::Directory(plugin_root.clone()),
                },
                error_message: None,
            };

            let command = build_command(&plugin).expect("build sandboxed cargo command");
            let args = command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let target_dir = command
                .get_envs()
                .find_map(|(key, value)| {
                    (key == "CARGO_TARGET_DIR").then_some(value.expect("target dir should be set"))
                })
                .expect("cargo target dir should be present")
                .to_string_lossy()
                .into_owned();

            assert_eq!(
                command.get_program().to_string_lossy(),
                "/usr/bin/sandbox-exec"
            );
            assert_eq!(args[0], "-p");
            assert_eq!(
                args[2],
                resolve_program_on_path(OsStr::new("cargo"))
                    .expect("cargo should resolve on PATH")
                    .to_string_lossy()
            );
            assert_eq!(args[3], "run");
            assert_eq!(target_dir, cargo_target_dir(&plugin_root).to_string_lossy());
            assert!(args[1].contains("(allow file-read-metadata"));
            assert!(args[1].contains(temp_dir.path().to_string_lossy().as_ref()));
            let home_dir = home_dir_from_environment();
            if !home_dir.as_os_str().is_empty() {
                assert!(args[1].contains(home_dir.to_string_lossy().as_ref()));
            }
            let canonical_temp_dir = env::temp_dir()
                .canonicalize()
                .expect("temp dir should canonicalize");
            assert!(args[1].contains(canonical_temp_dir.to_string_lossy().as_ref()));
        }

        #[cfg(target_os = "macos")]
        #[test]
        #[allow(clippy::disallowed_methods)]
        fn sandboxed_plugin_process_cannot_write_outside_plugin_root() {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let plugin_root = temp_dir.path().join("sandboxed-plugin");
            let outside_path = env::current_dir()
                .expect("current dir")
                .join(".plugin-sandbox-write-denied");
            fs::remove_file(&outside_path).ok();

            write_plugin(
                &plugin_root,
                r#"
id = "sandboxed-plugin"
name = "Sandboxed Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"
"#,
                &format!(
                    "#!/bin/sh\nset -e\nprintf 'sandbox test' > '{}'\n",
                    outside_path.display()
                ),
            )
            .expect("write sandbox test plugin");

            let plugin = plugin::InstalledPlugin {
                manifest: plugin::PluginManifest {
                    id: PluginId::new("sandboxed-plugin"),
                    name: "Sandboxed Plugin".into(),
                    version: "0.1.0".into(),
                    schema_version: 1,
                    description: None,
                    authors: Vec::new(),
                    repository: None,
                    homepage: None,
                    entrypoint: PathBuf::from("fake-plugin"),
                    panels: Vec::new(),
                    titlebar_widgets: Vec::new(),
                },
                state: PluginInstallState::Installed,
                installation: plugin::PluginInstallation {
                    root: plugin_root.clone(),
                    source: plugin::PluginInstallSource::Directory(plugin_root),
                },
                error_message: None,
            };

            let sandboxed_command = build_command(&plugin).expect("build sandboxed command");
            let mut command = std::process::Command::new(sandboxed_command.get_program());
            command.args(sandboxed_command.get_args());
            if let Some(current_dir) = sandboxed_command.get_current_dir() {
                command.current_dir(current_dir);
            }
            command.envs(
                sandboxed_command
                    .get_envs()
                    .filter_map(|(key, value)| value.map(|value| (key, value))),
            );
            let output = command.output().expect("wait for sandboxed plugin");
            fs::remove_file(&outside_path).ok();

            assert!(!output.status.success());
            assert!(!outside_path.exists());
        }

        #[gpui::test]
        async fn titlebar_widget_binding_survives_initial_render_race(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[titlebar_widgets]]
id = "usage-widget"
title = "Usage Widget"
side = "right"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let panel_instance_id = cx.update(|cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        let widgets = registry.titlebar_widget_views(
                            TitlebarStripSide::Right,
                            WeakEntity::<Workspace>::new_invalid(),
                            registry_cx,
                        )?;
                        assert_eq!(widgets.len(), 1);
                        let panel_instance_id = registry
                            .titlebar_widgets
                            .keys()
                            .next()
                            .cloned()
                            .expect("titlebar widget binding should exist");
                        drop(widgets);
                        Ok::<PanelInstanceId, anyhow::Error>(panel_instance_id)
                    })
                    .expect("create titlebar widget binding")
            });

            let root = UiNode::new(UiNodeKind::Div)
                .with_style(|mut style| {
                    style.display = Some(gpui::Display::Flex);
                    style
                })
                .with_child(UiNode::new(UiNodeKind::Icon).with_text("ai_open_ai"))
                .with_child(UiNode::text("Codex 42% left"));

            cx.update(|cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::Render {
                                panel_id: String::from("usage-widget"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(1),
                                root: root.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply titlebar widget render");

                let widget_tree = registry
                    .read(cx)
                    .titlebar_widgets
                    .get(&panel_instance_id)
                    .and_then(|binding| binding.widget.read(cx).tree.clone());
                assert_eq!(widget_tree, Some(root));
            });
        }

        #[gpui::test]
        async fn stale_titlebar_widget_render_before_binding_is_ignored(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[titlebar_widgets]]
id = "usage-widget"
title = "Usage Widget"
side = "right"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let workspace = WeakEntity::<Workspace>::new_invalid();
            let panel_instance_id = titlebar_widget_instance_id(
                &PluginId::new("test-plugin"),
                "usage-widget",
                workspace.entity_id(),
            );
            let stale_root =
                UiNode::new(UiNodeKind::Div).with_child(UiNode::text("stale render payload"));

            cx.update(|cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::Render {
                                panel_id: String::from("usage-widget"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(1),
                                root: stale_root.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply stale titlebar render");
            });

            assert!(
                process_receiver.try_recv().is_err(),
                "stale pre-bind render should not close titlebar widget session"
            );

            let rendered_root =
                UiNode::new(UiNodeKind::Div).with_child(UiNode::text("render after binding"));

            cx.update(|cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        let widgets = registry.titlebar_widget_views(
                            TitlebarStripSide::Right,
                            workspace.clone(),
                            registry_cx,
                        )?;
                        assert_eq!(widgets.len(), 1);
                        Ok::<(), anyhow::Error>(())
                    })
                    .expect("create titlebar widget binding");

                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::Render {
                                panel_id: String::from("usage-widget"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(2),
                                root: rendered_root.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply titlebar widget render");

                let widget_tree = registry
                    .read(cx)
                    .titlebar_widgets
                    .get(&panel_instance_id)
                    .and_then(|binding| binding.widget.read(cx).tree.clone());
                assert_eq!(widget_tree, Some(rendered_root));
            });
        }

        #[gpui::test]
        async fn render_delta_updates_existing_panel_tree(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            let _panel = workspace
                .update_in(cx, |workspace, window, workspace_cx| {
                    open_panel_in_workspace(
                        "test-plugin",
                        "panel-a",
                        workspace,
                        window,
                        workspace_cx,
                    )
                })
                .expect("open panel");

            let panel_instance_id = cx.update(|_window, cx| {
                registry
                    .read(cx)
                    .panels
                    .keys()
                    .next()
                    .cloned()
                    .expect("panel binding should exist")
            });

            let initial_root = UiNode::new(UiNodeKind::Div)
                .with_child(UiNode::new(UiNodeKind::Label).with_text("before"));
            let updated_root = UiNode::new(UiNodeKind::Div)
                .with_child(UiNode::new(UiNodeKind::Label).with_text("after"));
            let patches = plugin_protocol::diff_ui_trees(&initial_root, &updated_root);

            cx.update(|_window, cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::Render {
                                panel_id: String::from("panel-a"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(1),
                                root: initial_root.clone(),
                            },
                            registry_cx,
                        )?;
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::RenderDelta {
                                panel_id: String::from("panel-a"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(2),
                                patches: patches.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply panel render delta");

                let tree = registry
                    .read(cx)
                    .panels
                    .get(&panel_instance_id)
                    .and_then(|binding| binding.panel.upgrade())
                    .and_then(|panel| panel.read(cx).tree.clone());
                assert_eq!(tree, Some(updated_root));
            });
        }

        #[gpui::test]
        async fn close_panel_clears_tree_for_closed_state(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            let panel = workspace
                .update_in(cx, |workspace, window, workspace_cx| {
                    open_panel_in_workspace(
                        "test-plugin",
                        "panel-a",
                        workspace,
                        window,
                        workspace_cx,
                    )
                })
                .expect("open panel");

            let panel_instance_id = cx.update(|_window, cx| {
                registry
                    .read(cx)
                    .panels
                    .keys()
                    .next()
                    .cloned()
                    .expect("panel binding should exist")
            });

            let initial_root = UiNode::new(UiNodeKind::Div).with_child(
                UiNode::new(UiNodeKind::Button)
                    .with_text("before close")
                    .with_event(UiEventKind::Click, EventHandlerId::new("h_close")),
            );

            cx.update(|_window, cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::Render {
                                panel_id: String::from("panel-a"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(1),
                                root: initial_root.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply panel render");

                assert!(
                    panel.read(cx).tree.is_some(),
                    "panel should have rendered tree"
                );
            });

            cx.update(|_window, cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::ClosePanel {
                                panel_instance_id: panel_instance_id.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply close panel message");

                assert!(
                    panel.read(cx).error_message.is_some(),
                    "panel should show closed-state error message"
                );
                assert!(
                    panel.read(cx).tree.is_none(),
                    "closed state should not keep stale interactive tree"
                );
                assert!(
                    !registry.read(cx).panels.contains_key(&panel_instance_id),
                    "closed panel binding should be detached"
                );
            });
        }

        #[gpui::test]
        async fn report_error_clears_tree_for_error_state(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            let panel = workspace
                .update_in(cx, |workspace, window, workspace_cx| {
                    open_panel_in_workspace(
                        "test-plugin",
                        "panel-a",
                        workspace,
                        window,
                        workspace_cx,
                    )
                })
                .expect("open panel");

            let panel_instance_id = cx.update(|_window, cx| {
                registry
                    .read(cx)
                    .panels
                    .keys()
                    .next()
                    .cloned()
                    .expect("panel binding should exist")
            });

            let initial_root = UiNode::new(UiNodeKind::Div).with_child(
                UiNode::new(UiNodeKind::Button)
                    .with_text("before error")
                    .with_event(UiEventKind::Click, EventHandlerId::new("h_error")),
            );

            cx.update(|_window, cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::Render {
                                panel_id: String::from("panel-a"),
                                panel_instance_id: panel_instance_id.clone(),
                                generation: Some(1),
                                root: initial_root.clone(),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply panel render");
                assert!(
                    panel.read(cx).tree.is_some(),
                    "panel should have rendered tree"
                );
            });

            cx.update(|_window, cx| {
                registry
                    .update(cx, |registry, registry_cx| {
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::ReportError {
                                panel_instance_id: Some(panel_instance_id.clone()),
                                message: String::from("render pipeline failed"),
                            },
                            registry_cx,
                        )
                    })
                    .expect("apply panel error");

                assert!(
                    panel.read(cx).error_message.is_some(),
                    "panel should show error-state message"
                );
                assert!(
                    panel.read(cx).tree.is_none(),
                    "error state should not keep stale interactive tree"
                );
                assert!(
                    registry.read(cx).panels.contains_key(&panel_instance_id),
                    "error state should keep panel binding attached"
                );
            });
        }

        #[gpui::test]
        async fn detaching_last_view_terminates_the_plugin_process(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("registered-plugin");
            write_registering_plugin(
                &plugin_root,
                r#"
id = "registered-plugin"
name = "Registered Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
                "registered-plugin",
                "Registered Plugin",
            )
            .expect("write registering plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, terminate_receiver) =
                seed_process(&registry, "registered-plugin", true, cx);

            let fs = FakeFs::new(cx.executor());
            let project = Project::test(fs, [], cx).await;
            let (workspace, cx) =
                cx.add_window_view(|window, cx| new_test_workspace(project, window, cx));

            let panel = workspace
                .update_in(cx, |workspace, window, workspace_cx| {
                    open_panel_in_workspace(
                        "registered-plugin",
                        "panel-a",
                        workspace,
                        window,
                        workspace_cx,
                    )
                })
                .expect("open panel");

            let panel_instance_id = cx.update(|_window, cx| {
                registry
                    .read(cx)
                    .panels
                    .keys()
                    .next()
                    .cloned()
                    .expect("panel binding should exist")
            });

            workspace.update_in(cx, |workspace, window, workspace_cx| {
                workspace.remove_panel(&panel, window, workspace_cx);
            });
            drop(panel);

            let plugin_id = PluginId::new("registered-plugin");
            cx.update(|_window, cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::ViewDetached {
                            panel_instance_id: panel_instance_id.clone(),
                        },
                        registry_cx,
                    );
                });
            });
            assert!(matches!(
                terminate_receiver.try_recv().ok(),
                Some(ProcessTermination::Idle)
            ));

            cx.update(|_window, cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::Exited {
                            plugin_id: plugin_id.clone(),
                            process_instance_id: 1,
                            exit_status: None,
                            error_message: None,
                            suppress_ui_error: false,
                        },
                        registry_cx,
                    );
                });
                let registry = registry.read(cx);
                assert!(registry.panels.is_empty());
                assert!(!registry.processes.contains_key(&plugin_id));
            });
        }

        #[gpui::test]
        async fn mismatched_plugin_registration_requests_protocol_termination(
            cx: &mut TestAppContext,
        ) {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let registry = new_test_registry(layout, cx);
            let (_process_receiver, terminate_receiver) =
                seed_process(&registry, "expected-plugin", false, cx);

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::Message {
                            plugin_id: PluginId::new("expected-plugin"),
                            process_instance_id: 1,
                            message: PluginToHost::Register {
                                plugin: plugin_protocol::PluginMetadata {
                                    id: PluginId::new("actual-plugin"),
                                    name: "Actual Plugin".to_string(),
                                    version: "0.1.0".to_string(),
                                    description: None,
                                    panels: Vec::new(),
                                    titlebar_widgets: Vec::new(),
                                },
                            },
                        },
                        registry_cx,
                    );
                });

                assert!(
                    !registry
                        .read(cx)
                        .processes
                        .get(&PluginId::new("expected-plugin"))
                        .expect("seeded process should exist")
                        .registered
                );
            });

            match terminate_receiver.try_recv() {
                Ok(ProcessTermination::ProtocolViolation { message }) => {
                    assert!(message.contains("expected-plugin"));
                    assert!(message.contains("actual-plugin"));
                }
                Ok(other) => panic!("unexpected termination reason: {other:?}"),
                Err(error) => panic!("expected protocol violation termination, got {error}"),
            }
        }

        #[gpui::test]
        async fn unregistered_plugin_messages_request_protocol_termination(
            cx: &mut TestAppContext,
        ) {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let registry = new_test_registry(layout, cx);
            let (_process_receiver, terminate_receiver) =
                seed_process(&registry, "test-plugin", false, cx);

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::Message {
                            plugin_id: PluginId::new("test-plugin"),
                            process_instance_id: 1,
                            message: PluginToHost::Render {
                                panel_id: "panel-a".to_string(),
                                panel_instance_id: PanelInstanceId::new("panel-instance"),
                                generation: Some(1),
                                root: UiNode::new(UiNodeKind::Div),
                            },
                        },
                        registry_cx,
                    );
                });

                assert!(
                    !registry
                        .read(cx)
                        .processes
                        .get(&PluginId::new("test-plugin"))
                        .expect("seeded process should exist")
                        .registered
                );
            });

            match terminate_receiver.try_recv() {
                Ok(ProcessTermination::ProtocolViolation { message }) => {
                    assert!(message.contains("test-plugin"));
                    assert!(message.contains("render message before registering"));
                }
                Ok(other) => panic!("unexpected termination reason: {other:?}"),
                Err(error) => panic!("expected protocol violation termination, got {error}"),
            }
        }

        #[gpui::test]
        async fn remove_plugin_eagerly_cleans_up_running_process_entries(cx: &mut TestAppContext) {
            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("test-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (process_receiver, _terminate_receiver) =
                seed_process(&registry, "test-plugin", true, cx);

            cx.update(|cx| {
                let removed = registry
                    .update(cx, |registry, _| registry.remove_plugin("test-plugin"))
                    .expect("remove plugin");
                assert!(removed);
                assert!(
                    !registry
                        .read(cx)
                        .processes
                        .contains_key(&PluginId::new("test-plugin"))
                );
            });

            let shutdown_message = process_receiver
                .try_recv()
                .expect("shutdown should be queued before removal");
            let decoded: HostToPlugin =
                serde_json::from_str(&shutdown_message).expect("shutdown message should decode");
            assert_eq!(decoded, HostToPlugin::Shutdown);
            assert!(!plugin_root.exists());
        }

        #[gpui::test]
        async fn plugin_process_is_killed_if_it_never_registers(cx: &mut TestAppContext) {
            init_test_app(cx);

            let temp_dir = TempDir::new().expect("temp plugin dir");
            let layout = PluginStoreLayout {
                installed_root: temp_dir.path().join("installed"),
                development_root: temp_dir.path().join("development"),
            };
            let plugin_root = layout.installed_root.join("timed-out-plugin");
            write_fake_plugin(
                &plugin_root,
                r#"
id = "timed-out-plugin"
name = "Timed Out Plugin"
version = "0.1.0"
schema_version = 1
entry = "fake-plugin"

[[panels]]
id = "panel-a"
title = "Panel A"
dock = "right"
activation = "on_demand"
"#,
            )
            .expect("write fake plugin");

            let registry = new_test_registry(layout, cx);
            let (_process_receiver, terminate_receiver) =
                seed_process(&registry, "timed-out-plugin", false, cx);

            let plugin_id = PluginId::new("timed-out-plugin");
            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::RegistrationTimedOut {
                            plugin_id: plugin_id.clone(),
                            process_instance_id: 1,
                        },
                        registry_cx,
                    );
                });
            });
            assert!(matches!(
                terminate_receiver.try_recv().ok(),
                Some(ProcessTermination::RegistrationTimedOut)
            ));

            cx.update(|cx| {
                registry.update(cx, |registry, registry_cx| {
                    registry.handle_event(
                        PluginHostEvent::Exited {
                            plugin_id: plugin_id.clone(),
                            process_instance_id: 1,
                            exit_status: None,
                            error_message: None,
                            suppress_ui_error: false,
                        },
                        registry_cx,
                    );
                });
                assert!(!registry.read(cx).processes.contains_key(&plugin_id));
            });
        }

        #[gpui::test]
        async fn oversized_stdout_line_requests_protocol_termination(_cx: &mut TestAppContext) {
            let reader = futures::io::BufReader::new(futures::io::Cursor::new(vec![
                b'a';
                MAX_PLUGIN_STDIO_LINE_BYTES
                    + 1
            ]));
            let (event_sender, event_receiver) = channel::unbounded::<PluginHostEvent>();
            let (terminate_sender, terminate_receiver) = channel::unbounded::<ProcessTermination>();

            forward_plugin_stdout(
                reader,
                event_sender,
                terminate_sender,
                PluginId::new("oversized-stdout-plugin"),
                1,
            )
            .await
            .expect("stdout forwarding should complete");

            match terminate_receiver.try_recv() {
                Ok(ProcessTermination::ProtocolViolation { message }) => {
                    assert!(message.contains("oversized-stdout-plugin"));
                    assert!(message.contains("stdout"));
                    assert!(message.contains("bytes"));
                }
                Ok(other) => panic!("unexpected termination reason: {other:?}"),
                Err(error) => panic!("expected protocol violation termination, got {error}"),
            }

            assert!(event_receiver.try_recv().is_err());
        }
    }
}

pub use host::{
    PluginHostRegistry, RemotePluginPanel, ToggleRemotePluginPanel, init, open_panel_in_workspace,
    refresh_catalog,
};
