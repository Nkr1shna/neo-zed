use gpui::Action as _;
use gpui_plugin as gpui;
use std::{any::TypeId, cell::RefCell, rc::Rc};

gpui::actions!(window_surface, [OpenThing]);

#[test]
fn gpui_plugin_exposes_context_derive_surface() {
    #[derive(gpui::AppContext, gpui::VisualContext)]
    struct DerivedContext {
        #[app]
        app: &'static mut gpui::App,
        #[window]
        window: &'static mut gpui::Window,
    }

    let app = Box::leak(Box::new(gpui::App::new()));
    let window = Box::leak(Box::new(gpui::Window::default()));
    let _context = DerivedContext { app, window };
}

#[derive(Default)]
struct GlobalCounter(u32);

impl gpui::Global for GlobalCounter {}

#[derive(Clone)]
struct Ping(u32);

struct Producer;
impl gpui::EventEmitter<Ping> for Producer {}

struct Observer {
    observed: u32,
    subscribed: u32,
    global_notifications: u32,
    _subscriptions: Vec<gpui::Subscription>,
}

struct FocusableView;

impl gpui::Focusable for FocusableView {}

struct ReleaseObserver {
    _subscriptions: Vec<gpui::Subscription>,
}

struct LifecycleObserver {
    _subscriptions: Vec<gpui::Subscription>,
}

async fn spawn_in_task(
    _view: gpui::WeakEntity<FocusableView>,
    _cx: &mut gpui::AsyncWindowContext,
) -> usize {
    1
}

async fn spawn_in_priority_task(
    _view: gpui::WeakEntity<FocusableView>,
    _cx: &mut gpui::AsyncWindowContext,
) -> usize {
    2
}

async fn window_spawn_task(_cx: &mut gpui::AsyncWindowContext) -> usize {
    3
}

async fn window_spawn_priority_task(_cx: &mut gpui::AsyncWindowContext) -> usize {
    4
}

async fn app_spawn_task(_cx: &mut gpui::AsyncApp) -> usize {
    5
}

async fn app_spawn_priority_task(_cx: &mut gpui::AsyncApp) -> usize {
    6
}

#[test]
fn gpui_plugin_context_observe_subscribe_and_global_observe_work() {
    let mut app = gpui::App::new();
    let producer = app.new_entity(|_| Producer);
    let observer = app.new_entity(|cx| {
        let observe = cx.observe(&producer, |this: &mut Observer, _producer, _cx| {
            this.observed += 1;
        });
        let subscribe = cx.subscribe(
            &producer,
            |this: &mut Observer, _producer, event: &Ping, _cx| {
                this.subscribed += event.0;
            },
        );
        let observe_global = cx.observe_global::<GlobalCounter>(|this: &mut Observer, _cx| {
            this.global_notifications += 1;
        });
        Observer {
            observed: 0,
            subscribed: 0,
            global_notifications: 0,
            _subscriptions: vec![observe, subscribe, observe_global],
        }
    });

    producer.update(&mut app, |_producer, cx| {
        cx.notify();
        cx.emit(Ping(2));
    });

    gpui::BorrowAppContext::update_default_global(&mut app, |counter: &mut GlobalCounter, _app| {
        counter.0 += 1;
    });

    observer.read(|observer| {
        assert_eq!(observer.observed, 1);
        assert_eq!(observer.subscribed, 2);
        assert_eq!(observer.global_notifications, 1);
    });
}

#[test]
fn gpui_plugin_context_exposes_window_scoped_helper_surface() {
    let mut app = gpui::App::new();
    let focusable = app.new_entity(|_| FocusableView);
    let producer = app.new_entity(|_| Producer);
    let view = app.new_entity(|_| FocusableView);

    view.update(&mut app, |_view, cx| {
        let mut window = gpui::Window::default();
        cx.focus_view(&focusable, &mut window);
        cx.on_next_frame(&mut window, |_view, _window, _cx| {});
        cx.defer_in(&mut window, |_view, _window, _cx| {});
        let _observe_in = cx.observe_in(
            &focusable,
            &mut window,
            |_view, _focusable, _window, _cx| {},
        );
        let _subscribe_in = cx.subscribe_in(
            &producer,
            &window,
            |_view, _producer, _event: &Ping, _window, _cx| {},
        );
        let _observe_global_in =
            cx.observe_global_in::<GlobalCounter>(&window, |_view, _window, _cx| {});
        cx.on_action(
            TypeId::of::<Ping>(),
            &mut window,
            |_view, _action, _phase, _window, _cx| {},
        );
        cx.focus_self(&mut window);
        let _on_drop = cx.on_drop(|_view, _cx| {});
        std::mem::forget(_on_drop);
    });
}

#[test]
fn gpui_plugin_context_exposes_lifecycle_and_async_window_surface() {
    let mut app = gpui::App::new();
    let observed = app.new_entity(|_| FocusableView);
    let view = app.new_entity(|_| FocusableView);

    view.update(&mut app, |_view, cx| {
        let window = gpui::Window::default();
        let _observe_release = cx.observe_release(&observed, |_view, _observed, _cx| {});
        let _observe_release_in =
            cx.observe_release_in(&observed, &window, |_view, _observed, _window, _cx| {});
        let _on_release_in = cx.on_release_in(&window, |_view, _window, _app| {});
        let _on_app_restart = cx.on_app_restart(|_view, _app| {});
        let _on_app_quit = cx.on_app_quit(|_view, _cx| async move {});
        let _spawn_in = cx.spawn_in(&window, spawn_in_task);
        let _spawn_in_with_priority =
            cx.spawn_in_with_priority(gpui::Priority::High, &window, spawn_in_priority_task);
    });
}

#[test]
fn gpui_plugin_release_callbacks_fire_when_entities_are_dropped() {
    let mut app = gpui::App::new();
    let release_hits = Rc::new(RefCell::new(0));
    let self_release_hits = Rc::new(RefCell::new(0));
    let observed = app.new_entity(|_| FocusableView);
    let observer = app.new_entity(|cx| {
        let window = gpui::Window::default();
        let observe_release = {
            let release_hits = release_hits.clone();
            cx.observe_release(&observed, move |_view, _observed, _cx| {
                *release_hits.borrow_mut() += 1;
            })
        };
        let observe_release_in = {
            let release_hits = release_hits.clone();
            cx.observe_release_in(&observed, &window, move |_view, _observed, _window, _cx| {
                *release_hits.borrow_mut() += 10;
            })
        };
        let on_release_in = {
            let self_release_hits = self_release_hits.clone();
            cx.on_release_in(&window, move |_view, _window, _app| {
                *self_release_hits.borrow_mut() += 1;
            })
        };

        ReleaseObserver {
            _subscriptions: vec![observe_release, observe_release_in, on_release_in],
        }
    });

    drop(observed);
    assert_eq!(*release_hits.borrow(), 11);

    drop(observer);
    assert_eq!(*self_release_hits.borrow(), 1);
}

#[test]
fn gpui_plugin_exposes_window_and_app_dispatch_surface() {
    let mut app = gpui::App::new();
    let mut window = gpui::Window::default();
    let focus_handle = gpui::FocusHandle;
    let window_id = gpui::WindowId::from(7);
    let any_window = gpui::AnyWindowHandle::default();
    let typed_window = gpui::WindowHandle::<FocusableView>::new(window_id);
    let capture_hits = Rc::new(RefCell::new(0));
    let bubble_hits = Rc::new(RefCell::new(0));
    let next_frame_hits = Rc::new(RefCell::new(0));
    let deferred_hits = Rc::new(RefCell::new(0));

    {
        let capture_hits = capture_hits.clone();
        let bubble_hits = bubble_hits.clone();
        window.on_action(
            TypeId::of::<OpenThing>(),
            move |action, phase, _window, _app| {
                assert!(action.downcast_ref::<OpenThing>().is_some());
                match phase {
                    gpui::DispatchPhase::Capture => *capture_hits.borrow_mut() += 1,
                    gpui::DispatchPhase::Bubble => *bubble_hits.borrow_mut() += 1,
                }
            },
        );
    }

    window.on_next_frame(
        {
            let next_frame_hits = next_frame_hits.clone();
            move |_window, _app| *next_frame_hits.borrow_mut() += 1
        },
        &mut app,
    );
    window.defer(&mut app, {
        let deferred_hits = deferred_hits.clone();
        move |_window, _app| *deferred_hits.borrow_mut() += 1
    });

    let _async_window = window.to_async(&app);
    let _window_task = window.spawn(&app, window_spawn_task);
    let _window_priority_task =
        window.spawn_with_priority(gpui::Priority::High, &app, window_spawn_priority_task);
    focus_handle.dispatch_action(&OpenThing, &mut window, &mut app);
    window.dispatch_action(OpenThing.boxed_clone(), &mut app);
    let _app_task = app.spawn(app_spawn_task);
    let _app_priority_task = app.spawn_with_priority(gpui::Priority::High, app_spawn_priority_task);
    app.dispatch_action(&OpenThing);
    assert_eq!(window_id.as_u64(), 7);
    assert_eq!(any_window.window_id().as_u64(), 0);
    assert!(any_window.downcast::<FocusableView>().is_some());
    assert!(typed_window.read(&app).is_err());
    assert_eq!(*capture_hits.borrow(), 2);
    assert_eq!(*bubble_hits.borrow(), 2);
    assert_eq!(*next_frame_hits.borrow(), 1);
    assert_eq!(*deferred_hits.borrow(), 1);
}

#[test]
fn gpui_plugin_lifecycle_callbacks_run_when_runtime_invokes_them() {
    let mut app = gpui::App::new();
    let restart_hits = Rc::new(RefCell::new(0));
    let quit_hits = Rc::new(RefCell::new(0));

    let _view = app.new_entity(|cx| {
        let on_app_restart = {
            let restart_hits = restart_hits.clone();
            cx.on_app_restart(move |_view, _app| {
                *restart_hits.borrow_mut() += 1;
            })
        };
        let on_app_quit = {
            let quit_hits = quit_hits.clone();
            cx.on_app_quit(move |_view, _cx| {
                let quit_hits = quit_hits.clone();
                async move {
                    *quit_hits.borrow_mut() += 1;
                }
            })
        };
        LifecycleObserver {
            _subscriptions: vec![on_app_restart, on_app_quit],
        }
    });

    app.run_app_restart_callbacks();
    app.run_app_quit_callbacks();

    assert_eq!(*restart_hits.borrow(), 1);
    assert_eq!(*quit_hits.borrow(), 1);
}
