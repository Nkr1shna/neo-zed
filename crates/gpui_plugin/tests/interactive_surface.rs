use gpui::prelude::*;
use gpui_plugin as gpui;

gpui::actions!(plugin_test, [OpenPanel]);

#[test]
fn gpui_plugin_exposes_interactive_surface() {
    let focus_handle = gpui::FocusHandle;
    let scroll_handle = gpui::ScrollHandle::new();
    let scroll_anchor = gpui::ScrollAnchor::for_handle(scroll_handle.clone());

    let _element = gpui::div()
        .group("plugin-panel")
        .id("interactive-root")
        .track_focus(&focus_handle)
        .tab_stop(true)
        .tab_index(1)
        .tab_group()
        .key_context("Workspace")
        .hover(|style| style)
        .group_hover("plugin-panel", |style| style)
        .capture_any_mouse_down(|_, _, _| {})
        .on_any_mouse_down(|_, _, _| {})
        .on_mouse_down(gpui::MouseButton::Left, |_, _, _| {})
        .on_mouse_up(gpui::MouseButton::Left, |_, _, _| {})
        .capture_any_mouse_up(|_, _, _| {})
        .on_mouse_down_out(|_, _, _| {})
        .on_mouse_up_out(gpui::MouseButton::Left, |_, _, _| {})
        .on_mouse_move(|_, _, _| {})
        .on_scroll_wheel(|_, _, _| {})
        .on_pinch(|_, _, _| {})
        .capture_pinch(|_, _, _| {})
        .on_key_down(|_, _, _| {})
        .capture_key_down(|_, _, _| {})
        .on_key_up(|_, _, _| {})
        .capture_key_up(|_, _, _| {})
        .on_modifiers_changed(|_, _, _| {})
        .drag_over::<String>(|style, _, _, _| style)
        .group_drag_over::<String>("plugin-panel", |style| style)
        .on_drop::<String>(|_, _, _| {})
        .can_drop(|_, _, _| true)
        .occlude()
        .window_control_area(gpui::WindowControlArea::Drag)
        .block_mouse_except_scroll()
        .focus(|style| style)
        .in_focus(|style| style)
        .focus_visible(|style| style)
        .focusable()
        .overflow_scroll()
        .overflow_x_scroll()
        .overflow_y_scroll()
        .scrollbar_width(gpui::px(8.0))
        .track_scroll(&scroll_handle)
        .anchor_scroll(Some(scroll_anchor))
        .active(|style| style)
        .group_active("plugin-panel", |style| style)
        .on_click(|_, _, _| {})
        .on_aux_click(|_, _, _| {})
        .on_hover(|_, _, _| {})
        .capture_action(|_: &OpenPanel, _, _| {})
        .on_action(|_: &OpenPanel, _, _| {})
        .on_boxed_action(&OpenPanel, |_, _, _| {})
        .tooltip(|_, _| gpui::AnyView::default())
        .hoverable_tooltip(|_, _| gpui::AnyView::default())
        .child("content");
}
