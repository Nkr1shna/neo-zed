use gpui::prelude::*;
use gpui_plugin as gpui;
use std::time::Duration;

gpui::actions!(plugin_test, [GeneratedAction]);

#[derive(gpui::IntoElement)]
struct DerivedComponent;

impl gpui::RenderOnce for DerivedComponent {
    fn render(self, _window: &mut gpui::Window, _cx: &mut gpui::App) -> impl gpui::IntoElement {
        gpui::Empty
    }
}

#[derive(gpui::Render)]
struct DerivedView;

#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = plugin_test, no_json, no_register)]
struct DerivedAction;

#[test]
fn gpui_plugin_exposes_macro_and_prelude_surface() {
    let _ = DerivedComponent.into_element();
    let runtime = gpui::Runtime::new();
    let _ = runtime.theme();
    let _ = gpui::Rems(1.0);
    let _ = gpui::AvailableSpace::Definite(gpui::px(42.0));
    let _: Option<gpui::LayoutId> = None;
    let _ = gpui::GpuSpecs::default();
    let _ = gpui::AnimationExt::with_animation(
        gpui::div(),
        "animated-div",
        gpui::Animation::new(Duration::from_millis(100)),
        |this, _delta| this,
    );
    let _ = gpui::Transformation::default()
        .with_scaling(gpui::size(1.0, 1.0))
        .with_translation(gpui::point(gpui::px(1.0), gpui::px(2.0)))
        .with_rotation(gpui::radians(0.5));

    fn assert_render<T: gpui::Render>() {}
    fn assert_into_element<T: gpui::IntoElement>() {}
    fn assert_render_once<T: gpui::RenderOnce>() {}
    fn assert_stateful<T: gpui::StatefulInteractiveElement>() {}
    fn assert_action<T: gpui::Action>() {}
    fn assert_animation_ext<T: gpui::AnimationExt>() {}

    assert_render::<DerivedView>();
    assert_into_element::<gpui::Component<DerivedComponent>>();
    assert_render_once::<DerivedComponent>();
    assert_stateful::<gpui::Div>();
    assert_action::<GeneratedAction>();
    assert_action::<DerivedAction>();
    assert_animation_ext::<gpui::Div>();
}
