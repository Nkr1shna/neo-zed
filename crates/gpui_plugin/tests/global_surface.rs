use gpui_plugin as gpui;

#[derive(Default)]
struct Counter(u32);

impl gpui::Global for Counter {}

#[test]
fn gpui_plugin_exposes_global_context_surface() {
    let mut app = gpui::App::new();
    gpui::BorrowAppContext::update_default_global(&mut app, |counter: &mut Counter, _app| {
        counter.0 += 1;
    });

    let value = gpui::AppContext::read_global(&app, |counter: &Counter, _app| counter.0);
    assert_eq!(value, 1);
    assert_eq!(app.global::<Counter>().0, 1);
    assert_eq!(<Counter as gpui::ReadGlobal>::global(&app).0, 1);
    assert!(app.try_global::<Counter>().is_some());
}
