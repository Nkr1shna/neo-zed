use gpui_api::{
    Action, ActionEvent, ClickEvent, Context, KeyDownEvent, Keystroke, Render, Runtime, UiEvent,
    Window,
};
use gpui_plugin::{
    InteractiveElement, IntoElement, RenderOutput, RenderRoot, StatefulInteractiveElement, Styled,
    div, h_flex, v_flex,
};
use ui_plugin::{Button, Divider, Label};

gpui_plugin::actions!(mirror_action_test, [MirrorAction]);

struct MirrorPanel {
    clicks: usize,
}

impl Render for MirrorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(Label::new("Mirror Panel"))
            .child(Divider::horizontal())
            .child(
                Button::new("increment", format!("Clicks: {}", self.clicks)).on_click(cx.listener(
                    |this, _: &ClickEvent, _window, cx| {
                        this.clicks += 1;
                        cx.notify();
                    },
                )),
            )
    }
}

#[test]
fn mirror_render_output_serializes_expected_node_kinds_styles_and_events() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| MirrorPanel { clicks: 0 });

    let render_output = runtime.render_root(&panel).expect("render succeeds");
    let json = serde_json::to_value(&render_output.tree).expect("tree serializes");

    assert_eq!(json["kind"], "div");
    assert_eq!(json["styles"]["display"], "Flex");
    assert_eq!(json["styles"]["flex_direction"], "Column");

    let children = json["children"].as_array().expect("children array");
    assert_eq!(children[0]["kind"], "label");
    assert_eq!(children[0]["text"], "Mirror Panel");
    assert_eq!(children[1]["kind"], "divider");
    assert_eq!(children[2]["kind"], "button");
    assert_eq!(children[2]["text"], "Clicks: 0");
    assert_eq!(children[2]["events"][0]["event"], "click");
    assert!(
        children[2]["events"][0]["handler_id"]
            .as_str()
            .expect("handler id")
            .starts_with("h_")
    );
}

#[test]
fn mirror_surface_supports_div_stack_helpers_and_render_contracts() {
    let _ = div().flex().flex_row();
    let _ = h_flex();
    let _ = v_flex();

    fn assert_render_root<T: RenderRoot>() {}
    fn assert_render_output(_: &RenderOutput) {}

    assert_render_root::<MirrorPanel>();

    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| MirrorPanel { clicks: 0 });
    let render_output = runtime.render_root(&panel).expect("render succeeds");
    assert_render_output(&render_output);
}

struct InteractiveMirrorPanel {
    hovered: bool,
    key_presses: usize,
}

impl Render for InteractiveMirrorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .on_hover(cx.listener(|this, hovered, _, cx| {
                this.hovered = *hovered;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, _, _, cx| {
                this.key_presses += 1;
                cx.notify();
            }))
            .child(Label::new(format!(
                "hovered={} keys={}",
                self.hovered, self.key_presses
            )))
    }
}

#[test]
fn mirror_runtime_dispatches_non_click_events() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| InteractiveMirrorPanel {
        hovered: false,
        key_presses: 0,
    });

    let render_output = runtime.render_root(&panel).expect("render succeeds");
    let hover_handler = render_output
        .tree
        .events
        .iter()
        .find(|event| event.event == plugin_protocol::UiEventKind::Hover)
        .expect("hover handler")
        .handler_id
        .clone();
    let keydown_handler = render_output
        .tree
        .events
        .iter()
        .find(|event| event.event == plugin_protocol::UiEventKind::KeyDown)
        .expect("key down handler")
        .handler_id
        .clone();

    let mut window = Window::default();
    render_output
        .dispatch(
            hover_handler,
            &UiEvent::Hover(true),
            &mut window,
            &mut runtime,
        )
        .expect("hover dispatch succeeds");
    render_output
        .dispatch(
            keydown_handler,
            &UiEvent::KeyDown(KeyDownEvent {
                keystroke: Keystroke::parse("a").expect("keystroke parses"),
                is_held: false,
                prefer_character_input: false,
            }),
            &mut window,
            &mut runtime,
        )
        .expect("key down dispatch succeeds");

    let rerendered = runtime.render_root(&panel).expect("rerender succeeds");
    assert_eq!(
        rerendered.tree.children[0].text.as_deref(),
        Some("hovered=true keys=1")
    );
}

struct IdentityMirrorPanel;

impl Render for IdentityMirrorPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().child("identity").id("validation-canvas-root")
    }
}

#[test]
fn mirror_runtime_serializes_element_id_for_host_visible_identity() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| IdentityMirrorPanel);

    let render_output = runtime.render_root(&panel).expect("render succeeds");
    assert_eq!(
        render_output.tree.element_id.as_deref(),
        Some("validation-canvas-root")
    );
    assert_eq!(
        render_output
            .tree
            .props
            .get("element_id")
            .and_then(|value| match value {
                plugin_protocol::StyleValue::Text(value) => Some(value.as_str()),
                _ => None,
            }),
        Some("validation-canvas-root")
    );
}

struct ActionMirrorPanel {
    actions: usize,
}

impl Render for ActionMirrorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .on_action(cx.listener(|this, _: &MirrorAction, _, cx| {
                this.actions += 1;
                cx.notify();
            }))
            .child(Label::new(format!("actions={}", self.actions)))
    }
}

#[test]
fn mirror_runtime_dispatches_action_events() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| ActionMirrorPanel { actions: 0 });

    let render_output = runtime.render_root(&panel).expect("render succeeds");
    let action_binding = render_output
        .tree
        .events
        .iter()
        .find(|event| event.event == plugin_protocol::UiEventKind::Action)
        .expect("action handler");
    let handler_id = action_binding.handler_id.clone();
    assert_eq!(
        action_binding.action_name.as_deref(),
        Some(MirrorAction::name_for_type())
    );

    let mut window = Window::default();
    render_output
        .dispatch(
            handler_id,
            &UiEvent::Action(ActionEvent {
                name: MirrorAction::name_for_type().to_string(),
                payload: Some(serde_json::json!({})),
            }),
            &mut window,
            &mut runtime,
        )
        .expect("action dispatch succeeds");

    let rerendered = runtime.render_root(&panel).expect("rerender succeeds");
    assert_eq!(
        rerendered.tree.children[0].text.as_deref(),
        Some("actions=1")
    );
}
