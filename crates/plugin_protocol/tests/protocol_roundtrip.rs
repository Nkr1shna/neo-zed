use gpui::rgb;
use plugin_protocol::{
    EventHandlerId, HostThemeSnapshot, HostToPlugin, PanelInstanceId, PluginId, PluginToHost,
    SerializedActionEvent, UiEvent, UiEventKind, UiNode, UiNodeKind, apply_ui_patches,
    diff_ui_trees,
};
use serde_json::json;
use theme::{Appearance, StatusColorsRefinement, ThemeColorsRefinement};

#[test]
fn host_to_plugin_theme_change_roundtrips_through_serde() {
    let theme = HostThemeSnapshot {
        id: "theme-id".to_string(),
        name: "One Dark".to_string(),
        appearance: Appearance::Dark,
        colors: ThemeColorsRefinement {
            text: Some(rgb(0xffffff).into()),
            border: Some(rgb(0x222222).into()),
            ..Default::default()
        },
        status: StatusColorsRefinement {
            error: Some(rgb(0xff0000).into()),
            success: Some(rgb(0x00ff00).into()),
            ..Default::default()
        },
    };
    let message = HostToPlugin::ThemeChanged {
        theme: theme.clone(),
    };

    let serialized = serde_json::to_value(&message).expect("theme message serializes");
    let decoded: HostToPlugin = serde_json::from_value(serialized).expect("theme message decodes");

    assert_eq!(decoded, message);
    assert_eq!(theme.id, "theme-id");
}

#[test]
fn host_to_plugin_action_event_roundtrips_through_serde() {
    let message = HostToPlugin::DispatchEvent {
        event: UiEvent {
            panel_instance_id: PanelInstanceId::new("panel-42"),
            handler_id: EventHandlerId::new("handler-42"),
            kind: UiEventKind::Action,
            payload: Some(json!(SerializedActionEvent {
                name: "plugin::DoThing".to_string(),
                payload: Some(json!({"source": "keyboard"})),
            })),
        },
    };

    let serialized = serde_json::to_string(&message).expect("action event serializes");
    let decoded: HostToPlugin = serde_json::from_str(&serialized).expect("action event decodes");

    assert_eq!(decoded, message);
}

#[test]
fn plugin_identity_wrappers_remain_serde_transparent() {
    let plugin_id = PluginId::new("acme.codex");
    let panel_instance_id = PanelInstanceId::new("panel-7");
    let handler_id = EventHandlerId::new("handler-7");

    let plugin_id_json = serde_json::to_value(&plugin_id).expect("plugin id serializes");
    let panel_id_json = serde_json::to_value(&panel_instance_id).expect("panel id serializes");
    let handler_id_json = serde_json::to_value(&handler_id).expect("handler id serializes");

    assert_eq!(plugin_id_json, json!("acme.codex"));
    assert_eq!(panel_id_json, json!("panel-7"));
    assert_eq!(handler_id_json, json!("handler-7"));
}

#[test]
fn render_delta_roundtrips_and_applies_to_existing_tree() {
    let previous =
        UiNode::new(UiNodeKind::Div).with_child(UiNode::new(UiNodeKind::Label).with_text("before"));
    let current =
        UiNode::new(UiNodeKind::Div).with_child(UiNode::new(UiNodeKind::Label).with_text("after"));
    let patches = diff_ui_trees(&previous, &current);
    let message = PluginToHost::RenderDelta {
        panel_id: "usage-widget".to_string(),
        panel_instance_id: PanelInstanceId::new("panel-9"),
        patches: patches.clone(),
    };

    let serialized = serde_json::to_value(&message).expect("render delta serializes");
    let decoded: PluginToHost = serde_json::from_value(serialized).expect("render delta decodes");

    assert_eq!(decoded, message);

    let mut patched = previous.clone();
    apply_ui_patches(&mut patched, &patches).expect("patches apply");
    assert_eq!(patched, current);
}
