use anyhow::Context as _;
use futures::Future;
use futures::future::LocalBoxFuture;
use gpui::StyleRefinement;
pub use refineable::Refineable;
use smol::LocalExecutor;
use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell, UnsafeCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context as TaskContext, Poll},
};

pub use anyhow::Result;
pub use gpui::Styled;
pub use gpui::http_client;
#[cfg(any(test, feature = "test-support"))]
pub use gpui::proptest;
pub use gpui::{
    AbsoluteLength, Action, AlignContent, AlignItems, AlignSelf, Animation, ArcCow, AvailableSpace,
    BackgroundExecutor, BorderStyle, BoxShadow, ClickEvent, CursorStyle, DefiniteLength,
    DispatchPhase, Display, ElementId, Fill, FlexDirection, FlexWrap, Font, FontFeatures,
    FontStyle, FontWeight, ForegroundExecutor, GpuSpecs, GridPlacement, GridTemplate, Hsla,
    JustifyContent, KeyContext, KeyDownEvent, KeyUpEvent, KeybindingKeystroke, KeyboardButton,
    Keystroke, LayoutId, Length, MacroActionBuilder, MacroActionData, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MousePressureEvent,
    MouseUpEvent, NavigationDirection, NoAction, Overflow, PinchEvent, Pixels, Point,
    PressureStage, Priority, Radians, Rems, ScrollAnchor, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, SharedString, Size, StrikethroughStyle, StyledImage, TextAlign, TextOverflow,
    TextStyleRefinement, TouchPhase, Transformation, Unbind, UnderlineStyle, WhiteSpace,
    WindowControlArea, hsla, is_no_action, is_unbind, point, px, radians, relative, rems, size,
};
pub use gpui::{FutureExt, Timeout};
#[cfg(any(target_os = "windows", target_os = "linux", target_family = "wasm"))]
pub use gpui::{PriorityQueueReceiver, PriorityQueueSender};
pub use plugin_protocol::{
    EventHandlerId, HostThemeSnapshot, StyleValue, UiEventKind, UiEventPhase, UiNode, UiNodeEvent,
    UiNodeKind,
};
pub use theme::ActiveTheme;

pub type EntityId = u64;
pub type HandlerId = EventHandlerId;
pub type StyleMap = StyleRefinement;
pub type App = Runtime;
pub type AsyncWindowContext = AsyncApp;
pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type EventDispatch = Rc<dyn Fn(&UiEvent, &mut Window, &mut App)>;
type ObserverCallback = Rc<RefCell<dyn FnMut(&mut Runtime) -> bool>>;
type EventSubscriberCallback = Rc<RefCell<dyn FnMut(&dyn Any, &mut Runtime) -> bool>>;
type ReleaseObserverCallback = Rc<RefCell<dyn FnMut(&mut dyn Any, &mut Runtime) -> bool>>;
type AppRestartCallback = Rc<RefCell<dyn FnMut(&mut Runtime) -> bool>>;
type AppQuitCallback = Rc<RefCell<dyn FnMut(&mut Runtime) -> LocalBoxFuture<'static, ()>>>;
type WindowDeferredCallback = Box<dyn FnOnce(&mut Window, &mut App)>;
type WindowActionListener = Rc<dyn Fn(&dyn Any, DispatchPhase, &mut Window, &mut App)>;
type FrozenGlobal = &'static dyn Any;

#[derive(Clone, Debug)]
pub enum UiEvent {
    Click(ClickEvent),
    AuxClick(ClickEvent),
    MouseDown(MouseDownEvent),
    MouseUp(MouseUpEvent),
    MouseMove(MouseMoveEvent),
    MousePressure(MousePressureEvent),
    Hover(bool),
    KeyDown(KeyDownEvent),
    KeyUp(KeyUpEvent),
    ModifiersChanged(ModifiersChangedEvent),
    ScrollWheel(ScrollWheelEvent),
    Pinch(PinchEvent),
    Action(ActionEvent),
}

#[derive(Default)]
pub struct Window {
    redraw_count: usize,
    focused_handle: Option<FocusHandle>,
    deferred_callbacks: Vec<WindowDeferredCallback>,
    next_frame_callbacks: Vec<WindowDeferredCallback>,
    action_listeners: Vec<(TypeId, WindowActionListener)>,
}

impl Window {
    pub fn redraw_count(&self) -> usize {
        self.redraw_count
    }

    pub fn request_redraw(&mut self) {
        self.redraw_count += 1;
    }

    pub fn window_handle(&self) -> AnyWindowHandle {
        AnyWindowHandle::default()
    }

    pub fn replace_root<V>(
        &mut self,
        app: &mut App,
        build_view: impl FnOnce(&mut Window, &mut Context<'_, V>) -> V,
    ) -> Entity<V>
    where
        V: 'static + Render,
    {
        AppContext::new(app, |cx| build_view(self, cx))
    }

    pub fn focus(&mut self, focus_handle: &FocusHandle, _app: &mut App) {
        self.focused_handle = Some(*focus_handle);
    }

    pub fn on_next_frame(
        &mut self,
        f: impl FnOnce(&mut Window, &mut App) + 'static,
        _app: &mut App,
    ) {
        self.next_frame_callbacks.push(Box::new(f));
    }

    pub fn defer(&mut self, _app: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        self.deferred_callbacks.push(Box::new(f));
    }

    pub fn on_action(
        &mut self,
        action_type: TypeId,
        listener: impl Fn(&dyn Any, DispatchPhase, &mut Window, &mut App) + 'static,
    ) {
        self.action_listeners.push((action_type, Rc::new(listener)));
    }

    pub fn dispatch_action(&mut self, action: Box<dyn Action>, cx: &mut App) {
        let action_type = action.as_any().type_id();
        let listeners = Iterator::map(
            self.action_listeners
                .iter()
                .filter(|(listener_type, _)| *listener_type == action_type),
            |(_, listener)| listener.clone(),
        )
        .collect::<Vec<_>>();

        for listener in &listeners {
            listener(action.as_any(), DispatchPhase::Capture, self, cx);
        }
        for listener in listeners {
            listener(action.as_any(), DispatchPhase::Bubble, self, cx);
        }

        self.flush_queued_callbacks(cx);
    }

    pub fn to_async(&self, cx: &App) -> AsyncWindowContext {
        cx.to_async()
    }

    pub fn spawn<AsyncFn, R>(&self, cx: &App, f: AsyncFn) -> Task<R>
    where
        R: 'static,
        AsyncFn: AsyncFnOnce(&mut AsyncWindowContext) -> R + 'static,
    {
        let mut async_window = cx.to_async();
        cx.spawn_task(async move { f(&mut async_window).await })
    }

    pub fn spawn_with_priority<AsyncFn, R>(
        &self,
        _priority: Priority,
        cx: &App,
        f: AsyncFn,
    ) -> Task<R>
    where
        R: 'static,
        AsyncFn: AsyncFnOnce(&mut AsyncWindowContext) -> R + 'static,
    {
        self.spawn(cx, f)
    }

    fn flush_queued_callbacks(&mut self, cx: &mut App) {
        while let Some(callback) = self.deferred_callbacks.pop() {
            callback(self, cx);
        }
        while let Some(callback) = self.next_frame_callbacks.pop() {
            callback(self, cx);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Empty;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FocusHandle;

impl FocusHandle {
    pub fn dispatch_action(&self, action: &dyn Action, window: &mut Window, cx: &mut App) {
        window.dispatch_action(action.boxed_clone(), cx);
    }
}

pub trait Focusable: 'static {
    fn focus_handle(_entity: &Entity<Self>, _cx: &App) -> FocusHandle
    where
        Self: Sized,
    {
        FocusHandle
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

impl WindowId {
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<u64> for WindowId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AnyWindowHandle {
    id: WindowId,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct WindowHandle<T> {
    handle: AnyWindowHandle,
    window_type: PhantomData<fn() -> T>,
}

impl<T> Copy for WindowHandle<T> {}

impl<T> Clone for WindowHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Default for WindowHandle<T> {
    fn default() -> Self {
        Self {
            handle: AnyWindowHandle::default(),
            window_type: PhantomData,
        }
    }
}

impl AnyWindowHandle {
    pub fn window_id(&self) -> WindowId {
        self.id
    }

    pub fn downcast<T: 'static>(&self) -> Option<WindowHandle<T>> {
        Some(WindowHandle {
            handle: *self,
            window_type: PhantomData,
        })
    }

    pub fn update<C, R>(
        self,
        cx: &mut C,
        update: impl FnOnce(AnyView, &mut Window, &mut App) -> R,
    ) -> Result<R>
    where
        C: AppContext,
    {
        cx.update_window(self, update)
    }

    pub fn read<T, C, R>(self, cx: &C, read: impl FnOnce(Entity<T>, &App) -> R) -> Result<R>
    where
        C: AppContext,
        T: 'static,
    {
        let view = self
            .downcast::<T>()
            .context("the type of the window's root view has changed")?;
        cx.read_window(&view, read)
    }
}

impl<T: 'static> WindowHandle<T> {
    pub fn new(id: WindowId) -> Self {
        Self {
            handle: AnyWindowHandle { id },
            window_type: PhantomData,
        }
    }

    pub fn update<C, R>(
        &self,
        cx: &mut C,
        _update: impl FnOnce(&mut T, &mut Window, &mut Context<'_, T>) -> R,
    ) -> Result<R>
    where
        C: AppContext,
    {
        self.handle.update(cx, |_root, _window, _cx| {
            anyhow::bail!("window root handling is not supported in gpui_plugin")
        })?
    }

    pub fn read<'a>(&self, _cx: &'a App) -> Result<&'a T> {
        anyhow::bail!("window root handling is not supported in gpui_plugin")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnyView {
    entity_id: Option<EntityId>,
}

impl<T: 'static> From<Entity<T>> for AnyView {
    fn from(value: Entity<T>) -> Self {
        Self {
            entity_id: Some(value.entity_id()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionEvent {
    pub name: String,
    pub payload: Option<serde_json::Value>,
}

impl Action for ActionEvent {
    fn boxed_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }

    fn partial_eq(&self, action: &dyn Action) -> bool {
        action
            .as_any()
            .downcast_ref::<Self>()
            .map_or(false, |other| self == other)
    }

    fn name(&self) -> &'static str {
        Box::leak(self.name.clone().into_boxed_str())
    }

    fn name_for_type() -> &'static str
    where
        Self: Sized,
    {
        "plugin::ActionEvent"
    }

    fn build(value: serde_json::Value) -> Result<Box<dyn Action>>
    where
        Self: Sized,
    {
        Ok(Box::new(Self {
            name: value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            payload: value.get("payload").cloned(),
        }))
    }

    fn action_json_schema(
        _generator: &mut gpui::private::schemars::SchemaGenerator,
    ) -> Option<gpui::private::schemars::Schema> {
        None
    }
}

#[derive(Clone)]
pub struct GroupStyle {
    pub group: SharedString,
    pub style: StyleRefinement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventPhase {
    Capture,
    #[default]
    Bubble,
}

#[derive(Clone)]
pub struct PendingEventHandler {
    kind: UiEventKind,
    phase: EventPhase,
    mouse_button: Option<String>,
    action_name: Option<String>,
    dispatch: EventDispatch,
}

#[derive(Clone, Default)]
pub struct Interactivity {
    pub group: Option<SharedString>,
    pub element_id: Option<ElementId>,
    pub tracked_focus_handle: Option<FocusHandle>,
    pub tab_stop: bool,
    pub tab_index: Option<isize>,
    pub tab_group: bool,
    pub focusable: bool,
    pub key_context: bool,
    pub window_control_area: Option<WindowControlArea>,
    pub tracked_scroll_handle: Option<ScrollHandle>,
    pub scroll_anchor: Option<ScrollAnchor>,
    pub hover_style: Option<StyleRefinement>,
    pub group_hover_style: Option<GroupStyle>,
    pub active_style: Option<StyleRefinement>,
    pub group_active_style: Option<GroupStyle>,
    pub focus_style: Option<StyleRefinement>,
    pub in_focus_style: Option<StyleRefinement>,
    pub focus_visible_style: Option<StyleRefinement>,
    pub base_style: StyleRefinement,
    pub occlude: bool,
    pub block_mouse_except_scroll: bool,
    handlers: Vec<PendingEventHandler>,
}

impl Interactivity {
    pub fn push_handler(
        &mut self,
        kind: UiEventKind,
        phase: EventPhase,
        mouse_button: Option<MouseButton>,
        action_name: Option<String>,
        dispatch: EventDispatch,
    ) {
        self.handlers.push(PendingEventHandler {
            kind,
            phase,
            mouse_button: mouse_button.map(mouse_button_name),
            action_name,
            dispatch,
        });
    }

    pub fn handlers(&self) -> &[PendingEventHandler] {
        &self.handlers
    }
}

fn mouse_button_name(button: MouseButton) -> String {
    match button {
        MouseButton::Left => "left".to_string(),
        MouseButton::Right => "right".to_string(),
        MouseButton::Middle => "middle".to_string(),
        MouseButton::Navigate(NavigationDirection::Back) => "back".to_string(),
        MouseButton::Navigate(NavigationDirection::Forward) => "forward".to_string(),
    }
}

pub struct Stateful<E> {
    element: E,
}

impl<E> Stateful<E> {
    pub fn new(element: E) -> Self {
        Self { element }
    }
}

impl<E: Element> Element for Stateful<E> {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        self.element.into_node(context)
    }
}

impl<E: ParentElement> ParentElement for Stateful<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

impl<E: Styled> Styled for Stateful<E> {
    fn style(&mut self) -> &mut StyleMap {
        self.element.style()
    }
}

pub trait Global: 'static {}

pub trait EventEmitter<Evt>: 'static {}

pub trait ReadGlobal {
    fn global(cx: &App) -> &Self;
}

pub trait UpdateGlobal {
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R,
        Self: Sized;

    fn set_global<C>(cx: &mut C, global: Self)
    where
        C: BorrowAppContext,
        Self: Sized;
}

impl<T: Global> UpdateGlobal for T {
    fn update_global<C, F, R>(cx: &mut C, update: F) -> R
    where
        C: BorrowAppContext,
        F: FnOnce(&mut Self, &mut C) -> R,
    {
        cx.update_global(update)
    }

    fn set_global<C>(cx: &mut C, global: Self)
    where
        C: BorrowAppContext,
    {
        cx.set_global(global)
    }
}

impl<T: Global> ReadGlobal for T {
    fn global(cx: &App) -> &Self {
        cx.global::<T>()
    }
}

enum SubscriptionKind {
    Observe {
        entity_id: EntityId,
    },
    Release {
        entity_id: EntityId,
    },
    Event {
        entity_id: EntityId,
        event_type: TypeId,
    },
    Global {
        global_type: TypeId,
    },
    AppRestart,
    AppQuit,
}

pub struct Subscription {
    runtime: Weak<RefCell<RuntimeState>>,
    subscription_id: usize,
    kind: SubscriptionKind,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let mut state = runtime.borrow_mut();
        match self.kind {
            SubscriptionKind::Observe { entity_id } => {
                if let Some(observers) = state.observers.get_mut(&entity_id) {
                    observers.remove(&self.subscription_id);
                    if observers.is_empty() {
                        state.observers.remove(&entity_id);
                    }
                }
            }
            SubscriptionKind::Release { entity_id } => {
                if let Some(observers) = state.release_observers.get_mut(&entity_id) {
                    observers.remove(&self.subscription_id);
                    if observers.is_empty() {
                        state.release_observers.remove(&entity_id);
                    }
                }
            }
            SubscriptionKind::Event {
                entity_id,
                event_type,
            } => {
                if let Some(subscribers) = state.event_subscribers.get_mut(&(entity_id, event_type))
                {
                    subscribers.remove(&self.subscription_id);
                    if subscribers.is_empty() {
                        state.event_subscribers.remove(&(entity_id, event_type));
                    }
                }
            }
            SubscriptionKind::Global { global_type } => {
                if let Some(observers) = state.global_observers.get_mut(&global_type) {
                    observers.remove(&self.subscription_id);
                    if observers.is_empty() {
                        state.global_observers.remove(&global_type);
                    }
                }
            }
            SubscriptionKind::AppRestart => {
                state.app_restart_observers.remove(&self.subscription_id);
            }
            SubscriptionKind::AppQuit => {
                state.app_quit_observers.remove(&self.subscription_id);
            }
        }
    }
}

pub struct Reservation<T> {
    entity_id: EntityId,
    reservation_type: PhantomData<fn() -> T>,
}

impl<T> Reservation<T> {
    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }
}

pub struct GpuiBorrow<'a, T: 'static> {
    borrow: EntityWriteGuard<T>,
    borrow_lifetime: PhantomData<&'a mut T>,
}

impl<T: 'static> Deref for GpuiBorrow<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.borrow
    }
}

impl<T: 'static> DerefMut for GpuiBorrow<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.borrow
    }
}

impl<T: 'static> Drop for GpuiBorrow<'_, T> {
    fn drop(&mut self) {}
}

struct RuntimeState {
    next_entity_id: EntityId,
    next_generation: u64,
    next_subscription_id: usize,
    dirty_entities: BTreeSet<EntityId>,
    executor: Rc<LocalExecutor<'static>>,
    errors: Vec<String>,
    globals: HashMap<TypeId, Box<dyn Any>>,
    frozen_globals: HashMap<TypeId, FrozenGlobal>,
    observers: HashMap<EntityId, HashMap<usize, ObserverCallback>>,
    release_observers: HashMap<EntityId, HashMap<usize, ReleaseObserverCallback>>,
    event_subscribers: HashMap<(EntityId, TypeId), HashMap<usize, EventSubscriberCallback>>,
    global_observers: HashMap<TypeId, HashMap<usize, ObserverCallback>>,
    app_restart_observers: HashMap<usize, AppRestartCallback>,
    app_quit_observers: HashMap<usize, AppQuitCallback>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            next_entity_id: 1,
            next_generation: 1,
            next_subscription_id: 1,
            dirty_entities: BTreeSet::new(),
            executor: Rc::new(LocalExecutor::new()),
            errors: Vec::new(),
            globals: HashMap::new(),
            frozen_globals: HashMap::new(),
            observers: HashMap::new(),
            release_observers: HashMap::new(),
            event_subscribers: HashMap::new(),
            global_observers: HashMap::new(),
            app_restart_observers: HashMap::new(),
            app_quit_observers: HashMap::new(),
        }
    }
}

#[derive(Clone)]
pub struct Runtime {
    state: Rc<RefCell<RuntimeState>>,
    theme_registry: Arc<theme::ThemeRegistry>,
    theme: Arc<theme::Theme>,
}

impl Default for Runtime {
    fn default() -> Self {
        let theme_registry = Arc::new(theme::ThemeRegistry::new(Box::new(())));
        let theme = theme_registry
            .get(theme::DEFAULT_DARK_THEME)
            .expect("default theme must be available");
        Self {
            state: Rc::new(RefCell::new(RuntimeState::default())),
            theme_registry,
            theme,
        }
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

pub trait RuntimeAccess {
    fn runtime(&self) -> Runtime;
}

impl RuntimeAccess for Runtime {
    fn runtime(&self) -> Runtime {
        self.clone()
    }
}

impl ActiveTheme for Runtime {
    fn theme(&self) -> &Arc<theme::Theme> {
        &self.theme
    }
}

#[derive(Clone, Debug)]
pub struct AsyncApp {
    runtime: Runtime,
}

impl RuntimeAccess for AsyncApp {
    fn runtime(&self) -> Runtime {
        self.runtime.clone()
    }
}

impl AsyncApp {
    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        let mut async_app = self.clone();
        self.runtime
            .spawn_task(async move { f(&mut async_app).await })
    }

    pub fn spawn_with_priority<AsyncFn, R>(&self, _priority: Priority, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        self.spawn(f)
    }

    pub fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.runtime.background_spawn(future)
    }
}

impl ActiveTheme for AsyncApp {
    fn theme(&self) -> &Arc<theme::Theme> {
        self.runtime.theme()
    }
}

#[must_use]
pub struct Task<R> {
    state: TaskState<R>,
}

enum TaskState<R> {
    Ready(Option<R>),
    Running(smol::Task<R>),
}

impl<R> fmt::Debug for Task<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Task")
            .field("is_ready", &self.is_ready())
            .finish_non_exhaustive()
    }
}

impl<R> Task<R> {
    pub fn ready(value: R) -> Self
    where
        R: 'static,
    {
        Self {
            state: TaskState::Ready(Some(value)),
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state, TaskState::Ready(_))
    }

    pub fn detach(self) {
        if let TaskState::Running(task) = self.state {
            task.detach();
        }
    }
}

impl<R> Future for Task<R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match &mut this.state {
            TaskState::Ready(value) => Poll::Ready(value.take().expect("ready task polled once")),
            TaskState::Running(task) => match Future::poll(Pin::new(task), cx) {
                Poll::Ready(value) => {
                    this.state = TaskState::Ready(None);
                    Poll::Ready(value)
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl<R> From<smol::Task<R>> for Task<R> {
    fn from(task: smol::Task<R>) -> Self {
        Self {
            state: TaskState::Running(task),
        }
    }
}

impl<R> Unpin for Task<R> {}

impl<R> Task<R> {
    fn running(task: smol::Task<R>) -> Self {
        Self::from(task)
    }
}

pub struct Deferred<F: FnOnce()> {
    callback: Option<F>,
}

impl<F: FnOnce()> Deferred<F> {
    pub fn new(callback: F) -> Self {
        Self {
            callback: Some(callback),
        }
    }
}

impl<F: FnOnce()> Drop for Deferred<F> {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback();
        }
    }
}

struct EntityValue<T: 'static> {
    entity_id: EntityId,
    runtime: Weak<RefCell<RuntimeState>>,
    theme_registry: Arc<theme::ThemeRegistry>,
    theme: Arc<theme::Theme>,
    active_readers: Cell<usize>,
    active_writer: Cell<bool>,
    value: UnsafeCell<T>,
}

impl<T: 'static> Drop for EntityValue<T> {
    fn drop(&mut self) {
        let Some(state) = self.runtime.upgrade() else {
            return;
        };

        let callbacks = state
            .borrow()
            .release_observers
            .get(&self.entity_id)
            .map(|callbacks| callbacks.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        if !callbacks.is_empty() {
            let mut runtime = Runtime {
                state: state.clone(),
                theme_registry: self.theme_registry.clone(),
                theme: self.theme.clone(),
            };

            for callback in callbacks {
                callback.borrow_mut()(self.get_mut(), &mut runtime);
            }
        }

        let mut state = state.borrow_mut();
        state.dirty_entities.remove(&self.entity_id);
        state.observers.remove(&self.entity_id);
        state.release_observers.remove(&self.entity_id);
        state
            .event_subscribers
            .retain(|(entity_id, _), _| *entity_id != self.entity_id);
    }
}

impl<T: 'static> EntityValue<T> {
    fn borrow(value: &Rc<Self>) -> EntityReadGuard<T> {
        if value.active_writer.get() {
            panic!("entity already mutably borrowed");
        }
        value
            .active_readers
            .set(value.active_readers.get().saturating_add(1));
        EntityReadGuard {
            value: value.clone(),
        }
    }

    fn borrow_mut(value: &Rc<Self>) -> EntityWriteGuard<T> {
        if value.active_writer.get() || value.active_readers.get() > 0 {
            panic!("entity already borrowed");
        }
        value.active_writer.set(true);
        EntityWriteGuard {
            value: value.clone(),
        }
    }

    fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

struct EntityReadGuard<T: 'static> {
    value: Rc<EntityValue<T>>,
}

impl<T: 'static> Deref for EntityReadGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.value.value.get() }
    }
}

impl<T: 'static> Drop for EntityReadGuard<T> {
    fn drop(&mut self) {
        self.value
            .active_readers
            .set(self.value.active_readers.get().saturating_sub(1));
    }
}

struct EntityWriteGuard<T: 'static> {
    value: Rc<EntityValue<T>>,
}

impl<T: 'static> Deref for EntityWriteGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.value.value.get() }
    }
}

impl<T: 'static> DerefMut for EntityWriteGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.value.value.get() }
    }
}

impl<T: 'static> Drop for EntityWriteGuard<T> {
    fn drop(&mut self) {
        self.value.active_writer.set(false);
    }
}

pub struct Entity<T: 'static> {
    entity_id: EntityId,
    value: Rc<EntityValue<T>>,
    runtime: Weak<RefCell<RuntimeState>>,
    entity_type: PhantomData<fn(T) -> T>,
}

impl<T: 'static> fmt::Debug for Entity<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entity")
            .field("entity_id", &self.entity_id)
            .finish()
    }
}

pub struct WeakEntity<T: 'static> {
    entity_id: EntityId,
    value: Weak<EntityValue<T>>,
    runtime: Weak<RefCell<RuntimeState>>,
    entity_type: PhantomData<fn(T) -> T>,
}

impl<T: 'static> fmt::Debug for WeakEntity<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakEntity")
            .field("entity_id", &self.entity_id)
            .finish()
    }
}

impl<T: 'static> Clone for Entity<T> {
    fn clone(&self) -> Self {
        Self {
            entity_id: self.entity_id,
            value: self.value.clone(),
            runtime: self.runtime.clone(),
            entity_type: PhantomData,
        }
    }
}

impl<T: 'static> Clone for WeakEntity<T> {
    fn clone(&self) -> Self {
        Self {
            entity_id: self.entity_id,
            value: self.value.clone(),
            runtime: self.runtime.clone(),
            entity_type: PhantomData,
        }
    }
}

impl<T: 'static> Entity<T> {
    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    pub fn downgrade(&self) -> WeakEntity<T> {
        WeakEntity {
            entity_id: self.entity_id,
            value: Rc::downgrade(&self.value),
            runtime: self.runtime.clone(),
            entity_type: PhantomData,
        }
    }

    pub fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let value = EntityValue::borrow(&self.value);
        read(&value)
    }

    pub fn read_with<C, R>(&self, cx: &C, read: impl FnOnce(&T, &Runtime) -> R) -> R
    where
        C: RuntimeAccess,
    {
        let runtime = cx.runtime();
        let value = EntityValue::borrow(&self.value);
        read(&value, &runtime)
    }

    pub fn update<C, R>(&self, cx: &mut C, update: impl FnOnce(&mut T, &mut Context<T>) -> R) -> R
    where
        C: RuntimeAccess,
    {
        let runtime = cx.runtime();
        let mut entity = EntityValue::borrow_mut(&self.value);
        let mut entity_context = Context::new(runtime, self.downgrade());
        update(&mut entity, &mut entity_context)
    }
}

impl<T: 'static> WeakEntity<T> {
    pub fn entity_id(&self) -> EntityId {
        self.entity_id
    }

    pub fn upgrade(&self) -> Option<Entity<T>> {
        Some(Entity {
            entity_id: self.entity_id,
            value: self.value.upgrade()?,
            runtime: self.runtime.clone(),
            entity_type: PhantomData,
        })
    }

    pub fn update<C, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> Result<R>
    where
        C: RuntimeAccess,
    {
        let entity = self.upgrade().context("entity released")?;
        Ok(entity.update(cx, update))
    }

    pub fn read_with<C, R>(&self, cx: &C, read: impl FnOnce(&T, &Runtime) -> R) -> Result<R>
    where
        C: RuntimeAccess,
    {
        let entity = self.upgrade().context("entity released")?;
        Ok(entity.read_with(cx, read))
    }
}

pub struct Context<'a, T: 'static> {
    runtime: Runtime,
    entity: WeakEntity<T>,
    context_lifetime: PhantomData<&'a ()>,
}

impl<'a, T> Context<'a, T> {
    fn new(runtime: Runtime, entity: WeakEntity<T>) -> Self {
        Self {
            runtime,
            entity,
            context_lifetime: PhantomData,
        }
    }
}

impl<T> Deref for Context<'_, T> {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

impl<T> DerefMut for Context<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
}

impl<T> ActiveTheme for Context<'_, T> {
    fn theme(&self) -> &Arc<theme::Theme> {
        self.runtime.theme()
    }
}

impl<T: 'static> Context<'_, T> {
    pub fn entity_id(&self) -> EntityId {
        self.entity.entity_id()
    }

    pub fn entity(&self) -> Entity<T> {
        self.entity
            .upgrade()
            .expect("entity must be alive while its context exists")
    }

    pub fn weak_entity(&self) -> WeakEntity<T> {
        self.entity.clone()
    }

    pub fn notify(&mut self) {
        self.runtime.notify_entity(self.entity_id());
    }

    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(WeakEntity<T>, &mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        let weak_entity = self.weak_entity();
        let mut async_app = self.runtime.to_async();
        self.runtime
            .spawn_task(async move { f(weak_entity, &mut async_app).await })
    }

    pub fn listener<E: 'static>(
        &self,
        f: impl Fn(&mut T, &E, &mut Window, &mut Context<T>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let entity = self.weak_entity();
        move |event, window, runtime| {
            if let Err(error) = entity.update(runtime, |this, cx| f(this, event, window, cx)) {
                runtime.record_error(error);
            }
        }
    }

    pub fn processor<E: 'static, R: 'static>(
        &self,
        f: impl Fn(&mut T, E, &mut Window, &mut Context<T>) -> R + 'static,
    ) -> impl Fn(E, &mut Window, &mut App) -> R + 'static {
        let entity = self.entity();
        move |event, window, runtime| entity.update(runtime, |this, cx| f(this, event, window, cx))
    }

    pub fn on_drop(
        &self,
        f: impl FnOnce(&mut T, &mut Context<T>) + 'static,
    ) -> Deferred<impl FnOnce()> {
        let entity = self.weak_entity();
        let mut runtime = self.runtime.clone();
        Deferred::new(move || {
            if let Err(error) = entity.update(&mut runtime, f) {
                runtime.record_error(error);
            }
        })
    }

    pub fn observe<W>(
        &mut self,
        entity: &Entity<W>,
        mut on_notify: impl FnMut(&mut T, Entity<W>, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        W: 'static,
    {
        let observer = self.weak_entity();
        let observed = entity.downgrade();
        self.runtime
            .observe_entity(entity.entity_id(), move |runtime| {
                if let Some((observer, observed)) = observer.upgrade().zip(observed.upgrade()) {
                    observer.update(runtime, |this, cx| on_notify(this, observed, cx));
                    true
                } else {
                    false
                }
            })
    }

    pub fn observe_release<W>(
        &self,
        entity: &Entity<W>,
        on_release: impl FnOnce(&mut T, &mut W, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
        W: 'static,
    {
        let observer = self.weak_entity();
        let mut on_release = Some(on_release);
        self.runtime
            .observe_entity_release(entity.entity_id(), move |released, runtime| {
                let Some(observer) = observer.upgrade() else {
                    return false;
                };
                let Some(on_release) = on_release.take() else {
                    return false;
                };
                let Some(released) = released.downcast_mut::<W>() else {
                    runtime.record_error(anyhow::anyhow!(
                        "release observer registered with incorrect entity type"
                    ));
                    return false;
                };
                observer.update(runtime, |this, cx| on_release(this, released, cx));
                false
            })
    }

    pub fn observe_self(
        &mut self,
        mut on_notify: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription {
        let entity = self.entity();
        self.observe(&entity, move |this, _, cx| on_notify(this, cx))
    }

    pub fn subscribe<W, Evt>(
        &mut self,
        entity: &Entity<W>,
        mut on_event: impl FnMut(&mut T, Entity<W>, &Evt, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        W: 'static + EventEmitter<Evt>,
        Evt: 'static,
    {
        let subscriber = self.weak_entity();
        let emitter = entity.downgrade();
        self.runtime.subscribe_to_event(
            entity.entity_id(),
            TypeId::of::<Evt>(),
            move |event, runtime| {
                if let Some((subscriber, emitter)) = subscriber.upgrade().zip(emitter.upgrade()) {
                    let event = event
                        .downcast_ref::<Evt>()
                        .expect("event subscriber registered with incorrect event type");
                    subscriber.update(runtime, |this, cx| on_event(this, emitter, event, cx));
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn subscribe_self<Evt>(
        &mut self,
        mut on_event: impl FnMut(&mut T, &Evt, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: EventEmitter<Evt>,
        Evt: 'static,
    {
        let entity = self.entity();
        self.subscribe(&entity, move |this, _, event, cx| on_event(this, event, cx))
    }

    pub fn observe_global<G: Global>(
        &mut self,
        mut on_notify: impl FnMut(&mut T, &mut Context<T>) + 'static,
    ) -> Subscription {
        let observer = self.weak_entity();
        self.runtime
            .observe_global_type(TypeId::of::<G>(), move |runtime| {
                if let Some(observer) = observer.upgrade() {
                    observer.update(runtime, |this, cx| on_notify(this, cx));
                    true
                } else {
                    false
                }
            })
    }

    pub fn on_app_restart(
        &self,
        mut on_restart: impl FnMut(&mut T, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let observer = self.weak_entity();
        self.runtime.observe_app_restart(move |runtime| {
            observer
                .update(runtime, |this, cx| {
                    on_restart(this, cx);
                })
                .is_ok()
        })
    }

    pub fn on_app_quit<Fut>(
        &self,
        mut on_quit: impl FnMut(&mut T, &mut Context<T>) -> Fut + 'static,
    ) -> Subscription
    where
        Fut: Future<Output = ()> + 'static,
        T: 'static,
    {
        let observer = self.weak_entity();
        self.runtime.observe_app_quit(move |runtime| {
            let future = observer.update(runtime, |this, cx| on_quit(this, cx)).ok();
            Box::pin(async move {
                if let Some(future) = future {
                    future.await;
                }
            })
        })
    }

    pub fn emit<Evt: 'static>(&mut self, event: Evt)
    where
        T: EventEmitter<Evt>,
    {
        self.runtime.emit_event(self.entity_id(), &event);
    }

    pub fn focus_view<W: Focusable>(&mut self, view: &Entity<W>, window: &mut Window) {
        window.focus(&Focusable::focus_handle(view, self), self);
    }

    pub fn on_next_frame(
        &self,
        window: &mut Window,
        f: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) {
        let entity = self.entity();
        let mut runtime = self.runtime.clone();
        window.on_next_frame(
            move |window, runtime| {
                entity.update(runtime, |this, cx| f(this, window, cx));
            },
            &mut runtime,
        );
    }

    pub fn defer_in(
        &mut self,
        window: &mut Window,
        f: impl FnOnce(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) {
        let entity = self.entity();
        window.defer(&mut self.runtime, move |window, runtime| {
            entity.update(runtime, |this, cx| f(this, window, cx));
        });
    }

    pub fn spawn_in<AsyncFn, R>(&self, _window: &Window, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(WeakEntity<T>, &mut AsyncWindowContext) -> R + 'static,
        R: 'static,
    {
        let weak_entity = self.weak_entity();
        let mut async_window = self.runtime.to_async();
        self.runtime
            .spawn_task(async move { f(weak_entity, &mut async_window).await })
    }

    pub fn spawn_in_with_priority<AsyncFn, R>(
        &self,
        _priority: Priority,
        window: &Window,
        f: AsyncFn,
    ) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(WeakEntity<T>, &mut AsyncWindowContext) -> R + 'static,
        R: 'static,
    {
        self.spawn_in(window, f)
    }

    pub fn observe_in<W>(
        &mut self,
        entity: &Entity<W>,
        _window: &mut Window,
        mut on_notify: impl FnMut(&mut T, Entity<W>, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        W: 'static,
    {
        let observer = self.weak_entity();
        let observed = entity.downgrade();
        self.runtime
            .observe_entity(entity.entity_id(), move |runtime| {
                if let Some((observer, observed)) = observer.upgrade().zip(observed.upgrade()) {
                    let mut window = Window::default();
                    observer.update(runtime, |this, cx| {
                        on_notify(this, observed, &mut window, cx)
                    });
                    true
                } else {
                    false
                }
            })
    }

    pub fn subscribe_in<W, Evt>(
        &mut self,
        entity: &Entity<W>,
        _window: &Window,
        mut on_event: impl FnMut(&mut T, &Entity<W>, &Evt, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        W: 'static + EventEmitter<Evt>,
        Evt: 'static,
    {
        let subscriber = self.weak_entity();
        let emitter = entity.downgrade();
        self.runtime.subscribe_to_event(
            entity.entity_id(),
            TypeId::of::<Evt>(),
            move |event, runtime| {
                if let Some((subscriber, emitter)) = subscriber.upgrade().zip(emitter.upgrade()) {
                    let event = event
                        .downcast_ref::<Evt>()
                        .expect("event subscriber registered with incorrect event type");
                    let mut window = Window::default();
                    subscriber.update(runtime, |this, cx| {
                        on_event(this, &emitter, event, &mut window, cx);
                    });
                    true
                } else {
                    false
                }
            },
        )
    }

    pub fn observe_global_in<G: Global>(
        &mut self,
        _window: &Window,
        mut on_notify: impl FnMut(&mut T, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription {
        let observer = self.weak_entity();
        self.runtime
            .observe_global_type(TypeId::of::<G>(), move |runtime| {
                if let Some(observer) = observer.upgrade() {
                    let mut window = Window::default();
                    observer.update(runtime, |this, cx| on_notify(this, &mut window, cx));
                    true
                } else {
                    false
                }
            })
    }

    pub fn on_release_in(
        &mut self,
        _window: &Window,
        on_release: impl FnOnce(&mut T, &mut Window, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let entity_id = self.entity_id();
        let mut on_release = Some(on_release);
        self.runtime
            .observe_entity_release(entity_id, move |released, runtime| {
                let Some(on_release) = on_release.take() else {
                    return false;
                };
                let Some(released) = released.downcast_mut::<T>() else {
                    runtime.record_error(anyhow::anyhow!(
                        "release callback registered with incorrect entity type"
                    ));
                    return false;
                };
                let mut window = Window::default();
                on_release(released, &mut window, runtime);
                false
            })
    }

    pub fn observe_release_in<W>(
        &self,
        observed: &Entity<W>,
        _window: &Window,
        mut on_release: impl FnMut(&mut T, &mut W, &mut Window, &mut Context<T>) + 'static,
    ) -> Subscription
    where
        T: 'static,
        W: 'static,
    {
        let observer = self.weak_entity();
        self.runtime
            .observe_entity_release(observed.entity_id(), move |released, runtime| {
                let Some(observer) = observer.upgrade() else {
                    return false;
                };
                let Some(released) = released.downcast_mut::<W>() else {
                    runtime.record_error(anyhow::anyhow!(
                        "release observer registered with incorrect entity type"
                    ));
                    return false;
                };
                let mut window = Window::default();
                observer.update(runtime, |this, cx| {
                    on_release(this, released, &mut window, cx);
                });
                false
            })
    }

    pub fn on_action(
        &mut self,
        action_type: TypeId,
        window: &mut Window,
        listener: impl Fn(&mut T, &dyn Any, DispatchPhase, &mut Window, &mut Context<T>) + 'static,
    ) {
        let entity = self.weak_entity();
        window.on_action(action_type, move |action, phase, window, runtime| {
            if let Err(error) = entity.update(runtime, |this, cx| {
                listener(this, action, phase, window, cx);
            }) {
                runtime.record_error(error);
            }
        });
    }

    pub fn focus_self(&mut self, window: &mut Window)
    where
        T: Focusable,
    {
        let entity = self.entity();
        window.focus(&Focusable::focus_handle(&entity, self), self);
    }
}

pub trait AppContext {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<'_, T>) -> T)
    -> Entity<T>;

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T>;

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<'_, T>) -> T,
    ) -> Entity<T>;

    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<'_, T>) -> R,
    ) -> R
    where
        T: 'static;

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static;

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static;

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T;

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static;

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static;

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global;
}

pub trait VisualContext: AppContext {
    type Result<T>;

    fn window_handle(&self) -> AnyWindowHandle;

    fn update_window_entity<T: 'static, R>(
        &mut self,
        entity: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Window, &mut Context<'_, T>) -> R,
    ) -> Self::Result<R>;

    fn new_window_entity<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Window, &mut Context<'_, T>) -> T,
    ) -> Self::Result<Entity<T>>;

    fn replace_root_view<V>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<'_, V>) -> V,
    ) -> Self::Result<Entity<V>>
    where
        V: 'static + Render;

    fn focus<V>(&mut self, entity: &Entity<V>) -> Self::Result<()>
    where
        V: Focusable;
}

pub trait BorrowAppContext {
    fn set_global<T: Global>(&mut self, global: T);

    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global;

    fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default;
}

pub trait FluentBuilder {
    fn map<U>(self, f: impl FnOnce(Self) -> U) -> U
    where
        Self: Sized,
    {
        f(self)
    }

    fn when(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized,
    {
        self.map(|this| if condition { then(this) } else { this })
    }

    fn when_else(
        self,
        condition: bool,
        then: impl FnOnce(Self) -> Self,
        else_fn: impl FnOnce(Self) -> Self,
    ) -> Self
    where
        Self: Sized,
    {
        self.map(|this| if condition { then(this) } else { else_fn(this) })
    }

    fn when_some<TValue>(
        self,
        option: Option<TValue>,
        then: impl FnOnce(Self, TValue) -> Self,
    ) -> Self
    where
        Self: Sized,
    {
        self.map(|this| {
            if let Some(value) = option {
                then(this, value)
            } else {
                this
            }
        })
    }

    fn when_none<TValue>(self, option: &Option<TValue>, then: impl FnOnce(Self) -> Self) -> Self
    where
        Self: Sized,
    {
        self.map(|this| if option.is_some() { this } else { then(this) })
    }
}

impl<T> FluentBuilder for T {}

pub trait Element: 'static {
    fn into_node(self, context: &mut RenderContext) -> UiNode;

    fn into_any_element(self) -> AnyElement
    where
        Self: Sized,
    {
        AnyElement::new(self)
    }
}

trait ElementNode {
    fn into_node(self: Box<Self>, context: &mut RenderContext) -> UiNode;
}

impl<T: Element> ElementNode for T {
    fn into_node(self: Box<Self>, context: &mut RenderContext) -> UiNode {
        (*self).into_node(context)
    }
}

pub struct AnyElement(Box<dyn ElementNode>);

impl AnyElement {
    pub fn new(element: impl Element) -> Self {
        Self(Box::new(element))
    }

    pub fn into_node(self, context: &mut RenderContext) -> UiNode {
        self.0.into_node(context)
    }
}

impl Element for Empty {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        UiNode::new(UiNodeKind::Empty)
    }
}

impl Element for String {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        UiNode::text(self)
    }
}

impl Element for SharedString {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        UiNode::text(self.to_string())
    }
}

impl Element for &'static str {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        UiNode::text(self)
    }
}

pub trait IntoElement: Sized {
    type Element: Element;

    fn into_element(self) -> Self::Element;

    fn into_any_element(self) -> AnyElement {
        Element::into_any_element(self.into_element())
    }
}

impl<T: Element> IntoElement for T {
    type Element = T;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub trait AnimationExt {
    fn with_animation(
        self,
        id: impl Into<ElementId>,
        animation: Animation,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animations: vec![animation],
            animator: Box::new(move |this, _, value| animator(this, value)),
        }
    }

    fn with_animations(
        self,
        id: impl Into<ElementId>,
        animations: Vec<Animation>,
        animator: impl Fn(Self, usize, f32) -> Self + 'static,
    ) -> AnimationElement<Self>
    where
        Self: Sized,
    {
        AnimationElement {
            id: id.into(),
            element: Some(self),
            animations,
            animator: Box::new(animator),
        }
    }
}

impl<E: IntoElement + 'static> AnimationExt for E {}

pub struct AnimationElement<E> {
    id: ElementId,
    element: Option<E>,
    animations: Vec<Animation>,
    animator: Box<dyn Fn(E, usize, f32) -> E + 'static>,
}

impl<E> AnimationElement<E> {
    pub fn map_element(mut self, f: impl FnOnce(E) -> E) -> AnimationElement<E> {
        self.element = self.element.map(f);
        self
    }
}

impl<E: IntoElement + 'static> Element for AnimationElement<E> {
    fn into_node(mut self, context: &mut RenderContext) -> UiNode {
        let animation_index = self.animations.len().saturating_sub(1);
        let element = self
            .element
            .take()
            .expect("animation element can only render once");
        let mut node = (self.animator)(element, animation_index, 1.0)
            .into_any_element()
            .into_node(context);
        node.props.insert(
            "animation_element_id".to_string(),
            self.id.to_string().into(),
        );
        node
    }
}

pub trait RenderOnce: 'static + Sized {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}

pub struct Component<C: RenderOnce> {
    component: Option<C>,
}

impl<C: RenderOnce> Component<C> {
    pub fn new(component: C) -> Self {
        Self {
            component: Some(component),
        }
    }
}

impl<C: RenderOnce> Element for Component<C> {
    fn into_node(mut self, context: &mut RenderContext) -> UiNode {
        let mut window = Window::default();
        self.component
            .take()
            .expect("component can only render once")
            .render(&mut window, &mut context.runtime)
            .into_any_element()
            .into_node(context)
    }
}

pub trait ParentElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);

    fn child(mut self, child: impl IntoElement) -> Self
    where
        Self: Sized,
    {
        self.extend(std::iter::once(child.into_any_element()));
        self
    }

    fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self
    where
        Self: Sized,
    {
        self.extend(Iterator::map(
            children.into_iter(),
            IntoElement::into_any_element,
        ));
        self
    }
}

pub trait InteractiveElement: Sized {
    fn interactivity(&mut self) -> &mut Interactivity;

    fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.interactivity().group = Some(group.into());
        self
    }

    fn id(mut self, id: impl Into<ElementId>) -> Stateful<Self> {
        self.interactivity().element_id = Some(id.into());
        Stateful::new(self)
    }

    fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.interactivity().focusable = true;
        self.interactivity().tracked_focus_handle = Some(*focus_handle);
        self
    }

    fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.interactivity().tab_stop = tab_stop;
        self
    }

    fn tab_index(mut self, index: isize) -> Self {
        self.interactivity().focusable = true;
        self.interactivity().tab_index = Some(index);
        self.interactivity().tab_stop = true;
        self
    }

    fn tab_group(mut self) -> Self {
        self.interactivity().tab_group = true;
        if self.interactivity().tab_index.is_none() {
            self.interactivity().tab_index = Some(0);
        }
        self
    }

    fn key_context<C, E>(mut self, key_context: C) -> Self
    where
        C: TryInto<KeyContext, Error = E>,
        E: fmt::Debug,
    {
        if key_context.try_into().is_ok() {
            self.interactivity().key_context = true;
        }
        self
    }

    fn hover(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity().hover_style = Some(f(StyleRefinement::default()));
        self
    }

    fn group_hover(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.interactivity().group_hover_style = Some(GroupStyle {
            group: group_name.into(),
            style: f(StyleRefinement::default()),
        });
        self
    }

    fn on_mouse_down(
        mut self,
        button: MouseButton,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MouseDown,
            EventPhase::Bubble,
            Some(button),
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MouseDown(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_any_mouse_down(
        mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MouseDown,
            EventPhase::Capture,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MouseDown(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn on_any_mouse_down(
        mut self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MouseDown,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MouseDown(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn on_mouse_up(
        mut self,
        button: MouseButton,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MouseUp,
            EventPhase::Bubble,
            Some(button),
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MouseUp(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_any_mouse_up(
        mut self,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MouseUp,
            EventPhase::Capture,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MouseUp(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn on_mouse_pressure(
        mut self,
        listener: impl Fn(&MousePressureEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MousePressure,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MousePressure(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_mouse_pressure(
        mut self,
        listener: impl Fn(&MousePressureEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MousePressure,
            EventPhase::Capture,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MousePressure(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn on_mouse_down_out(
        self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.capture_any_mouse_down(listener)
    }

    fn on_mouse_up_out(
        self,
        button: MouseButton,
        listener: impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_mouse_up(button, listener)
    }

    fn on_mouse_move(
        mut self,
        listener: impl Fn(&MouseMoveEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::MouseMove,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::MouseMove(mouse_event) = event {
                    listener(mouse_event, window, app);
                }
            }),
        );
        self
    }

    fn on_drag_move<T: 'static>(
        self,
        _listener: impl Fn(&gpui::DragMoveEvent<T>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self
    }

    fn on_scroll_wheel(
        mut self,
        listener: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::ScrollWheel,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::ScrollWheel(scroll_event) = event {
                    listener(scroll_event, window, app);
                }
            }),
        );
        self
    }

    fn on_pinch(mut self, listener: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static) -> Self {
        self.interactivity().push_handler(
            UiEventKind::Pinch,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::Pinch(pinch_event) = event {
                    listener(pinch_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_pinch(
        mut self,
        listener: impl Fn(&PinchEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::Pinch,
            EventPhase::Capture,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::Pinch(pinch_event) = event {
                    listener(pinch_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_action<A: Action>(
        mut self,
        listener: impl Fn(&A, &mut Window, &mut App) + 'static,
    ) -> Self {
        let action_name = A::name_for_type().to_string();
        self.interactivity().push_handler(
            UiEventKind::Action,
            EventPhase::Capture,
            None,
            Some(action_name.clone()),
            Rc::new(move |event, window, app| {
                if let UiEvent::Action(action_event) = event
                    && action_event.name == action_name
                {
                    let payload = action_event
                        .payload
                        .clone()
                        .unwrap_or(serde_json::Value::Null);
                    match A::build(payload) {
                        Ok(action) => {
                            if let Some(action) = action.as_any().downcast_ref::<A>() {
                                listener(action, window, app);
                            }
                        }
                        Err(error) => app.record_error(error),
                    }
                }
            }),
        );
        self
    }

    fn on_action<A: Action>(
        mut self,
        listener: impl Fn(&A, &mut Window, &mut App) + 'static,
    ) -> Self {
        let action_name = A::name_for_type().to_string();
        self.interactivity().push_handler(
            UiEventKind::Action,
            EventPhase::Bubble,
            None,
            Some(action_name.clone()),
            Rc::new(move |event, window, app| {
                if let UiEvent::Action(action_event) = event
                    && action_event.name == action_name
                {
                    let payload = action_event
                        .payload
                        .clone()
                        .unwrap_or(serde_json::Value::Null);
                    match A::build(payload) {
                        Ok(action) => {
                            if let Some(action) = action.as_any().downcast_ref::<A>() {
                                listener(action, window, app);
                            }
                        }
                        Err(error) => app.record_error(error),
                    }
                }
            }),
        );
        self
    }

    fn on_boxed_action(
        mut self,
        action: &dyn Action,
        listener: impl Fn(&dyn Action, &mut Window, &mut App) + 'static,
    ) -> Self {
        let action_name = action.name().to_string();
        self.interactivity().push_handler(
            UiEventKind::Action,
            EventPhase::Bubble,
            None,
            Some(action_name.clone()),
            Rc::new(move |event, window, app| {
                if let UiEvent::Action(action_event) = event
                    && action_event.name == action_name
                {
                    listener(action_event, window, app);
                }
            }),
        );
        self
    }

    fn on_key_down(
        mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::KeyDown,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::KeyDown(key_event) = event {
                    listener(key_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_key_down(
        mut self,
        listener: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::KeyDown,
            EventPhase::Capture,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::KeyDown(key_event) = event {
                    listener(key_event, window, app);
                }
            }),
        );
        self
    }

    fn on_key_up(
        mut self,
        listener: impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::KeyUp,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::KeyUp(key_event) = event {
                    listener(key_event, window, app);
                }
            }),
        );
        self
    }

    fn capture_key_up(
        mut self,
        listener: impl Fn(&KeyUpEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::KeyUp,
            EventPhase::Capture,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::KeyUp(key_event) = event {
                    listener(key_event, window, app);
                }
            }),
        );
        self
    }

    fn on_modifiers_changed(
        mut self,
        listener: impl Fn(&ModifiersChangedEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity().push_handler(
            UiEventKind::ModifiersChanged,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::ModifiersChanged(modifier_event) = event {
                    listener(modifier_event, window, app);
                }
            }),
        );
        self
    }

    fn drag_over<S: 'static>(
        self,
        _f: impl 'static + Fn(StyleRefinement, &S, &mut Window, &mut App) -> StyleRefinement,
    ) -> Self {
        self
    }

    fn group_drag_over<S: 'static>(
        self,
        _group_name: impl Into<SharedString>,
        _f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self
    }

    fn on_drop<T: 'static>(self, _listener: impl Fn(&T, &mut Window, &mut App) + 'static) -> Self {
        self
    }

    fn can_drop(
        self,
        _predicate: impl Fn(&dyn Any, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self
    }

    fn occlude(mut self) -> Self {
        self.interactivity().occlude = true;
        self
    }

    fn window_control_area(mut self, area: WindowControlArea) -> Self {
        self.interactivity().window_control_area = Some(area);
        self
    }

    fn block_mouse_except_scroll(mut self) -> Self {
        self.interactivity().block_mouse_except_scroll = true;
        self
    }

    fn focus(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity().focus_style = Some(f(StyleRefinement::default()));
        self
    }

    fn in_focus(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity().in_focus_style = Some(f(StyleRefinement::default()));
        self
    }

    fn focus_visible(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity().focus_visible_style = Some(f(StyleRefinement::default()));
        self
    }
}

impl<E: InteractiveElement> InteractiveElement for Stateful<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

pub trait StatefulInteractiveElement: InteractiveElement {
    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.interactivity().push_handler(
            UiEventKind::Click,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::Click(click_event) = event {
                    handler(click_event, window, app);
                }
            }),
        );
        self
    }

    fn focusable(mut self) -> Self {
        self.interactivity().focusable = true;
        self
    }

    fn overflow_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
        self.interactivity().base_style.overflow.y = Some(Overflow::Scroll);
        self
    }

    fn overflow_x_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.x = Some(Overflow::Scroll);
        self
    }

    fn overflow_y_scroll(mut self) -> Self {
        self.interactivity().base_style.overflow.y = Some(Overflow::Scroll);
        self
    }

    fn scrollbar_width(mut self, width: impl Into<AbsoluteLength>) -> Self {
        self.interactivity().base_style.scrollbar_width = Some(width.into());
        self
    }

    fn track_scroll(mut self, scroll_handle: &ScrollHandle) -> Self {
        self.interactivity().tracked_scroll_handle = Some(scroll_handle.clone());
        self
    }

    fn anchor_scroll(mut self, scroll_anchor: Option<ScrollAnchor>) -> Self {
        self.interactivity().scroll_anchor = scroll_anchor;
        self
    }

    fn active(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.interactivity().active_style = Some(f(StyleRefinement::default()));
        self
    }

    fn group_active(
        mut self,
        group_name: impl Into<SharedString>,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        self.interactivity().group_active_style = Some(GroupStyle {
            group: group_name.into(),
            style: f(StyleRefinement::default()),
        });
        self
    }

    fn on_aux_click(
        mut self,
        listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self.interactivity().push_handler(
            UiEventKind::AuxClick,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::AuxClick(click_event) = event {
                    listener(click_event, window, app);
                }
            }),
        );
        self
    }

    fn on_drag<T, W>(
        self,
        _value: T,
        _constructor: impl Fn(&T, Point<Pixels>, &mut Window, &mut App) -> Entity<W> + 'static,
    ) -> Self
    where
        Self: Sized,
        T: 'static,
        W: 'static + Render,
    {
        self
    }

    fn on_hover(mut self, listener: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self
    where
        Self: Sized,
    {
        self.interactivity().push_handler(
            UiEventKind::Hover,
            EventPhase::Bubble,
            None,
            None,
            Rc::new(move |event, window, app| {
                if let UiEvent::Hover(is_hovered) = event {
                    listener(is_hovered, window, app);
                }
            }),
        );
        self
    }

    fn tooltip(self, _build_tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self
    where
        Self: Sized,
    {
        self
    }

    fn hoverable_tooltip(
        self,
        _build_tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self
    where
        Self: Sized,
    {
        self
    }
}

impl<T: InteractiveElement> StatefulInteractiveElement for T {}

pub trait Render: 'static + Sized {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;
}

pub trait RenderRoot: Render {}

impl<T: Render> RenderRoot for T {}

pub struct RenderContext {
    generation: u64,
    runtime: Runtime,
    next_handler_index: usize,
    handlers: BTreeMap<HandlerId, EventDispatch>,
}

impl RenderContext {
    fn new(generation: u64, runtime: Runtime) -> Self {
        Self {
            generation,
            runtime,
            next_handler_index: 1,
            handlers: BTreeMap::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn register_click_handler(&mut self, handler: ClickHandler) -> UiNodeEvent {
        self.register_event_handler(PendingEventHandler {
            kind: UiEventKind::Click,
            phase: EventPhase::Bubble,
            mouse_button: None,
            action_name: None,
            dispatch: Rc::new(move |event, window, runtime| {
                if let UiEvent::Click(click_event) = event {
                    handler(click_event, window, runtime);
                }
            }),
        })
    }

    pub fn register_event_handler(&mut self, handler: PendingEventHandler) -> UiNodeEvent {
        let handler_id = EventHandlerId::new(format!("h_{}", self.next_handler_index));
        self.next_handler_index += 1;

        self.handlers.insert(handler_id.clone(), handler.dispatch);

        UiNodeEvent {
            event: handler.kind,
            handler_id,
            phase: match handler.phase {
                EventPhase::Capture => UiEventPhase::Capture,
                EventPhase::Bubble => UiEventPhase::Bubble,
            },
            mouse_button: handler.mouse_button,
            action_name: handler.action_name,
        }
    }

    fn finish(self, tree: UiNode) -> RenderOutput {
        RenderOutput {
            generation: self.generation,
            tree,
            handlers: self.handlers,
        }
    }
}

pub struct RenderOutput {
    generation: u64,
    pub tree: UiNode,
    handlers: BTreeMap<HandlerId, EventDispatch>,
}

impl fmt::Debug for RenderOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderOutput")
            .field("generation", &self.generation)
            .field("tree", &self.tree)
            .finish()
    }
}

impl RenderOutput {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn dispatch(
        &self,
        handler_id: HandlerId,
        event: &UiEvent,
        window: &mut Window,
        runtime: &mut Runtime,
    ) -> Result<()> {
        let handler = self
            .handlers
            .get(&handler_id)
            .with_context(|| format!("unknown handler id {handler_id}"))?;
        handler(event, window, runtime);
        Ok(())
    }
}

impl Runtime {
    fn frozen_global<G: Global>(&self) -> Option<&G> {
        let type_id = TypeId::of::<G>();
        if let Some(global) = self.state.borrow().frozen_globals.get(&type_id).copied() {
            return global.downcast_ref::<G>();
        }

        let leaked = {
            let mut state = self.state.borrow_mut();
            let global = state.globals.remove(&type_id)?;
            let leaked: FrozenGlobal = Box::leak(global);
            state.frozen_globals.insert(type_id, leaked);
            leaked
        };

        leaked.downcast_ref::<G>()
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_entity<T: 'static>(
        &mut self,
        build: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        let entity_id = {
            let mut state = self.state.borrow_mut();
            let entity_id = state.next_entity_id;
            state.next_entity_id += 1;
            entity_id
        };
        let runtime = Rc::downgrade(&self.state);
        let runtime_handle = self.clone();

        let theme = self.theme.clone();
        let theme_registry = self.theme_registry.clone();
        let value = Rc::new_cyclic(|weak_value| {
            let entity = WeakEntity {
                entity_id,
                value: weak_value.clone(),
                runtime: runtime.clone(),
                entity_type: PhantomData,
            };
            let mut cx = Context::new(runtime_handle, entity);
            EntityValue {
                entity_id,
                runtime: runtime.clone(),
                theme_registry: theme_registry.clone(),
                theme: theme.clone(),
                active_readers: Cell::new(0),
                active_writer: Cell::new(false),
                value: UnsafeCell::new(build(&mut cx)),
            }
        });

        Entity {
            entity_id,
            value,
            runtime,
            entity_type: PhantomData,
        }
    }

    pub fn render_root<T: RenderRoot>(&mut self, entity: &Entity<T>) -> Result<RenderOutput> {
        let generation = {
            let mut state = self.state.borrow_mut();
            let generation = state.next_generation;
            state.next_generation += 1;
            generation
        };

        let mut render_context = RenderContext::new(generation, self.clone());
        let mut window = Window::default();
        let tree = entity.update(self, |root, cx| {
            root.render(&mut window, cx)
                .into_any_element()
                .into_node(&mut render_context)
        });
        self.clear_dirty(entity.entity_id());
        Ok(render_context.finish(tree))
    }

    pub fn drain_tasks(&mut self) -> usize {
        let mut drained = 0;
        let executor = self.state.borrow().executor.clone();
        while executor.try_tick() {
            drained += 1;
        }
        drained
    }

    pub fn is_dirty(&self, entity_id: EntityId) -> bool {
        self.state.borrow().dirty_entities.contains(&entity_id)
    }

    pub fn take_errors(&mut self) -> Vec<String> {
        let mut state = self.state.borrow_mut();
        let mut errors = Vec::new();
        std::mem::swap(&mut errors, &mut state.errors);
        errors
    }

    pub fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        Task::running(smol::spawn(future))
    }

    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        let mut async_app = self.to_async();
        self.spawn_task(async move { f(&mut async_app).await })
    }

    pub fn spawn_with_priority<AsyncFn, R>(&self, _priority: Priority, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        self.spawn(f)
    }

    pub fn dispatch_action(&mut self, action: &dyn Action) {
        let mut window = Window::default();
        window.dispatch_action(action.boxed_clone(), self);
    }

    pub fn apply_host_theme(&mut self, theme_snapshot: &HostThemeSnapshot) {
        let base_theme = self
            .theme_registry
            .get(theme_snapshot.name.as_str())
            .unwrap_or_else(|_| self.theme.clone());
        let mut theme = base_theme.as_ref().clone();
        theme.id = theme_snapshot.id.clone();
        theme.name = SharedString::from(theme_snapshot.name.clone());
        theme.appearance = theme_snapshot.appearance;
        theme.styles.colors.refine(&theme_snapshot.colors);
        theme.styles.status.refine(&theme_snapshot.status);
        self.theme = Arc::new(theme);
    }

    pub fn global<G: Global>(&self) -> &G {
        self.try_global::<G>()
            .unwrap_or_else(|| panic!("global {} not set", std::any::type_name::<G>()))
    }

    pub fn try_global<G: Global>(&self) -> Option<&G> {
        self.frozen_global::<G>()
    }

    pub fn set_global<G: Global>(&mut self, global: G) {
        let type_id = TypeId::of::<G>();
        {
            let mut state = self.state.borrow_mut();
            if state.frozen_globals.contains_key(&type_id) {
                panic!(
                    "global {} was already borrowed immutably and can no longer be replaced",
                    std::any::type_name::<G>()
                );
            }
            state.globals.insert(type_id, Box::new(global));
        }
        self.notify_global_type(type_id);
    }

    pub fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global,
    {
        let type_id = TypeId::of::<G>();
        let mut boxed_global = {
            let mut state = self.state.borrow_mut();
            if state.frozen_globals.contains_key(&type_id) {
                panic!(
                    "global {} was already borrowed immutably and can no longer be updated",
                    std::any::type_name::<G>()
                );
            }
            state
                .globals
                .remove(&type_id)
                .unwrap_or_else(|| panic!("global {} not set", std::any::type_name::<G>()))
        };
        let global = boxed_global
            .downcast_mut::<G>()
            .expect("global stored under incorrect type");
        let result = f(global, self);
        self.state
            .borrow_mut()
            .globals
            .insert(type_id, boxed_global);
        self.notify_global_type(type_id);
        result
    }

    pub fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default,
    {
        let type_id = TypeId::of::<G>();
        {
            let state = self.state.borrow();
            if state.frozen_globals.contains_key(&type_id) {
                panic!(
                    "global {} was already borrowed immutably and can no longer be initialized",
                    std::any::type_name::<G>()
                );
            }
        }
        if !self.state.borrow().globals.contains_key(&type_id) {
            self.state
                .borrow_mut()
                .globals
                .insert(type_id, Box::new(G::default()));
        }
        self.update_global(f)
    }

    fn to_async(&self) -> AsyncApp {
        AsyncApp {
            runtime: self.clone(),
        }
    }

    pub async fn tick(&self) {
        let executor = self.state.borrow().executor.clone();
        executor.tick().await;
    }

    fn mark_dirty(&self, entity_id: EntityId) {
        self.state.borrow_mut().dirty_entities.insert(entity_id);
    }

    fn next_subscription_id(&self) -> usize {
        let mut state = self.state.borrow_mut();
        let subscription_id = state.next_subscription_id;
        state.next_subscription_id += 1;
        subscription_id
    }

    fn observe_entity(
        &self,
        entity_id: EntityId,
        callback: impl FnMut(&mut Runtime) -> bool + 'static,
    ) -> Subscription {
        let subscription_id = self.next_subscription_id();
        self.state
            .borrow_mut()
            .observers
            .entry(entity_id)
            .or_default()
            .insert(subscription_id, Rc::new(RefCell::new(callback)));
        Subscription {
            runtime: Rc::downgrade(&self.state),
            subscription_id,
            kind: SubscriptionKind::Observe { entity_id },
        }
    }

    fn observe_entity_release(
        &self,
        entity_id: EntityId,
        callback: impl FnMut(&mut dyn Any, &mut Runtime) -> bool + 'static,
    ) -> Subscription {
        let subscription_id = self.next_subscription_id();
        self.state
            .borrow_mut()
            .release_observers
            .entry(entity_id)
            .or_default()
            .insert(subscription_id, Rc::new(RefCell::new(callback)));
        Subscription {
            runtime: Rc::downgrade(&self.state),
            subscription_id,
            kind: SubscriptionKind::Release { entity_id },
        }
    }

    fn subscribe_to_event(
        &self,
        entity_id: EntityId,
        event_type: TypeId,
        callback: impl FnMut(&dyn Any, &mut Runtime) -> bool + 'static,
    ) -> Subscription {
        let subscription_id = self.next_subscription_id();
        self.state
            .borrow_mut()
            .event_subscribers
            .entry((entity_id, event_type))
            .or_default()
            .insert(subscription_id, Rc::new(RefCell::new(callback)));
        Subscription {
            runtime: Rc::downgrade(&self.state),
            subscription_id,
            kind: SubscriptionKind::Event {
                entity_id,
                event_type,
            },
        }
    }

    fn observe_global_type(
        &self,
        global_type: TypeId,
        callback: impl FnMut(&mut Runtime) -> bool + 'static,
    ) -> Subscription {
        let subscription_id = self.next_subscription_id();
        self.state
            .borrow_mut()
            .global_observers
            .entry(global_type)
            .or_default()
            .insert(subscription_id, Rc::new(RefCell::new(callback)));
        Subscription {
            runtime: Rc::downgrade(&self.state),
            subscription_id,
            kind: SubscriptionKind::Global { global_type },
        }
    }

    fn observe_app_restart(
        &self,
        callback: impl FnMut(&mut Runtime) -> bool + 'static,
    ) -> Subscription {
        let subscription_id = self.next_subscription_id();
        self.state
            .borrow_mut()
            .app_restart_observers
            .insert(subscription_id, Rc::new(RefCell::new(callback)));
        Subscription {
            runtime: Rc::downgrade(&self.state),
            subscription_id,
            kind: SubscriptionKind::AppRestart,
        }
    }

    fn observe_app_quit(
        &self,
        callback: impl FnMut(&mut Runtime) -> LocalBoxFuture<'static, ()> + 'static,
    ) -> Subscription {
        let subscription_id = self.next_subscription_id();
        self.state
            .borrow_mut()
            .app_quit_observers
            .insert(subscription_id, Rc::new(RefCell::new(callback)));
        Subscription {
            runtime: Rc::downgrade(&self.state),
            subscription_id,
            kind: SubscriptionKind::AppQuit,
        }
    }

    pub fn run_app_restart_callbacks(&mut self) {
        let callbacks = {
            let state = self.state.borrow();
            let mut callbacks = Vec::with_capacity(state.app_restart_observers.len());
            for (subscription_id, callback) in state.app_restart_observers.iter() {
                callbacks.push((*subscription_id, callback.clone()));
            }
            callbacks
        };
        for (subscription_id, callback) in callbacks {
            let keep = callback.borrow_mut()(self);
            if !keep {
                self.state
                    .borrow_mut()
                    .app_restart_observers
                    .remove(&subscription_id);
            }
        }
    }

    pub fn run_app_quit_callbacks(&mut self) {
        let callbacks = {
            let mut state = self.state.borrow_mut();
            let mut callbacks = Vec::with_capacity(state.app_quit_observers.len());
            for (_, callback) in state.app_quit_observers.drain() {
                callbacks.push(callback);
            }
            callbacks
        };
        let mut futures = Vec::with_capacity(callbacks.len());
        for callback in callbacks {
            futures.push(callback.borrow_mut()(self));
        }
        smol::block_on(async move {
            for future in futures {
                future.await;
            }
        });
    }

    fn notify_entity(&self, entity_id: EntityId) {
        self.mark_dirty(entity_id);
        let callbacks: Vec<(usize, ObserverCallback)> = self
            .state
            .borrow()
            .observers
            .get(&entity_id)
            .map(|callbacks| {
                let mut entries = Vec::with_capacity(callbacks.len());
                for (id, callback) in callbacks.iter() {
                    entries.push((*id, callback.clone()));
                }
                entries
            })
            .unwrap_or_default();
        for (subscription_id, callback) in callbacks {
            let keep = callback.borrow_mut()(&mut self.clone());
            if !keep && let Some(observers) = self.state.borrow_mut().observers.get_mut(&entity_id)
            {
                observers.remove(&subscription_id);
            }
        }
    }

    fn emit_event<Evt: 'static>(&self, entity_id: EntityId, event: &Evt) {
        let event_type = TypeId::of::<Evt>();
        let callbacks: Vec<(usize, EventSubscriberCallback)> = self
            .state
            .borrow()
            .event_subscribers
            .get(&(entity_id, event_type))
            .map(|callbacks| {
                let mut entries = Vec::with_capacity(callbacks.len());
                for (id, callback) in callbacks.iter() {
                    entries.push((*id, callback.clone()));
                }
                entries
            })
            .unwrap_or_default();
        for (subscription_id, callback) in callbacks {
            let keep = callback.borrow_mut()(event, &mut self.clone());
            if !keep
                && let Some(subscribers) = self
                    .state
                    .borrow_mut()
                    .event_subscribers
                    .get_mut(&(entity_id, event_type))
            {
                subscribers.remove(&subscription_id);
            }
        }
    }

    fn notify_global_type(&self, global_type: TypeId) {
        let callbacks: Vec<(usize, ObserverCallback)> = self
            .state
            .borrow()
            .global_observers
            .get(&global_type)
            .map(|callbacks| {
                let mut entries = Vec::with_capacity(callbacks.len());
                for (id, callback) in callbacks.iter() {
                    entries.push((*id, callback.clone()));
                }
                entries
            })
            .unwrap_or_default();
        for (subscription_id, callback) in callbacks {
            let keep = callback.borrow_mut()(&mut self.clone());
            if !keep
                && let Some(observers) = self
                    .state
                    .borrow_mut()
                    .global_observers
                    .get_mut(&global_type)
            {
                observers.remove(&subscription_id);
            }
        }
    }

    fn clear_dirty(&self, entity_id: EntityId) {
        self.state.borrow_mut().dirty_entities.remove(&entity_id);
    }

    fn spawn_task<R>(&self, future: impl Future<Output = R> + 'static) -> Task<R>
    where
        R: 'static,
    {
        let executor = self.state.borrow().executor.clone();
        Task::running(executor.spawn(future))
    }

    fn record_error(&self, error: anyhow::Error) {
        self.state.borrow_mut().errors.push(error.to_string());
    }

    fn next_entity_id(&mut self) -> EntityId {
        let mut state = self.state.borrow_mut();
        let entity_id = state.next_entity_id;
        state.next_entity_id += 1;
        entity_id
    }

    fn insert_reserved_entity<T: 'static>(
        &mut self,
        entity_id: EntityId,
        build: impl FnOnce(&mut Context<'_, T>) -> T,
    ) -> Entity<T> {
        let runtime = Rc::downgrade(&self.state);
        let runtime_handle = self.clone();
        let theme = self.theme.clone();
        let theme_registry = self.theme_registry.clone();
        let value = Rc::new_cyclic(|weak_value| {
            let entity = WeakEntity {
                entity_id,
                value: weak_value.clone(),
                runtime: runtime.clone(),
                entity_type: PhantomData,
            };
            let mut cx = Context::new(runtime_handle, entity);
            EntityValue {
                entity_id,
                runtime: runtime.clone(),
                theme_registry: theme_registry.clone(),
                theme: theme.clone(),
                active_readers: Cell::new(0),
                active_writer: Cell::new(false),
                value: UnsafeCell::new(build(&mut cx)),
            }
        });

        Entity {
            entity_id,
            value,
            runtime,
            entity_type: PhantomData,
        }
    }
}

impl AppContext for Runtime {
    fn new<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Context<'_, T>) -> T,
    ) -> Entity<T> {
        self.new_entity(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        Reservation {
            entity_id: self.next_entity_id(),
            reservation_type: PhantomData,
        }
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<'_, T>) -> T,
    ) -> Entity<T> {
        self.insert_reserved_entity(reservation.entity_id, build_entity)
    }

    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<'_, T>) -> R,
    ) -> R
    where
        T: 'static,
    {
        handle.update(self, update)
    }

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        GpuiBorrow {
            borrow: EntityValue::borrow_mut(&handle.value),
            borrow_lifetime: PhantomData,
        }
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        handle.read_with(self, read)
    }

    fn update_window<T, F>(&mut self, _window: AnyWindowHandle, _f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        anyhow::bail!("window updates are unsupported in the plugin runtime");
    }

    fn read_window<T, R>(
        &self,
        _window: &WindowHandle<T>,
        _read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        anyhow::bail!("window reads are unsupported in the plugin runtime");
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        Runtime::background_spawn(self, future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        if let Some(global) = self
            .state
            .borrow()
            .frozen_globals
            .get(&TypeId::of::<G>())
            .copied()
        {
            let global = global
                .downcast_ref::<G>()
                .expect("global stored under incorrect type");
            return callback(global, self);
        }

        let state = self.state.borrow();
        let global = state
            .globals
            .get(&TypeId::of::<G>())
            .unwrap_or_else(|| panic!("global {} not set", std::any::type_name::<G>()))
            .downcast_ref::<G>()
            .expect("global stored under incorrect type");
        callback(global, self)
    }
}

impl<T: 'static> AppContext for Context<'_, T> {
    fn new<U: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Context<'_, U>) -> U,
    ) -> Entity<U> {
        self.runtime.new(build_entity)
    }

    fn reserve_entity<U: 'static>(&mut self) -> Reservation<U> {
        self.runtime.reserve_entity()
    }

    fn insert_entity<U: 'static>(
        &mut self,
        reservation: Reservation<U>,
        build_entity: impl FnOnce(&mut Context<'_, U>) -> U,
    ) -> Entity<U> {
        self.runtime.insert_entity(reservation, build_entity)
    }

    fn update_entity<U, R>(
        &mut self,
        handle: &Entity<U>,
        update: impl FnOnce(&mut U, &mut Context<'_, U>) -> R,
    ) -> R
    where
        U: 'static,
    {
        self.runtime.update_entity(handle, update)
    }

    fn as_mut<'a, U>(&'a mut self, handle: &Entity<U>) -> GpuiBorrow<'a, U>
    where
        U: 'static,
    {
        self.runtime.as_mut(handle)
    }

    fn read_entity<U, R>(&self, handle: &Entity<U>, read: impl FnOnce(&U, &App) -> R) -> R
    where
        U: 'static,
    {
        self.runtime.read_entity(handle, read)
    }

    fn update_window<U, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<U>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> U,
    {
        self.runtime.update_window(window, f)
    }

    fn read_window<U, R>(
        &self,
        window: &WindowHandle<U>,
        read: impl FnOnce(Entity<U>, &App) -> R,
    ) -> Result<R>
    where
        U: 'static,
    {
        self.runtime.read_window(window, read)
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.runtime.background_spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        self.runtime.read_global(callback)
    }
}

impl BorrowAppContext for Runtime {
    fn set_global<T: Global>(&mut self, global: T) {
        Runtime::set_global(self, global);
    }

    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global,
    {
        Runtime::update_global(self, f)
    }

    fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default,
    {
        Runtime::update_default_global(self, f)
    }
}

impl<T: 'static> BorrowAppContext for Context<'_, T> {
    fn set_global<G: Global>(&mut self, global: G) {
        self.runtime.set_global(global);
    }

    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global,
    {
        let type_id = TypeId::of::<G>();
        let mut boxed_global = {
            let mut state = self.runtime.state.borrow_mut();
            if state.frozen_globals.contains_key(&type_id) {
                panic!(
                    "global {} was already borrowed immutably and can no longer be updated",
                    std::any::type_name::<G>()
                );
            }
            state
                .globals
                .remove(&type_id)
                .unwrap_or_else(|| panic!("global {} not set", std::any::type_name::<G>()))
        };
        let global = boxed_global
            .downcast_mut::<G>()
            .expect("global stored under incorrect type");
        let result = f(global, self);
        self.runtime
            .state
            .borrow_mut()
            .globals
            .insert(type_id, boxed_global);
        self.runtime.notify_global_type(type_id);
        result
    }

    fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default,
    {
        let type_id = TypeId::of::<G>();
        if self
            .runtime
            .state
            .borrow()
            .frozen_globals
            .contains_key(&type_id)
        {
            panic!(
                "global {} was already borrowed immutably and can no longer be initialized",
                std::any::type_name::<G>()
            );
        }
        if !self.runtime.state.borrow().globals.contains_key(&type_id) {
            self.runtime.set_global(G::default());
        }
        self.update_global(f)
    }
}

impl AppContext for AsyncApp {
    fn new<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Context<'_, T>) -> T,
    ) -> Entity<T> {
        self.runtime.new(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        self.runtime.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<'_, T>) -> T,
    ) -> Entity<T> {
        self.runtime.insert_entity(reservation, build_entity)
    }

    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<'_, T>) -> R,
    ) -> R
    where
        T: 'static,
    {
        self.runtime.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> GpuiBorrow<'a, T>
    where
        T: 'static,
    {
        self.runtime.as_mut(handle)
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        self.runtime.read_entity(handle, read)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.runtime.update_window(window, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        self.runtime.read_window(window, read)
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.runtime.background_spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        self.runtime.read_global(callback)
    }
}

impl BorrowAppContext for AsyncApp {
    fn set_global<T: Global>(&mut self, global: T) {
        self.runtime.set_global(global);
    }

    fn update_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global,
    {
        let type_id = TypeId::of::<G>();
        let mut boxed_global = {
            let mut state = self.runtime.state.borrow_mut();
            if state.frozen_globals.contains_key(&type_id) {
                panic!(
                    "global {} was already borrowed immutably and can no longer be updated",
                    std::any::type_name::<G>()
                );
            }
            state
                .globals
                .remove(&type_id)
                .unwrap_or_else(|| panic!("global {} not set", std::any::type_name::<G>()))
        };
        let global = boxed_global
            .downcast_mut::<G>()
            .expect("global stored under incorrect type");
        let result = f(global, self);
        self.runtime
            .state
            .borrow_mut()
            .globals
            .insert(type_id, boxed_global);
        result
    }

    fn update_default_global<G, R>(&mut self, f: impl FnOnce(&mut G, &mut Self) -> R) -> R
    where
        G: Global + Default,
    {
        let type_id = TypeId::of::<G>();
        if self
            .runtime
            .state
            .borrow()
            .frozen_globals
            .contains_key(&type_id)
        {
            panic!(
                "global {} was already borrowed immutably and can no longer be initialized",
                std::any::type_name::<G>()
            );
        }
        if !self.runtime.state.borrow().globals.contains_key(&type_id) {
            self.runtime.set_global(G::default());
        }
        self.update_global(f)
    }
}
