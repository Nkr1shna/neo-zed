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
    use futures::future::{Either, select};
    use futures::io::{BufReader, BufWriter};
    use futures::{AsyncBufReadExt as _, AsyncWriteExt as _, FutureExt as _, pin_mut};
    use gpui::{
        Action, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
        InteractiveElement, ParentElement, Render, SharedString, StatefulInteractiveElement,
        Styled, Subscription, WeakEntity, Window, div,
    };
    use plugin::{InstalledPlugin, PluginStore, PluginStoreLayout};
    use plugin_protocol::{
        DockPosition as PluginDockPosition, EventHandlerId, HostThemeSnapshot, HostToPlugin,
        PanelActivation, PanelDescriptor, PanelInstanceId, PluginId, PluginToHost,
        SerializedActionEvent, SerializedClickEvent, SerializedKeyDownEvent, SerializedKeyUpEvent,
        SerializedModifiersChangedEvent, SerializedMouseDownEvent, SerializedMouseMoveEvent,
        SerializedMousePressureEvent, SerializedMouseUpEvent, SerializedPinchEvent,
        SerializedScrollWheelEvent, StyleValue, TitlebarWidgetDescriptor, TitlebarWidgetSide,
        UiEvent, UiEventKind, UiEventPhase, UiNode, UiNodeKind, apply_ui_patches,
    };
    use serde::Deserialize;
    use smol::{channel, process::Command};
    use std::{
        any::TypeId,
        collections::{BTreeMap, BTreeSet},
        env,
        ffi::OsString,
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
                    eprintln!("failed to sync plugin themes after theme change: {error:#}");
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
                eprintln!("failed to sync plugin panels for workspace startup: {error:#}");
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
            eprintln!("failed to refresh plugin catalog: {error:#}");
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
                    eprintln!("failed to refresh plugin panels in workspace window: {error:#}");
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
                    eprintln!("failed to render plugin titlebar strip: {error:#}");
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
                error_message: None,
                event_sender,
                workspace,
                registry,
                widget_entity_id: cx.entity().entity_id().as_u64(),
            }
        }

        fn update_tree(&mut self, tree: UiNode, cx: &mut Context<Self>) {
            self.tree = Some(tree);
            self.error_message = None;
            cx.notify();
        }

        fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
            self.error_message = Some(message.into().into());
            cx.notify();
        }

        fn clear_error(&mut self, cx: &mut Context<Self>) {
            self.error_message = None;
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
                handler_id,
                kind,
                payload,
            };

            let result = self
                .registry
                .update(cx, |registry, _cx| registry.dispatch_event(event));
            if let Err(error) = result {
                self.error_message = Some(error.to_string().into());
                cx.notify();
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
                self.error_message = Some(error.to_string().into());
                cx.notify();
            }
        }

        fn render_node(&self, node: &UiNode, cx: &mut Context<Self>) -> gpui::AnyElement {
            match node.kind {
                UiNodeKind::Empty => gpui::Empty.into_any_element(),
                UiNodeKind::Div => {
                    let element = apply_styles(div(), &node.styles).children(
                        node.children
                            .iter()
                            .map(|child| self.render_node(child, cx)),
                    );
                    let element = if node.events.is_empty() {
                        element.into_any_element()
                    } else {
                        apply_div_events(
                            element.id(format!(
                                "plugin-titlebar-div-{}-{}",
                                self.panel_instance_id, node.events[0].handler_id
                            )),
                            node,
                            cx,
                        )
                        .into_any_element()
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
                    if let Some(weight) =
                        style_number(&node.props, "font_weight").map(gpui::FontWeight)
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
                    let button_id = node
                        .events
                        .first()
                        .map(|event| format!("plugin-titlebar-button-{}", event.handler_id))
                        .unwrap_or_else(|| {
                            format!(
                                "plugin-titlebar-button-{}-{}",
                                self.panel_instance_id, self.widget_entity_id
                            )
                        });
                    render_button_node(node, button_id, cx)
                }
                UiNodeKind::Divider => render_divider_node(node),
                UiNodeKind::Indicator => render_indicator_node(node),
                UiNodeKind::Icon => {
                    let mut icon = Icon::new(
                        icon_name_from_descriptor(node.text.as_deref())
                            .unwrap_or(IconName::AiOpenAi),
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
                UiNodeKind::ProgressBar => render_progress_bar_node(node, cx),
                UiNodeKind::MenuItem => gpui::Empty.into_any_element(),
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
                self.render_node(&tree, cx)
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
            if let Some(process) = self.processes.get(&PluginId::new(plugin_id)) {
                send_message(&process.sender, &HostToPlugin::Shutdown)?;
            }
            let mut store = self.store();
            store.remove(plugin_id)
        }

        pub fn refresh_catalog(&mut self, cx: &mut Context<Self>) {
            if let Err(error) = self.prune_titlebar_widget_bindings(cx) {
                eprintln!("failed to prune plugin titlebar widgets: {error:#}");
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

            panel.update(cx, |panel, cx| panel.clear_error(cx))?;

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

            widget.update(cx, |widget, cx| widget.clear_error(cx));

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
                eprintln!("failed to close plugin panel view {panel_instance_id}: {error:#}");
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
                eprintln!(
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
                        eprintln!("plugin host message error: {error:#}");
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
                        eprintln!("failed to terminate timed out plugin `{plugin_id}`: {error}");
                    }
                }
                PluginHostEvent::ViewDetached { panel_instance_id } => {
                    if let Err(error) = self.detach_panel_binding(&panel_instance_id) {
                        eprintln!(
                            "failed to detach dropped plugin panel `{panel_instance_id}`: {error:#}"
                        );
                        return;
                    }

                    if let Err(error) = self.detach_titlebar_widget_binding(&panel_instance_id) {
                        eprintln!(
                            "failed to detach dropped plugin titlebar widget `{panel_instance_id}`: {error:#}"
                        );
                    }
                }
                PluginHostEvent::Exited {
                    plugin_id,
                    process_instance_id,
                    exit_status,
                    error_message,
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
                                binding
                                    .panel
                                    .update(cx, |panel, cx| {
                                        panel.set_error(message.clone(), cx);
                                    })
                                    .ok();
                            }
                            if let Some(widget) = self.titlebar_widgets.remove(&panel_instance_id) {
                                widget.widget.update(cx, |widget, cx| {
                                    widget.set_error(message.clone(), cx);
                                });
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
            match message {
                PluginToHost::Register { plugin } => {
                    if plugin.id != plugin_id {
                        eprintln!(
                            "plugin `{}` registered itself as `{}`",
                            plugin_id, plugin.id
                        );
                    }
                    if let Some(process) = self.processes.get_mut(&plugin_id) {
                        process.registered = true;
                    }
                }
                PluginToHost::Render {
                    panel_instance_id,
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
                            .update(cx, |panel, cx| panel.update_tree(root, cx))
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
                            .update(cx, |widget, cx| widget.update_tree(root, cx));
                    } else {
                        self.close_remote_view(&plugin_id, &panel_instance_id)?;
                    }
                }
                PluginToHost::RenderDelta {
                    panel_instance_id,
                    patches,
                    ..
                } => {
                    let updated_root =
                        if let Some(binding) = self.panels.get(&panel_instance_id).cloned() {
                            let mut root = if let Ok(tree) =
                                binding.panel.read_with(cx, |panel, _| panel.tree.clone())
                            {
                                tree
                            } else {
                                None
                            };
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
                            self.close_remote_view(&plugin_id, &panel_instance_id)?;
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
                                .update(cx, |panel, cx| panel.update_tree(root, cx))
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
                            .update(cx, |widget, cx| widget.update_tree(root, cx));
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
                            eprintln!("plugin view error for {}: {}", panel_instance_id, message);
                        }
                    } else {
                        eprintln!("plugin `{}` error: {}", plugin_id, message);
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
                plugin_id.clone(),
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
                eprintln!("failed to terminate idle plugin `{plugin_id}`: {error}");
            }
        }

        fn has_current_process(&self, plugin_id: &PluginId, process_instance_id: u64) -> bool {
            self.processes
                .get(plugin_id)
                .map(|process| process.instance_id == process_instance_id)
                .unwrap_or(false)
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

    #[derive(Clone, Debug)]
    enum ProcessTermination {
        Idle,
        RegistrationTimedOut,
        ProtocolViolation { message: String },
    }

    impl ProcessTermination {
        fn error_message(self, plugin_id: &PluginId) -> Option<String> {
            match self {
                Self::Idle => None,
                Self::RegistrationTimedOut => Some(format!(
                    "plugin `{plugin_id}` did not register with the host before the startup timeout"
                )),
                Self::ProtocolViolation { message } => Some(message),
            }
        }
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
            eprintln!("failed to request plugin termination after {context}: {error}");
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
                eprintln!("failed to forward plugin stdout message: {error}");
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
            eprintln!("plugin stderr: {}", line.trim());
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
        },
    }

    pub struct RemotePluginPanel {
        _plugin_id: PluginId,
        descriptor: PanelDescriptor,
        panel_instance_id: PanelInstanceId,
        activation_priority: u32,
        tree: Option<UiNode>,
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
                _plugin_id: PluginId::new(plugin_id),
                descriptor,
                panel_instance_id,
                activation_priority,
                tree: None,
                error_message: None,
                event_sender,
                focus_handle: cx.focus_handle(),
                registry,
                panel_entity_id: cx.entity_id().as_u64(),
            }
        }

        fn update_tree(&mut self, tree: UiNode, cx: &mut Context<Self>) {
            self.tree = Some(tree);
            self.error_message = None;
            cx.notify();
        }

        fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
            self.error_message = Some(message.into().into());
            cx.notify();
        }

        fn clear_error(&mut self, cx: &mut Context<Self>) {
            self.error_message = None;
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
                handler_id,
                kind,
                payload,
            };
            let result = self
                .registry
                .update(cx, |registry, _cx| registry.dispatch_event(event));
            if let Err(error) = result {
                self.error_message = Some(error.to_string().into());
                cx.notify();
            }
        }

        fn dispatch_click(&mut self, handler_id: EventHandlerId, cx: &mut Context<Self>) {
            self.dispatch_event(handler_id, UiEventKind::Click, None, cx);
        }

        fn render_node(&self, node: &UiNode, cx: &mut Context<Self>) -> gpui::AnyElement {
            match node.kind {
                UiNodeKind::Empty => gpui::Empty.into_any_element(),
                UiNodeKind::Div => {
                    let element = apply_styles(div(), &node.styles).children(
                        node.children
                            .iter()
                            .map(|child| self.render_node(child, cx)),
                    );
                    let element = if node.events.is_empty() {
                        element.into_any_element()
                    } else {
                        apply_div_events(
                            element.id(format!(
                                "plugin-panel-div-{}-{}",
                                self.panel_instance_id, node.events[0].handler_id
                            )),
                            node,
                            cx,
                        )
                        .into_any_element()
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
                    if let Some(weight) =
                        style_number(&node.props, "font_weight").map(gpui::FontWeight)
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
                    let button_id = node
                        .events
                        .first()
                        .map(|event| format!("plugin-button-{}", event.handler_id))
                        .unwrap_or_else(|| {
                            format!(
                                "plugin-button-{}-{}",
                                self.panel_instance_id, self.panel_entity_id
                            )
                        });
                    render_button_node(node, button_id, cx)
                }
                UiNodeKind::Divider => render_divider_node(node),
                UiNodeKind::Indicator => render_indicator_node(node),
                UiNodeKind::Icon => {
                    let mut icon = Icon::new(
                        icon_name_from_descriptor(node.text.as_deref())
                            .unwrap_or(IconName::AiOpenAi),
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
                UiNodeKind::ProgressBar => render_progress_bar_node(node, cx),
                UiNodeKind::MenuItem => gpui::Empty.into_any_element(),
            }
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
            plugin_panel_persistence_key(self._plugin_id.as_str(), &self.descriptor.id).into()
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

            if let Some(tree) = self.tree.clone() {
                body.child(self.render_node(&tree, cx))
            } else {
                body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_full()
                        .child(Label::new("Loading...").color(Color::Muted)),
                )
            }
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
                                eprintln!("failed to dispatch remote action event: {error:?}");
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
                button = button.on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch_plugin_event(handler_id.clone(), UiEventKind::Click, None, cx);
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
            button = button.on_click(cx.listener(move |this, _, _, cx| {
                this.dispatch_plugin_event(handler_id.clone(), UiEventKind::Click, None, cx);
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

    fn render_progress_bar_node(node: &UiNode, cx: &mut App) -> gpui::AnyElement {
        let progress_id = node
            .props
            .get("element_id")
            .and_then(|value| match value {
                StyleValue::Text(value) => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "plugin-progress".to_string());
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
                    plugin_id: snapshot._plugin_id.clone(),
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

    fn spawn_process(
        plugin: &InstalledPlugin,
        event_sender: channel::Sender<PluginHostEvent>,
        async_cx: gpui::AsyncApp,
        process_instance_id: u64,
    ) -> Result<SpawnedPluginProcess> {
        let mut command = build_command(plugin)?;
        let plugin_id = plugin.manifest.id.clone();
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to spawn plugin process {command:?}"))?;

        let stdin = child
            .stdin
            .take()
            .context("plugin process did not expose stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("plugin process did not expose stdout")?;
        let stderr = child
            .stderr
            .take()
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
                let mut termination_reason = None;
                let status = child.status().fuse();
                let terminate = terminate_receiver.recv().fuse();
                pin_mut!(status, terminate);

                let exit_status = match select(status, terminate).await {
                    Either::Left((status, _)) => status.ok().and_then(|status| status.code()),
                    Either::Right((termination, _)) => {
                        termination_reason = termination.ok();
                        child.kill().ok();
                        child.status().await.ok().and_then(|status| status.code())
                    }
                };
                event_sender
                    .send(PluginHostEvent::Exited {
                        plugin_id: exit_plugin_id,
                        process_instance_id,
                        exit_status,
                        error_message: termination_reason
                            .and_then(|reason| reason.error_message(&plugin_id)),
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
        } else if plugin.installation.root.join("Cargo.toml").exists() {
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

    fn build_launch_spec(plugin: &InstalledPlugin) -> Result<PluginLaunchSpec> {
        let plugin_root = plugin.installation.root.clone();
        let cargo_manifest = plugin_root.join("Cargo.toml");

        if cargo_manifest.exists() {
            return Ok(PluginLaunchSpec {
                program: OsString::from("cargo"),
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
    fn insert_home_runtime_paths(paths: &mut BTreeSet<PathBuf>) {
        paths.insert(env::temp_dir());

        if let Some(tmpdir) = env::var_os("TMPDIR").map(PathBuf::from) {
            paths.insert(tmpdir);
        }

        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths::home_dir().clone());
        if !home_dir.as_os_str().is_empty() {
            paths.insert(home_dir.join("Library").join("Keychains"));
            paths.insert(home_dir.join("Library").join("Preferences"));
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
    fn sbpl_quote(value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('\"', "\\\"");
        format!("\"{escaped}\"")
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
        for path in readable_paths {
            profile.push_str(&format!(
                "       (subpath {})\n",
                sbpl_quote(path.as_os_str().to_string_lossy().as_ref())
            ));
        }
        profile.push_str(")\n");

        let writable_paths = sandbox_writable_paths(plugin, launch_spec);
        profile.push_str("(allow file-write*\n");
        for path in writable_paths {
            profile.push_str(&format!(
                "       (subpath {})\n",
                sbpl_quote(path.as_os_str().to_string_lossy().as_ref())
            ));
        }
        profile.push_str(")\n");
        profile.push_str(
            r#"(allow network*)
(allow process-fork)
(allow process-exec*)
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

                    workspace.add_panel(panel_a.clone(), window, workspace_cx);
                    workspace.add_panel(panel_b.clone(), window, workspace_cx);
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
            assert_eq!(args[2], "cargo");
            assert_eq!(args[3], "run");
            assert_eq!(target_dir, cargo_target_dir(&plugin_root).to_string_lossy());
        }

        #[cfg(target_os = "macos")]
        #[test]
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
                                root: initial_root.clone(),
                            },
                            registry_cx,
                        )?;
                        registry.apply_message(
                            PluginId::new("test-plugin"),
                            PluginToHost::RenderDelta {
                                panel_id: String::from("panel-a"),
                                panel_instance_id: panel_instance_id.clone(),
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
