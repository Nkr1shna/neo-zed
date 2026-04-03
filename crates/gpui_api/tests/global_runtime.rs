use gpui_api as gpui;

#[derive(Debug, Default)]
struct GlobalCounter(u32);

impl gpui::Global for GlobalCounter {}

#[::core::prelude::v1::test]
fn try_global_freezes_globals_without_invalidating_existing_reads() {
    let mut app = gpui::App::new();
    app.set_global(GlobalCounter(7));

    let first = app
        .try_global::<GlobalCounter>()
        .expect("global is registered");
    let first_ptr = std::ptr::from_ref(first);
    assert_eq!(first.0, 7);

    let second = app
        .try_global::<GlobalCounter>()
        .expect("global remains readable");
    assert_eq!(std::ptr::from_ref(second), first_ptr);
    assert_eq!(app.global::<GlobalCounter>().0, 7);
}

#[::core::prelude::v1::test]
fn frozen_globals_cannot_be_updated_or_reinitialized() {
    let mut app = gpui::App::new();
    app.set_global(GlobalCounter(3));
    let _ = app
        .try_global::<GlobalCounter>()
        .expect("global is registered");

    let update_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.update_global::<GlobalCounter, _>(|counter, _| {
            counter.0 += 1;
        });
    }));
    assert!(
        update_result.is_err(),
        "frozen globals must reject mutation"
    );

    let reinitialize_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.update_default_global::<GlobalCounter, _>(|counter, _| {
            counter.0 += 1;
        });
    }));
    assert!(
        reinitialize_result.is_err(),
        "frozen globals must reject default initialization"
    );
}
