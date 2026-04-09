use anyhow::Context as _;
use futures::{FutureExt as _, future::select, pin_mut};
use gpui_api::{RenderContext, StyleMap, UiNode, UiNodeKind};
use plugin_protocol::{
    HostThemeSnapshot, HostToPlugin, INTERACTIVE_PROP_BLOCK_MOUSE_EXCEPT_SCROLL,
    INTERACTIVE_PROP_FOCUSABLE, INTERACTIVE_PROP_GROUP, INTERACTIVE_PROP_KEY_CONTEXT,
    INTERACTIVE_PROP_OCCLUDE, INTERACTIVE_PROP_TAB_GROUP, INTERACTIVE_PROP_TAB_INDEX,
    INTERACTIVE_PROP_TAB_STOP, INTERACTIVE_PROP_WINDOW_CONTROL_AREA, PanelDescriptor,
    PanelInstanceId, PluginActionDescriptor, PluginId, PluginMetadata, PluginToHost,
    SerializedActionEvent, SerializedClickEvent, SerializedKeyDownEvent, SerializedKeyUpEvent,
    SerializedModifiersChangedEvent, SerializedMouseDownEvent, SerializedMouseMoveEvent,
    SerializedMousePressureEvent, SerializedMouseUpEvent, SerializedPinchEvent,
    SerializedScrollWheelEvent, TitlebarWidgetDescriptor, UiEventKind as ProtocolUiEventKind,
    diff_ui_trees,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::{BufRead as _, BufWriter},
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub use gpui::{actions, private};
#[cfg(any(test, feature = "test-support"))]
pub use gpui_api::proptest;
pub use gpui_api::{
    AbsoluteLength, Action, ActiveTheme, AlignContent, AlignItems, AlignSelf, Animation,
    AnimationElement, AnimationExt, AnyElement, AnyView, AnyWindowHandle, App, AppContext, ArcCow,
    AsyncApp, AsyncWindowContext, AvailableSpace, BackgroundExecutor, BorderStyle,
    BorrowAppContext, ClickEvent, Component, Context, CursorStyle, Deferred, DefiniteLength,
    DispatchPhase, Display, Element, ElementId, Empty, Entity, EventEmitter, EventPhase, Fill,
    FlexDirection, FlexWrap, FluentBuilder, FocusHandle, Focusable, FontWeight, ForegroundExecutor,
    FutureExt, Global, GpuSpecs, GpuiBorrow, Hsla, InteractiveElement, Interactivity, IntoElement,
    JustifyContent, KeyContext, KeyDownEvent, KeyUpEvent, KeyboardButton, Keystroke, LayoutId,
    Length, MacroActionBuilder, MacroActionData, Modifiers, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MousePressureEvent, MouseUpEvent, NavigationDirection,
    NoAction, Overflow, ParentElement, PinchEvent, Pixels, Point, PressureStage, Priority, Radians,
    ReadGlobal, Refineable, Rems, Render, RenderOnce, RenderOutput, RenderRoot, Reservation,
    Result, Runtime, ScrollAnchor, ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString, Size,
    Stateful, StatefulInteractiveElement, Styled, StyledImage, Subscription, Task, TextAlign,
    TextOverflow, Timeout, TouchPhase, Transformation, UiEvent, UiNodeEvent,
    UiNodeKind as NodeKind, Unbind, UpdateGlobal, VisualContext, WeakEntity, WhiteSpace, Window,
    WindowControlArea, WindowHandle, WindowId, http_client, is_no_action, is_unbind, point, px,
    radians, relative, rems, size,
};
#[cfg(any(target_os = "windows", target_os = "linux", target_family = "wasm"))]
pub use gpui_api::{PriorityQueueReceiver, PriorityQueueSender};
pub use gpui_macros::{
    AppContext, IntoElement, Render, VisualContext, property_test, register_action, test,
};
pub use plugin_protocol::{PluginHostRequest, PluginHostResponse};

pub mod prelude {
    pub use crate::{
        ActiveTheme, AnyElement, App, AppContext as _, BorrowAppContext, Component, Context,
        Deferred, Element, ElementId, Empty, FluentBuilder, FontWeight, InteractiveElement,
        IntoElement, ParentElement, Refineable, Render, RenderOnce, Stateful,
        StatefulInteractiveElement, Styled, StyledImage, VisualContext,
    };
}

#[derive(Default)]
pub struct Div {
    styles: StyleMap,
    children: Vec<AnyElement>,
    interactivity: Interactivity,
}

impl Div {
    pub fn child(self, child: impl gpui_api::IntoElement) -> Self {
        ParentElement::child(self, child)
    }

    pub fn children(self, children: impl IntoIterator<Item = impl gpui_api::IntoElement>) -> Self {
        ParentElement::children(self, children)
    }
}

impl Element for Div {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Div);
        node.styles = self.styles;
        node.styles.refine(&self.interactivity.base_style);
        if let Some(element_id) = self.interactivity.element_id.as_ref() {
            let element_id = element_id.to_string();
            node.element_id = Some(element_id.clone());
            node.props
                .insert("element_id".to_string(), element_id.into());
        }
        serialize_interactivity_props(&mut node.props, &self.interactivity);
        node.events = Iterator::map(self.interactivity.handlers().iter().cloned(), |handler| {
            context.register_event_handler(handler)
        })
        .collect();
        node.children =
            Iterator::map(self.children.into_iter(), |child| child.into_node(context)).collect();
        node
    }
}

fn serialize_interactivity_props(
    props: &mut BTreeMap<String, plugin_protocol::StyleValue>,
    interactivity: &Interactivity,
) {
    if let Some(group_name) = interactivity.group.as_ref() {
        props.insert(
            INTERACTIVE_PROP_GROUP.to_string(),
            group_name.to_string().into(),
        );
    }
    if interactivity.tab_stop || interactivity.tab_index.is_some() {
        props.insert(
            INTERACTIVE_PROP_TAB_STOP.to_string(),
            interactivity.tab_stop.into(),
        );
    }
    if let Some(tab_index) = interactivity.tab_index {
        props.insert(
            INTERACTIVE_PROP_TAB_INDEX.to_string(),
            (tab_index as f32).into(),
        );
    }
    if interactivity.tab_group {
        props.insert(INTERACTIVE_PROP_TAB_GROUP.to_string(), true.into());
    }
    if interactivity.focusable {
        props.insert(INTERACTIVE_PROP_FOCUSABLE.to_string(), true.into());
    }
    if let Some(key_context) = interactivity.key_context.as_ref() {
        props.insert(
            INTERACTIVE_PROP_KEY_CONTEXT.to_string(),
            key_context.clone().into(),
        );
    }
    if let Some(window_control_area) = interactivity.window_control_area {
        props.insert(
            INTERACTIVE_PROP_WINDOW_CONTROL_AREA.to_string(),
            window_control_area_name(window_control_area)
                .to_string()
                .into(),
        );
    }
    if interactivity.occlude {
        props.insert(INTERACTIVE_PROP_OCCLUDE.to_string(), true.into());
    }
    if interactivity.block_mouse_except_scroll {
        props.insert(
            INTERACTIVE_PROP_BLOCK_MOUSE_EXCEPT_SCROLL.to_string(),
            true.into(),
        );
    }
}

fn window_control_area_name(window_control_area: WindowControlArea) -> &'static str {
    match window_control_area {
        WindowControlArea::Drag => "drag",
        WindowControlArea::Close => "close",
        WindowControlArea::Max => "max",
        WindowControlArea::Min => "min",
    }
}

impl ParentElement for Div {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Div {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

impl InteractiveElement for Div {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

pub fn div() -> Div {
    Div::default()
}

pub fn h_flex() -> Div {
    div().flex().flex_row().items_center()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteViewKind {
    Panel,
    TitlebarWidget,
}

impl RemoteViewKind {
    fn noun(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::TitlebarWidget => "titlebar widget",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RemoteViewDescriptor {
    Panel(PanelDescriptor),
    TitlebarWidget(TitlebarWidgetDescriptor),
}

impl RemoteViewDescriptor {
    fn id(&self) -> &str {
        match self {
            Self::Panel(descriptor) => &descriptor.id,
            Self::TitlebarWidget(descriptor) => &descriptor.id,
        }
    }

    fn kind(&self) -> RemoteViewKind {
        match self {
            Self::Panel(_) => RemoteViewKind::Panel,
            Self::TitlebarWidget(_) => RemoteViewKind::TitlebarWidget,
        }
    }
}

type SessionFactory =
    Box<dyn Fn(PanelInstanceId, String) -> Result<Box<dyn ActiveRemoteViewSession>>>;
type ActionHandler = Box<dyn FnMut() -> Result<()>>;

trait ActiveRemoteViewSession {
    fn initial_messages(&mut self) -> Result<Vec<PluginToHost>>;
    fn handle_theme_changed(&mut self, theme: &HostThemeSnapshot) -> Result<Vec<PluginToHost>>;
    fn handle_event(&mut self, event: &plugin_protocol::UiEvent) -> Result<Vec<PluginToHost>>;
    fn drain_messages(&mut self) -> Result<Vec<PluginToHost>>;
    fn shutdown(&mut self) -> Result<()>;
}

struct RenderSession<T: 'static> {
    view_id: String,
    view_instance_id: PanelInstanceId,
    runtime: Runtime,
    root: Entity<T>,
    last_render: Option<RenderOutput>,
}

impl<T: Render + 'static> RenderSession<T> {
    fn new(
        view_id: String,
        view_instance_id: PanelInstanceId,
        build: impl Fn(&mut Context<T>) -> T,
    ) -> Self {
        let mut runtime = Runtime::new();
        let root = runtime.new_entity(build);
        Self {
            view_id,
            view_instance_id,
            runtime,
            root,
            last_render: None,
        }
    }

    fn render_message(&mut self) -> Result<PluginToHost> {
        let render = self.runtime.render_root(&self.root)?;
        let generation = Some(render.generation());
        let message = if let Some(previous_render) = self.last_render.as_ref() {
            PluginToHost::RenderDelta {
                panel_id: self.view_id.clone(),
                panel_instance_id: self.view_instance_id.clone(),
                generation,
                patches: diff_ui_trees(&previous_render.tree, &render.tree),
            }
        } else {
            PluginToHost::Render {
                panel_id: self.view_id.clone(),
                panel_instance_id: self.view_instance_id.clone(),
                generation,
                root: render.tree.clone(),
            }
        };
        self.last_render = Some(render);
        Ok(message)
    }

    fn collect_messages(&mut self) -> Result<Vec<PluginToHost>> {
        let mut messages = Iterator::map(self.runtime.take_errors().into_iter(), |message| {
            PluginToHost::ReportError {
                panel_instance_id: Some(self.view_instance_id.clone()),
                message,
            }
        })
        .collect::<Vec<_>>();

        if self.last_render.is_none() || self.runtime.is_dirty(self.root.entity_id()) {
            messages.push(self.render_message()?);
        }

        Ok(messages)
    }

    fn apply_host_theme(&mut self, theme: &HostThemeSnapshot) {
        self.runtime.apply_host_theme(theme);
        self.root.update(&mut self.runtime, |_this, cx| cx.notify());
    }
}

impl<T: Render + 'static> ActiveRemoteViewSession for RenderSession<T> {
    fn initial_messages(&mut self) -> Result<Vec<PluginToHost>> {
        Ok(vec![self.render_message()?])
    }

    fn handle_theme_changed(&mut self, theme: &HostThemeSnapshot) -> Result<Vec<PluginToHost>> {
        self.apply_host_theme(theme);
        if self.last_render.is_some() {
            self.collect_messages()
        } else {
            Ok(Vec::new())
        }
    }

    fn handle_event(&mut self, event: &plugin_protocol::UiEvent) -> Result<Vec<PluginToHost>> {
        let Some(render) = self.last_render.as_ref() else {
            anyhow::bail!("view {} has not been rendered yet", self.view_id);
        };
        if event
            .generation
            .is_some_and(|generation| generation != render.generation())
        {
            return Ok(Vec::new());
        }

        let runtime_event = match event.kind {
            ProtocolUiEventKind::Click => UiEvent::Click(
                decode_payload::<SerializedClickEvent>(&event.payload, "click")?.into(),
            ),
            ProtocolUiEventKind::AuxClick => UiEvent::AuxClick(
                decode_payload::<SerializedClickEvent>(&event.payload, "aux_click")?.into(),
            ),
            ProtocolUiEventKind::MouseDown => UiEvent::MouseDown(
                decode_payload::<SerializedMouseDownEvent>(&event.payload, "mouse_down")?.into(),
            ),
            ProtocolUiEventKind::MouseUp => UiEvent::MouseUp(
                decode_payload::<SerializedMouseUpEvent>(&event.payload, "mouse_up")?.into(),
            ),
            ProtocolUiEventKind::MouseMove => UiEvent::MouseMove(
                decode_payload::<SerializedMouseMoveEvent>(&event.payload, "mouse_move")?.into(),
            ),
            ProtocolUiEventKind::MousePressure => UiEvent::MousePressure(
                decode_payload::<SerializedMousePressureEvent>(&event.payload, "mouse_pressure")?
                    .into(),
            ),
            ProtocolUiEventKind::Hover => {
                UiEvent::Hover(decode_payload::<bool>(&event.payload, "hover")?)
            }
            ProtocolUiEventKind::KeyDown => UiEvent::KeyDown(
                decode_payload::<SerializedKeyDownEvent>(&event.payload, "key_down")?.into(),
            ),
            ProtocolUiEventKind::KeyUp => UiEvent::KeyUp(
                decode_payload::<SerializedKeyUpEvent>(&event.payload, "key_up")?.into(),
            ),
            ProtocolUiEventKind::ModifiersChanged => UiEvent::ModifiersChanged(
                decode_payload::<SerializedModifiersChangedEvent>(
                    &event.payload,
                    "modifiers_changed",
                )?
                .into(),
            ),
            ProtocolUiEventKind::ScrollWheel => UiEvent::ScrollWheel(
                decode_payload::<SerializedScrollWheelEvent>(&event.payload, "scroll_wheel")?
                    .into(),
            ),
            ProtocolUiEventKind::Pinch => UiEvent::Pinch(
                decode_payload::<SerializedPinchEvent>(&event.payload, "pinch")?.into(),
            ),
            ProtocolUiEventKind::Action => {
                let action = decode_payload::<SerializedActionEvent>(&event.payload, "action")?;
                UiEvent::Action(gpui_api::ActionEvent {
                    name: action.name,
                    payload: action.payload,
                })
            }
        };

        let mut window = Window::default();
        render.dispatch(
            event.handler_id.clone(),
            &runtime_event,
            &mut window,
            &mut self.runtime,
        )?;

        self.collect_messages()
    }

    fn drain_messages(&mut self) -> Result<Vec<PluginToHost>> {
        let _ = self.runtime.drain_tasks();
        self.collect_messages()
    }

    fn shutdown(&mut self) -> Result<()> {
        self.runtime.run_app_quit_callbacks();
        Ok(())
    }
}

fn decode_payload<T: for<'de> Deserialize<'de>>(
    payload: &Option<serde_json::Value>,
    kind: &str,
) -> Result<T> {
    let payload = payload
        .clone()
        .with_context(|| format!("missing plugin event payload for {kind}"))?;
    serde_json::from_value(payload)
        .with_context(|| format!("failed to decode plugin event payload for {kind}"))
}

pub struct PluginApp {
    metadata: PluginMetadata,
    remote_views: BTreeMap<String, RemoteViewDescriptor>,
    actions: BTreeMap<String, PluginActionDescriptor>,
    factories: BTreeMap<String, SessionFactory>,
    action_handlers: BTreeMap<String, ActionHandler>,
    sessions: BTreeMap<PanelInstanceId, Box<dyn ActiveRemoteViewSession>>,
    registration_errors: Vec<anyhow::Error>,
}

impl PluginApp {
    pub fn register_panel<T>(
        &mut self,
        panel_id: impl Into<String>,
        build: impl Fn(&mut Context<T>) -> T + 'static,
    ) where
        T: Render,
    {
        self.register_remote_view(panel_id, RemoteViewKind::Panel, build);
    }

    pub fn register_titlebar_widget<T>(
        &mut self,
        widget_id: impl Into<String>,
        build: impl Fn(&mut Context<T>) -> T + 'static,
    ) where
        T: Render,
    {
        self.register_remote_view(widget_id, RemoteViewKind::TitlebarWidget, build);
    }

    pub fn register_action(
        &mut self,
        action_id: impl Into<String>,
        handler: impl FnMut() -> Result<()> + 'static,
    ) {
        let action_id = action_id.into();
        if !self.actions.contains_key(&action_id) {
            self.registration_errors.push(anyhow::anyhow!(
                "action `{action_id}` is not declared in plugin.toml"
            ));
            return;
        }

        if self.action_handlers.contains_key(&action_id) {
            self.registration_errors.push(anyhow::anyhow!(
                "action `{action_id}` has multiple runtime handlers"
            ));
            return;
        }

        self.action_handlers.insert(action_id, Box::new(handler));
    }

    fn register_remote_view<T>(
        &mut self,
        view_id: impl Into<String>,
        kind: RemoteViewKind,
        build: impl Fn(&mut Context<T>) -> T + 'static,
    ) where
        T: Render,
    {
        let view_id = view_id.into();
        let Some(descriptor) = self.remote_views.get(&view_id) else {
            self.registration_errors.push(anyhow::anyhow!(
                "{} `{view_id}` is not declared in plugin.toml",
                kind.noun()
            ));
            return;
        };

        if descriptor.kind() != kind {
            self.registration_errors.push(anyhow::anyhow!(
                "{} `{view_id}` is declared in plugin.toml as a {}",
                kind.noun(),
                descriptor.kind().noun()
            ));
            return;
        }

        let build = Rc::new(build);
        self.factories.insert(
            view_id.clone(),
            Box::new(move |view_instance_id, requested_view_id| {
                let build = build.clone();
                Ok(Box::new(RenderSession::<T>::new(
                    requested_view_id,
                    view_instance_id,
                    move |cx| build(cx),
                )))
            }),
        );
    }

    fn from_current_directory() -> Result<Self> {
        let manifest = RuntimeManifest::load(&std::env::current_dir()?)?;
        let metadata = PluginMetadata::from(manifest);
        let mut remote_views = BTreeMap::new();
        for descriptor in
            Iterator::map(metadata.panels.iter().cloned(), RemoteViewDescriptor::Panel).chain(
                Iterator::map(
                    metadata.titlebar_widgets.iter().cloned(),
                    RemoteViewDescriptor::TitlebarWidget,
                ),
            )
        {
            let view_id = descriptor.id().to_string();
            if remote_views.insert(view_id.clone(), descriptor).is_some() {
                anyhow::bail!("duplicate remote view `{view_id}` in plugin.toml");
            }
        }

        let mut actions = BTreeMap::new();
        for descriptor in &metadata.actions {
            if actions
                .insert(descriptor.id.clone(), descriptor.clone())
                .is_some()
            {
                anyhow::bail!("duplicate action `{}` in plugin.toml", descriptor.id);
            }
        }

        Ok(Self {
            metadata,
            remote_views,
            actions,
            factories: BTreeMap::default(),
            action_handlers: BTreeMap::default(),
            sessions: BTreeMap::default(),
            registration_errors: Vec::new(),
        })
    }

    fn validate_registrations(&self) -> Result<()> {
        if let Some(error) = self.registration_errors.first() {
            return Err(anyhow::anyhow!(error.to_string()));
        }

        for descriptor in self.remote_views.values() {
            if !self.factories.contains_key(descriptor.id()) {
                anyhow::bail!(
                    "{} `{}` is declared in plugin.toml but has no registered implementation",
                    descriptor.kind().noun(),
                    descriptor.id()
                );
            }
        }

        Ok(())
    }

    fn invoke_action(&mut self, action_id: &str) -> Result<()> {
        let handler = self
            .action_handlers
            .get_mut(action_id)
            .with_context(|| format!("action `{action_id}` has no runtime handler"))?;
        handler()
    }

    fn handle_message(&mut self, message: HostToPlugin) -> Result<LoopControl> {
        match message {
            HostToPlugin::OpenPanel {
                panel_id,
                panel_instance_id,
                theme,
            } => {
                let factory = self
                    .factories
                    .get(&panel_id)
                    .with_context(|| format!("unknown remote view `{panel_id}`"))?;
                let mut session = factory(panel_instance_id.clone(), panel_id)?;
                session.handle_theme_changed(&theme)?;
                let messages = session.initial_messages()?;
                self.sessions.insert(panel_instance_id, session);
                Ok(LoopControl::Continue(messages))
            }
            HostToPlugin::ThemeChanged { theme } => {
                let mut messages = Vec::new();
                for session in self.sessions.values_mut() {
                    messages.extend(session.handle_theme_changed(&theme)?);
                }
                Ok(LoopControl::Continue(messages))
            }
            HostToPlugin::DispatchEvent { event } => {
                let Some(session) = self.sessions.get_mut(&event.panel_instance_id) else {
                    return Ok(LoopControl::Continue(vec![PluginToHost::ReportError {
                        panel_instance_id: Some(event.panel_instance_id.clone()),
                        message: format!(
                            "remote view session `{}` is not open",
                            event.panel_instance_id
                        ),
                    }]));
                };
                Ok(LoopControl::Continue(session.handle_event(&event)?))
            }
            HostToPlugin::InvokeAction { action_id } => {
                let mut messages = Vec::new();
                if let Err(error) = self.invoke_action(&action_id) {
                    messages.push(PluginToHost::ReportError {
                        panel_instance_id: None,
                        message: error.to_string(),
                    });
                }
                messages.extend(self.drain_messages()?);
                Ok(LoopControl::Continue(messages))
            }
            HostToPlugin::ClosePanel { panel_instance_id } => {
                self.sessions.remove(&panel_instance_id);
                Ok(LoopControl::Continue(vec![PluginToHost::ClosePanel {
                    panel_instance_id,
                }]))
            }
            HostToPlugin::HostResponse { .. } => Ok(LoopControl::Continue(Vec::new())),
            HostToPlugin::Shutdown => {
                for session in self.sessions.values_mut() {
                    session.shutdown()?;
                }
                Ok(LoopControl::Shutdown)
            }
        }
    }

    fn drain_messages(&mut self) -> Result<Vec<PluginToHost>> {
        let mut messages = Vec::new();
        for session in self.sessions.values_mut() {
            messages.extend(session.drain_messages()?);
        }
        Ok(messages)
    }
}

enum LoopControl {
    Continue(Vec<PluginToHost>),
    Shutdown,
}

#[derive(Clone, Debug, Deserialize)]
struct RuntimeManifest {
    id: PluginId,
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    panels: Vec<PanelDescriptor>,
    #[serde(default)]
    titlebar_widgets: Vec<plugin_protocol::TitlebarWidgetDescriptor>,
    #[serde(default)]
    actions: Vec<PluginActionDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;
    use std::cell::RefCell;
    use theme::{Appearance, StatusColorsRefinement, ThemeColorsRefinement};

    struct StubSession;

    impl ActiveRemoteViewSession for StubSession {
        fn initial_messages(&mut self) -> Result<Vec<PluginToHost>> {
            Ok(Vec::new())
        }

        fn handle_theme_changed(
            &mut self,
            _theme: &HostThemeSnapshot,
        ) -> Result<Vec<PluginToHost>> {
            Ok(Vec::new())
        }

        fn handle_event(&mut self, _event: &plugin_protocol::UiEvent) -> Result<Vec<PluginToHost>> {
            Ok(Vec::new())
        }

        fn drain_messages(&mut self) -> Result<Vec<PluginToHost>> {
            Ok(Vec::new())
        }

        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[::core::prelude::v1::test]
    fn validates_panel_and_titlebar_widget_registrations() {
        let mut factories: BTreeMap<String, SessionFactory> = BTreeMap::new();
        let deploy_factory: SessionFactory =
            Box::new(|_, _| Ok(Box::new(StubSession) as Box<dyn ActiveRemoteViewSession>));
        let search_factory: SessionFactory =
            Box::new(|_, _| Ok(Box::new(StubSession) as Box<dyn ActiveRemoteViewSession>));
        factories.insert("deploy-panel".to_string(), deploy_factory);
        factories.insert("search-widget".to_string(), search_factory);

        let app = PluginApp {
            metadata: PluginMetadata {
                id: PluginId::new("acme.test-panel"),
                name: "Test Panel".into(),
                version: "0.1.0".into(),
                description: None,
                panels: vec![PanelDescriptor {
                    id: "deploy-panel".into(),
                    title: "Deploy".into(),
                    dock: plugin_protocol::DockPosition::Right,
                    icon_name: None,
                    tooltip: None,
                    activation: plugin_protocol::PanelActivation::OnDemand,
                }],
                titlebar_widgets: vec![plugin_protocol::TitlebarWidgetDescriptor {
                    id: "search-widget".into(),
                    title: "Search".into(),
                    icon_name: None,
                    tooltip: None,
                    side: plugin_protocol::TitlebarWidgetSide::Right,
                    priority: 100,
                    opens_panel_id: Some("deploy-panel".into()),
                }],
                actions: vec![PluginActionDescriptor {
                    id: "deploy-now".into(),
                    title: "Deploy Now".into(),
                    description: None,
                }],
            },
            remote_views: BTreeMap::from([
                (
                    "deploy-panel".to_string(),
                    RemoteViewDescriptor::Panel(PanelDescriptor {
                        id: "deploy-panel".into(),
                        title: "Deploy".into(),
                        dock: plugin_protocol::DockPosition::Right,
                        icon_name: None,
                        tooltip: None,
                        activation: plugin_protocol::PanelActivation::OnDemand,
                    }),
                ),
                (
                    "search-widget".to_string(),
                    RemoteViewDescriptor::TitlebarWidget(
                        plugin_protocol::TitlebarWidgetDescriptor {
                            id: "search-widget".into(),
                            title: "Search".into(),
                            icon_name: None,
                            tooltip: None,
                            side: plugin_protocol::TitlebarWidgetSide::Right,
                            priority: 100,
                            opens_panel_id: Some("deploy-panel".into()),
                        },
                    ),
                ),
            ]),
            actions: BTreeMap::from([(
                "deploy-now".to_string(),
                PluginActionDescriptor {
                    id: "deploy-now".into(),
                    title: "Deploy Now".into(),
                    description: None,
                },
            )]),
            factories,
            action_handlers: BTreeMap::default(),
            sessions: BTreeMap::default(),
            registration_errors: Vec::new(),
        };

        app.validate_registrations().unwrap();
    }

    #[::core::prelude::v1::test]
    fn invoke_action_without_runtime_handler_reports_error() {
        let mut app = PluginApp {
            metadata: PluginMetadata {
                id: PluginId::new("acme.test-panel"),
                name: "Test Panel".into(),
                version: "0.1.0".into(),
                description: None,
                panels: Vec::new(),
                titlebar_widgets: Vec::new(),
                actions: vec![PluginActionDescriptor {
                    id: "deploy-now".into(),
                    title: "Deploy Now".into(),
                    description: None,
                }],
            },
            remote_views: BTreeMap::default(),
            actions: BTreeMap::from([(
                "deploy-now".to_string(),
                PluginActionDescriptor {
                    id: "deploy-now".into(),
                    title: "Deploy Now".into(),
                    description: None,
                },
            )]),
            factories: BTreeMap::default(),
            action_handlers: BTreeMap::default(),
            sessions: BTreeMap::default(),
            registration_errors: Vec::new(),
        };

        let result = app
            .handle_message(HostToPlugin::InvokeAction {
                action_id: "deploy-now".to_string(),
            })
            .expect("invoke action should not crash");

        let LoopControl::Continue(messages) = result else {
            panic!("invoke action should not request shutdown");
        };
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            &messages[0],
            PluginToHost::ReportError {
                panel_instance_id: None,
                message,
            } if message.contains("deploy-now")
        ));
    }

    struct ThemeAwarePanel;

    impl Render for ThemeAwarePanel {
        fn render(
            &mut self,
            _window: &mut Window,
            cx: &mut Context<Self>,
        ) -> impl gpui_api::IntoElement {
            div().bg(cx.theme().colors().background)
        }
    }

    struct ShutdownAwarePanel {
        _subscriptions: Vec<Subscription>,
    }

    impl Render for ShutdownAwarePanel {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> impl gpui_api::IntoElement {
            div()
        }
    }

    struct StaleGenerationPanel {
        initial_click_count: usize,
        replacement_click_count: usize,
        show_initial_handler: bool,
    }

    impl Render for StaleGenerationPanel {
        fn render(
            &mut self,
            _window: &mut Window,
            cx: &mut Context<Self>,
        ) -> impl gpui_api::IntoElement {
            if self.show_initial_handler {
                div()
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.initial_click_count = this.initial_click_count.saturating_add(1);
                        this.show_initial_handler = false;
                        cx.notify();
                    }))
                    .child("initial-handler")
            } else {
                div()
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.replacement_click_count =
                            this.replacement_click_count.saturating_add(1);
                        cx.notify();
                    }))
                    .child("replacement-handler")
            }
        }
    }

    #[::core::prelude::v1::test]
    fn stale_generation_events_do_not_invoke_replacement_handlers() {
        let panel_instance_id = PanelInstanceId::new("stale-generation-panel");
        let mut session = RenderSession::new(
            "stale-generation-panel".to_string(),
            panel_instance_id.clone(),
            |_cx| StaleGenerationPanel {
                initial_click_count: 0,
                replacement_click_count: 0,
                show_initial_handler: true,
            },
        );

        let initial_message = session
            .initial_messages()
            .expect("initial render should succeed")
            .into_iter()
            .next()
            .expect("initial render should emit one message");
        let (initial_generation, initial_handler_id) = match initial_message {
            PluginToHost::Render {
                generation, root, ..
            } => (
                generation.expect("render message should include generation"),
                root.events
                    .first()
                    .expect("initial root should include click handler")
                    .handler_id
                    .clone(),
            ),
            other => panic!("expected initial render message, got {other:?}"),
        };

        let click_payload = Some(
            serde_json::to_value(SerializedClickEvent::from(&ClickEvent::default()))
                .expect("click payload should serialize"),
        );

        let rerender_messages = session
            .handle_event(&plugin_protocol::UiEvent {
                panel_instance_id: panel_instance_id.clone(),
                generation: Some(initial_generation),
                handler_id: initial_handler_id.clone(),
                kind: ProtocolUiEventKind::Click,
                payload: click_payload.clone(),
            })
            .expect("initial click dispatch should succeed");
        assert!(
            rerender_messages
                .iter()
                .any(|message| matches!(message, PluginToHost::RenderDelta { .. })),
            "initial handler should trigger a rerender"
        );

        let stale_dispatch_messages = session
            .handle_event(&plugin_protocol::UiEvent {
                panel_instance_id,
                generation: Some(initial_generation),
                handler_id: initial_handler_id,
                kind: ProtocolUiEventKind::Click,
                payload: click_payload,
            })
            .expect("stale click dispatch should not crash");

        assert!(
            stale_dispatch_messages.is_empty(),
            "stale handler dispatch should be ignored"
        );

        session.root.read(|panel| {
            assert_eq!(panel.initial_click_count, 1);
            assert_eq!(panel.replacement_click_count, 0);
        });
    }

    #[::core::prelude::v1::test]
    fn theme_change_updates_runtime_theme_and_requests_rerender() {
        let mut session = RenderSession::new(
            "theme-panel".to_string(),
            PanelInstanceId::new("panel-1"),
            |_cx| ThemeAwarePanel,
        );

        let initial_messages = session.initial_messages().unwrap();
        assert_eq!(initial_messages.len(), 1);
        assert_eq!(session.runtime.theme().appearance, Appearance::Dark);

        let messages = session
            .handle_theme_changed(&HostThemeSnapshot {
                id: "custom-light".to_string(),
                name: "Custom Light".to_string(),
                appearance: Appearance::Light,
                colors: ThemeColorsRefinement {
                    background: Some(rgb(0xffffff).into()),
                    ..Default::default()
                },
                status: StatusColorsRefinement {
                    info: Some(rgb(0x0000ff).into()),
                    ..Default::default()
                },
            })
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(
            messages.first(),
            Some(PluginToHost::RenderDelta { .. })
        ));
        assert_eq!(session.runtime.theme().appearance, Appearance::Light);
        assert_eq!(session.runtime.theme().name.as_ref(), "Custom Light");
    }

    #[::core::prelude::v1::test]
    fn shutdown_runs_on_app_quit_callbacks() {
        let quit_hits = Rc::new(RefCell::new(0));
        let mut session = RenderSession::new(
            "shutdown-panel".to_string(),
            PanelInstanceId::new("panel-quit"),
            {
                let quit_hits = quit_hits.clone();
                move |cx| {
                    let on_app_quit = {
                        let quit_hits = quit_hits.clone();
                        cx.on_app_quit(move |_this, _cx| {
                            let quit_hits = quit_hits.clone();
                            async move {
                                *quit_hits.borrow_mut() += 1;
                            }
                        })
                    };
                    ShutdownAwarePanel {
                        _subscriptions: vec![on_app_quit],
                    }
                }
            },
        );

        session.initial_messages().unwrap();
        session.shutdown().unwrap();

        assert_eq!(*quit_hits.borrow(), 1);
    }
}

impl RuntimeManifest {
    fn load(plugin_directory: &Path) -> Result<Self> {
        let manifest_path = plugin_directory.join("plugin.toml");
        let manifest_contents = std::fs::read_to_string(&manifest_path).with_context(|| {
            format!("failed to read plugin manifest {}", manifest_path.display())
        })?;
        toml::from_str(&manifest_contents).with_context(|| {
            format!(
                "failed to parse plugin manifest {}",
                manifest_path.display()
            )
        })
    }
}

impl From<RuntimeManifest> for PluginMetadata {
    fn from(manifest: RuntimeManifest) -> Self {
        Self {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            panels: manifest.panels,
            titlebar_widgets: manifest.titlebar_widgets,
            actions: manifest.actions,
        }
    }
}

enum OutboundMessage {
    Plugin(PluginToHost),
    HostRequest {
        request_id: u64,
        request: PluginHostRequest,
    },
}

struct PendingHostRequest {
    response_sender: mpsc::Sender<anyhow::Result<PluginHostResponse>>,
}

struct HostClient {
    outbound_sender: mpsc::Sender<OutboundMessage>,
    pending_requests: Arc<Mutex<BTreeMap<u64, PendingHostRequest>>>,
    next_request_id: AtomicU64,
}

static HOST_CLIENT: OnceLock<HostClient> = OnceLock::new();

pub fn host_request(request: PluginHostRequest) -> anyhow::Result<PluginHostResponse> {
    let host_client = HOST_CLIENT
        .get()
        .ok_or_else(|| anyhow::anyhow!("plugin host client is not initialized"))?;
    let request_id = host_client.next_request_id.fetch_add(1, Ordering::Relaxed);
    let (response_sender, response_receiver) = mpsc::channel();
    host_client
        .pending_requests
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin host request registry is poisoned"))?
        .insert(request_id, PendingHostRequest { response_sender });
    host_client
        .outbound_sender
        .send(OutboundMessage::HostRequest {
            request_id,
            request,
        })
        .map_err(|_| anyhow::anyhow!("failed to send host request to plugin runtime"))?;

    match response_receiver.recv() {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "plugin host request channel closed before a response was received"
        )),
    }
}

fn write_message(writer: &mut impl std::io::Write, message: &PluginToHost) -> Result<()> {
    serde_json::to_writer(&mut *writer, message)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn write_outbound_message(
    writer: &mut impl std::io::Write,
    message: &OutboundMessage,
) -> Result<()> {
    match message {
        OutboundMessage::Plugin(message) => write_message(writer, message),
        OutboundMessage::HostRequest {
            request_id,
            request,
        } => {
            serde_json::to_writer(
                &mut *writer,
                &PluginToHost::HostRequest {
                    request_id: *request_id,
                    request: request.clone(),
                },
            )?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            Ok(())
        }
    }
}

pub fn run(register: impl FnOnce(&mut PluginApp)) -> Result<()> {
    let mut app = PluginApp::from_current_directory()?;
    register(&mut app);
    app.validate_registrations()?;

    let (host_message_sender, host_message_receiver) = smol::channel::unbounded::<HostToPlugin>();
    let pending_requests = Arc::new(Mutex::new(BTreeMap::<u64, PendingHostRequest>::new()));
    let (outbound_sender, outbound_receiver) = mpsc::channel::<OutboundMessage>();

    HOST_CLIENT
        .set(HostClient {
            outbound_sender: outbound_sender.clone(),
            pending_requests: pending_requests.clone(),
            next_request_id: AtomicU64::new(1),
        })
        .map_err(|_| anyhow::anyhow!("plugin host client was initialized more than once"))?;

    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    let message: HostToPlugin = match serde_json::from_str(&line)
                        .with_context(|| "failed to decode host message".to_string())
                    {
                        Ok(message) => message,
                        Err(error) => {
                            eprintln!("{error:#}");
                            break;
                        }
                    };

                    match message {
                        HostToPlugin::HostResponse {
                            request_id,
                            response,
                            error,
                        } => {
                            let pending_request =
                                pending_requests
                                    .lock()
                                    .ok()
                                    .and_then(|mut pending_requests| {
                                        pending_requests.remove(&request_id)
                                    });
                            if let Some(pending_request) = pending_request {
                                let result = match (response, error) {
                                    (Some(response), None) => Ok(response),
                                    (None, Some(error)) => Err(anyhow::anyhow!(error)),
                                    (Some(_), Some(error)) => {
                                        Err(anyhow::anyhow!("host response was invalid: {error}"))
                                    }
                                    (None, None) => Err(anyhow::anyhow!(
                                        "host response was missing both payload and error"
                                    )),
                                };
                                let _ = pending_request.response_sender.send(result);
                            }
                        }
                        message => {
                            if host_message_sender.send_blocking(message).is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("failed to read plugin host message: {error:#}");
                    break;
                }
            }
        }
    });

    thread::spawn(move || {
        let stdout = std::io::stdout();
        let mut stdout = BufWriter::new(stdout.lock());
        while let Ok(message) = outbound_receiver.recv() {
            if let Err(error) = write_outbound_message(&mut stdout, &message) {
                eprintln!("failed to write plugin message: {error:#}");
                break;
            }
        }
    });

    outbound_sender
        .send(OutboundMessage::Plugin(PluginToHost::Register {
            plugin: app.metadata.clone(),
        }))
        .map_err(|_| anyhow::anyhow!("failed to send plugin registration to host"))?;

    let (tick_sender, tick_receiver) = smol::channel::unbounded::<()>();
    thread::spawn(move || {
        while tick_sender.send_blocking(()).is_ok() {
            thread::sleep(Duration::from_millis(16));
        }
    });

    smol::block_on(async move {
        loop {
            let inbound = host_message_receiver.recv().fuse();
            let tick = tick_receiver.recv().fuse();
            pin_mut!(inbound, tick);

            match select(inbound, tick).await {
                futures::future::Either::Left((message, _)) => {
                    let message = match message {
                        Ok(message) => message,
                        Err(_) => break,
                    };
                    match app.handle_message(message)? {
                        LoopControl::Continue(messages) => {
                            for message in messages {
                                outbound_sender
                                    .send(OutboundMessage::Plugin(message))
                                    .map_err(|_| {
                                        anyhow::anyhow!(
                                            "failed to send plugin response to writer thread"
                                        )
                                    })?;
                            }
                        }
                        LoopControl::Shutdown => break,
                    }
                }
                futures::future::Either::Right((result, _)) => {
                    if result.is_err() {
                        break;
                    }
                }
            }

            for message in app.drain_messages()? {
                outbound_sender
                    .send(OutboundMessage::Plugin(message))
                    .map_err(|_| {
                        anyhow::anyhow!("failed to send plugin message to writer thread")
                    })?;
            }
        }

        Ok(())
    })
}
