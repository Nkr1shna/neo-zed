use gpui_api::{
    Action, ActionEvent, ClickEvent, Context, HandlerId, Render, Runtime, UiEvent, Window,
};
use gpui_plugin::InteractiveElement;
use plugin_host::{HostToPluginMessage, PanelSession, PluginToHostMessage};
use ui_plugin::{Button, Label};

gpui_plugin::actions!(plugin_host_protocol_test, [MirrorAction]);

struct SessionPanel {
    count: usize,
    status: String,
}

impl Render for SessionPanel {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_plugin::IntoElement {
        gpui_plugin::v_flex()
            .child(Label::new(format!("Count: {}", self.count)))
            .child(Label::new(self.status.clone()))
            .child(Button::new("increment", "Increment").on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.count += 1;
                    cx.notify();
                },
            )))
            .child(Button::new("async", "Async").on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.status = "working".to_string();
                    cx.spawn(async move |this, cx| {
                        this.update(cx, |this, cx| {
                            this.status = "done".to_string();
                            cx.notify();
                        })
                        .expect("entity still alive");
                    })
                    .detach();
                },
            )))
    }
}

struct ActionSessionPanel {
    actions: usize,
}

impl Render for ActionSessionPanel {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui_plugin::IntoElement {
        gpui_plugin::div()
            .on_action(cx.listener(|this, _: &MirrorAction, _, cx| {
                this.actions += 1;
                cx.notify();
            }))
            .child(Label::new(format!("Actions: {}", self.actions)))
    }
}

fn extract_button_handler_id(message: &PluginToHostMessage, button_text: &str) -> HandlerId {
    let PluginToHostMessage::Rendered { tree, .. } = message else {
        panic!("expected rendered message");
    };

    tree.children
        .iter()
        .find(|child| child.text.as_deref() == Some(button_text))
        .and_then(|child| child.events.first())
        .map(|event| event.handler_id.clone())
        .expect("button handler id present")
}

fn extract_action_handler_id(message: &PluginToHostMessage, action_name: &str) -> HandlerId {
    let PluginToHostMessage::Rendered { tree, .. } = message else {
        panic!("expected rendered message");
    };

    tree.events
        .iter()
        .find(|event| {
            event.event == plugin_protocol::UiEventKind::Action
                && event.action_name.as_deref() == Some(action_name)
        })
        .map(|event| event.handler_id.clone())
        .expect("action handler id present")
}

#[test]
fn handler_dispatch_mutates_state_and_emits_rerender_request() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| SessionPanel {
        count: 0,
        status: "idle".to_string(),
    });
    let mut session = PanelSession::new(runtime, panel);

    let initial_render = session.initial_render().expect("initial render");
    let increment_handler_id = extract_button_handler_id(&initial_render, "Increment");

    let rerender = session
        .handle_message(HostToPluginMessage::DispatchEvent {
            handler_id: increment_handler_id,
            event: UiEvent::Click(ClickEvent::default()),
        })
        .expect("dispatch succeeds")
        .expect("message returned");

    assert!(matches!(
        rerender,
        PluginToHostMessage::RerenderRequested { .. }
    ));

    let rendered = session.render_if_dirty().expect("rerender succeeds");
    let PluginToHostMessage::Rendered { tree, .. } = rendered else {
        panic!("expected rendered tree");
    };
    assert_eq!(tree.children[0].text.as_deref(), Some("Count: 1"));
}

#[test]
fn notify_from_spawned_task_emits_rerender_request_after_task_drain() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| SessionPanel {
        count: 0,
        status: "idle".to_string(),
    });
    let mut session = PanelSession::new(runtime, panel);

    let initial_render = session.initial_render().expect("initial render");
    let async_handler_id = extract_button_handler_id(&initial_render, "Async");

    let dispatch_result = session
        .handle_message(HostToPluginMessage::DispatchEvent {
            handler_id: async_handler_id,
            event: UiEvent::Click(ClickEvent::default()),
        })
        .expect("dispatch succeeds");
    assert!(dispatch_result.is_none());

    let drained = session.drain_tasks().expect("task drain succeeds");
    assert_eq!(drained, 1);

    let rerender = session
        .take_pending_message()
        .expect("pending rerender notification");
    assert!(matches!(
        rerender,
        PluginToHostMessage::RerenderRequested { .. }
    ));

    let rendered = session.render_if_dirty().expect("rerender succeeds");
    let PluginToHostMessage::Rendered { tree, .. } = rendered else {
        panic!("expected rendered tree");
    };
    assert_eq!(tree.children[1].text.as_deref(), Some("done"));
}

#[test]
fn action_dispatch_roundtrips_through_the_plugin_session() {
    let mut runtime = Runtime::new();
    let panel = runtime.new_entity(|_| ActionSessionPanel { actions: 0 });
    let mut session = PanelSession::new(runtime, panel);

    let initial_render = session.initial_render().expect("initial render");
    let action_handler_id =
        extract_action_handler_id(&initial_render, MirrorAction::name_for_type());

    let rerender = session
        .handle_message(HostToPluginMessage::DispatchEvent {
            handler_id: action_handler_id,
            event: UiEvent::Action(ActionEvent {
                name: MirrorAction::name_for_type().to_string(),
                payload: Some(serde_json::json!({"source": "keyboard"})),
            }),
        })
        .expect("dispatch succeeds")
        .expect("message returned");

    assert!(matches!(
        rerender,
        PluginToHostMessage::RerenderRequested { .. }
    ));

    let rendered = session.render_if_dirty().expect("rerender succeeds");
    let PluginToHostMessage::Rendered { tree, .. } = rendered else {
        panic!("expected rendered tree");
    };
    assert_eq!(tree.children[0].text.as_deref(), Some("Actions: 1"));
}
