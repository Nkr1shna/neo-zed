use std::collections::BTreeMap;
use std::fmt;

use gpui::{
    Bounds, Capslock, ClickEvent, KeyDownEvent, KeyUpEvent, KeyboardButton, Keystroke, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseClickEvent, MouseDownEvent, MouseMoveEvent,
    MousePressureEvent, MouseUpEvent, NavigationDirection, PinchEvent, Pixels, Point,
    PressureStage, ScrollDelta, ScrollWheelEvent, StyleRefinement, TouchPhase,
};
use serde::{Deserialize, Serialize};
use theme::{Appearance, StatusColorsRefinement, ThemeColorsRefinement};

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventHandlerId(String);

impl EventHandlerId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventHandlerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PanelInstanceId(String);

impl PanelInstanceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PanelInstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockPosition {
    Left,
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelActivation {
    OnDemand,
    OnStartup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginInstallState {
    Installed,
    Development,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelDescriptor {
    pub id: String,
    pub title: String,
    pub dock: DockPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    pub activation: PanelActivation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitlebarWidgetSide {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TitlebarWidgetDescriptor {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    #[serde(default = "default_titlebar_widget_side")]
    pub side: TitlebarWidgetSide,
    #[serde(default = "default_titlebar_widget_priority")]
    pub priority: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens_panel_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub id: PluginId,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub panels: Vec<PanelDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub titlebar_widgets: Vec<TitlebarWidgetDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StyleValue {
    Bool(bool),
    Number(f32),
    Text(String),
}

impl From<&str> for StyleValue {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

impl From<String> for StyleValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<bool> for StyleValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f32> for StyleValue {
    fn from(value: f32) -> Self {
        Self::Number(value)
    }
}

fn default_titlebar_widget_side() -> TitlebarWidgetSide {
    TitlebarWidgetSide::Right
}

fn default_titlebar_widget_priority() -> u32 {
    100
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiNodeKind {
    Empty,
    Div,
    Label,
    Button,
    Divider,
    Indicator,
    Icon,
    ProgressBar,
    MenuItem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiNodeEvent {
    pub event: UiEventKind,
    pub handler_id: EventHandlerId,
    #[serde(default)]
    pub phase: UiEventPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_button: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiNode {
    pub kind: UiNodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_default_style_refinement")]
    pub styles: StyleRefinement,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, StyleValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<UiNodeEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<UiNode>,
}

pub const INTERACTIVE_PROP_GROUP: &str = "interactive_group";
pub const INTERACTIVE_PROP_TAB_STOP: &str = "interactive_tab_stop";
pub const INTERACTIVE_PROP_TAB_INDEX: &str = "interactive_tab_index";
pub const INTERACTIVE_PROP_TAB_GROUP: &str = "interactive_tab_group";
pub const INTERACTIVE_PROP_FOCUSABLE: &str = "interactive_focusable";
pub const INTERACTIVE_PROP_KEY_CONTEXT: &str = "interactive_key_context";
pub const INTERACTIVE_PROP_WINDOW_CONTROL_AREA: &str = "interactive_window_control_area";
pub const INTERACTIVE_PROP_OCCLUDE: &str = "interactive_occlude";
pub const INTERACTIVE_PROP_BLOCK_MOUSE_EXCEPT_SCROLL: &str =
    "interactive_block_mouse_except_scroll";

impl UiNode {
    pub fn new(kind: UiNodeKind) -> Self {
        Self {
            kind,
            text: None,
            element_id: None,
            styles: StyleRefinement::default(),
            props: BTreeMap::new(),
            events: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self {
            kind: UiNodeKind::Label,
            text: Some(value.into()),
            ..Self::new(UiNodeKind::Label)
        }
    }

    pub fn with_text(mut self, value: impl Into<String>) -> Self {
        self.text = Some(value.into());
        self
    }

    pub fn with_element_id(mut self, value: impl Into<String>) -> Self {
        self.element_id = Some(value.into());
        self
    }

    pub fn with_style(mut self, refine: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.styles = refine(self.styles);
        self
    }

    pub fn with_prop(mut self, key: impl Into<String>, value: StyleValue) -> Self {
        self.props.insert(key.into(), value);
        self
    }

    pub fn with_event(mut self, event: UiEventKind, handler_id: EventHandlerId) -> Self {
        self.events.push(UiNodeEvent {
            event,
            handler_id,
            phase: UiEventPhase::Bubble,
            mouse_button: None,
            action_name: None,
        });
        self
    }

    pub fn with_child(mut self, child: UiNode) -> Self {
        self.children.push(child);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiPatch {
    pub path: Vec<usize>,
    pub node: UiNode,
}

pub fn diff_ui_trees(previous: &UiNode, current: &UiNode) -> Vec<UiPatch> {
    let mut patches = Vec::new();
    let mut path = Vec::new();
    diff_ui_trees_inner(previous, current, &mut path, &mut patches);
    patches
}

fn diff_ui_trees_inner(
    previous: &UiNode,
    current: &UiNode,
    path: &mut Vec<usize>,
    patches: &mut Vec<UiPatch>,
) {
    if previous.kind != current.kind
        || previous.text != current.text
        || previous.element_id != current.element_id
        || previous.styles != current.styles
        || previous.props != current.props
        || previous.events != current.events
        || previous.children.len() != current.children.len()
    {
        patches.push(UiPatch {
            path: path.clone(),
            node: current.clone(),
        });
        return;
    }

    for (index, (previous_child, current_child)) in previous
        .children
        .iter()
        .zip(current.children.iter())
        .enumerate()
    {
        path.push(index);
        diff_ui_trees_inner(previous_child, current_child, path, patches);
        path.pop();
    }
}

pub fn apply_ui_patches(root: &mut UiNode, patches: &[UiPatch]) -> Result<(), String> {
    for patch in patches {
        apply_ui_patch(root, patch)?;
    }
    Ok(())
}

fn apply_ui_patch(root: &mut UiNode, patch: &UiPatch) -> Result<(), String> {
    if patch.path.is_empty() {
        *root = patch.node.clone();
        return Ok(());
    }

    let mut node = root;
    for &index in &patch.path[..patch.path.len() - 1] {
        node = node
            .children
            .get_mut(index)
            .ok_or_else(|| format!("ui patch path segment {index} is out of bounds"))?;
    }

    let child_index = *patch
        .path
        .last()
        .expect("non-empty patch path should have a final segment");
    let Some(slot) = node.children.get_mut(child_index) else {
        return Err(format!(
            "ui patch child index {child_index} is out of bounds"
        ));
    };
    *slot = patch.node.clone();
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiEventPhase {
    Capture,
    #[default]
    Bubble,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiEventKind {
    Click,
    AuxClick,
    MouseDown,
    MouseUp,
    MouseMove,
    MousePressure,
    Hover,
    KeyDown,
    KeyUp,
    ModifiersChanged,
    ScrollWheel,
    Pinch,
    Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiEvent {
    pub panel_instance_id: PanelInstanceId,
    pub handler_id: EventHandlerId,
    pub kind: UiEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedKeystroke {
    pub modifiers: Modifiers,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_char: Option<String>,
}

impl From<Keystroke> for SerializedKeystroke {
    fn from(value: Keystroke) -> Self {
        Self {
            modifiers: value.modifiers,
            key: value.key,
            key_char: value.key_char,
        }
    }
}

impl From<SerializedKeystroke> for Keystroke {
    fn from(value: SerializedKeystroke) -> Self {
        Self {
            modifiers: value.modifiers,
            key: value.key,
            key_char: value.key_char,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedKeyboardButton {
    Enter,
    Space,
}

impl From<KeyboardButton> for SerializedKeyboardButton {
    fn from(value: KeyboardButton) -> Self {
        match value {
            KeyboardButton::Enter => Self::Enter,
            KeyboardButton::Space => Self::Space,
        }
    }
}

impl From<SerializedKeyboardButton> for KeyboardButton {
    fn from(value: SerializedKeyboardButton) -> Self {
        match value {
            SerializedKeyboardButton::Enter => Self::Enter,
            SerializedKeyboardButton::Space => Self::Space,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedNavigationDirection {
    Back,
    Forward,
}

impl From<NavigationDirection> for SerializedNavigationDirection {
    fn from(value: NavigationDirection) -> Self {
        match value {
            NavigationDirection::Back => Self::Back,
            NavigationDirection::Forward => Self::Forward,
        }
    }
}

impl From<SerializedNavigationDirection> for NavigationDirection {
    fn from(value: SerializedNavigationDirection) -> Self {
        match value {
            SerializedNavigationDirection::Back => Self::Back,
            SerializedNavigationDirection::Forward => Self::Forward,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedMouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl From<MouseButton> for SerializedMouseButton {
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => Self::Left,
            MouseButton::Right => Self::Right,
            MouseButton::Middle => Self::Middle,
            MouseButton::Navigate(NavigationDirection::Back) => Self::Back,
            MouseButton::Navigate(NavigationDirection::Forward) => Self::Forward,
        }
    }
}

impl From<SerializedMouseButton> for MouseButton {
    fn from(value: SerializedMouseButton) -> Self {
        match value {
            SerializedMouseButton::Left => Self::Left,
            SerializedMouseButton::Right => Self::Right,
            SerializedMouseButton::Middle => Self::Middle,
            SerializedMouseButton::Back => Self::Navigate(NavigationDirection::Back),
            SerializedMouseButton::Forward => Self::Navigate(NavigationDirection::Forward),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedTouchPhase {
    Started,
    Moved,
    Ended,
}

impl From<TouchPhase> for SerializedTouchPhase {
    fn from(value: TouchPhase) -> Self {
        match value {
            TouchPhase::Started => Self::Started,
            TouchPhase::Moved => Self::Moved,
            TouchPhase::Ended => Self::Ended,
        }
    }
}

impl From<SerializedTouchPhase> for TouchPhase {
    fn from(value: SerializedTouchPhase) -> Self {
        match value {
            SerializedTouchPhase::Started => Self::Started,
            SerializedTouchPhase::Moved => Self::Moved,
            SerializedTouchPhase::Ended => Self::Ended,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum SerializedScrollDelta {
    Pixels(Point<Pixels>),
    Lines(Point<f32>),
}

impl From<ScrollDelta> for SerializedScrollDelta {
    fn from(value: ScrollDelta) -> Self {
        match value {
            ScrollDelta::Pixels(delta) => Self::Pixels(delta),
            ScrollDelta::Lines(delta) => Self::Lines(delta),
        }
    }
}

impl From<SerializedScrollDelta> for ScrollDelta {
    fn from(value: SerializedScrollDelta) -> Self {
        match value {
            SerializedScrollDelta::Pixels(delta) => Self::Pixels(delta),
            SerializedScrollDelta::Lines(delta) => Self::Lines(delta),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedPressureStage {
    Zero,
    Normal,
    Force,
}

impl From<PressureStage> for SerializedPressureStage {
    fn from(value: PressureStage) -> Self {
        match value {
            PressureStage::Zero => Self::Zero,
            PressureStage::Normal => Self::Normal,
            PressureStage::Force => Self::Force,
        }
    }
}

impl From<SerializedPressureStage> for PressureStage {
    fn from(value: SerializedPressureStage) -> Self {
        match value {
            SerializedPressureStage::Zero => Self::Zero,
            SerializedPressureStage::Normal => Self::Normal,
            SerializedPressureStage::Force => Self::Force,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedMouseDownEvent {
    pub button: SerializedMouseButton,
    pub position: Point<Pixels>,
    pub modifiers: Modifiers,
    pub click_count: usize,
    pub first_mouse: bool,
}

impl From<&MouseDownEvent> for SerializedMouseDownEvent {
    fn from(value: &MouseDownEvent) -> Self {
        Self {
            button: value.button.into(),
            position: value.position,
            modifiers: value.modifiers,
            click_count: value.click_count,
            first_mouse: value.first_mouse,
        }
    }
}

impl From<SerializedMouseDownEvent> for MouseDownEvent {
    fn from(value: SerializedMouseDownEvent) -> Self {
        Self {
            button: value.button.into(),
            position: value.position,
            modifiers: value.modifiers,
            click_count: value.click_count,
            first_mouse: value.first_mouse,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedMouseUpEvent {
    pub button: SerializedMouseButton,
    pub position: Point<Pixels>,
    pub modifiers: Modifiers,
    pub click_count: usize,
}

impl From<&MouseUpEvent> for SerializedMouseUpEvent {
    fn from(value: &MouseUpEvent) -> Self {
        Self {
            button: value.button.into(),
            position: value.position,
            modifiers: value.modifiers,
            click_count: value.click_count,
        }
    }
}

impl From<SerializedMouseUpEvent> for MouseUpEvent {
    fn from(value: SerializedMouseUpEvent) -> Self {
        Self {
            button: value.button.into(),
            position: value.position,
            modifiers: value.modifiers,
            click_count: value.click_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedMouseMoveEvent {
    pub position: Point<Pixels>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed_button: Option<SerializedMouseButton>,
    pub modifiers: Modifiers,
}

impl From<&MouseMoveEvent> for SerializedMouseMoveEvent {
    fn from(value: &MouseMoveEvent) -> Self {
        Self {
            position: value.position,
            pressed_button: value.pressed_button.map(Into::into),
            modifiers: value.modifiers,
        }
    }
}

impl From<SerializedMouseMoveEvent> for MouseMoveEvent {
    fn from(value: SerializedMouseMoveEvent) -> Self {
        Self {
            position: value.position,
            pressed_button: value.pressed_button.map(Into::into),
            modifiers: value.modifiers,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedMousePressureEvent {
    pub pressure: f32,
    pub stage: SerializedPressureStage,
    pub position: Point<Pixels>,
    pub modifiers: Modifiers,
}

impl From<&MousePressureEvent> for SerializedMousePressureEvent {
    fn from(value: &MousePressureEvent) -> Self {
        Self {
            pressure: value.pressure,
            stage: value.stage.into(),
            position: value.position,
            modifiers: value.modifiers,
        }
    }
}

impl From<SerializedMousePressureEvent> for MousePressureEvent {
    fn from(value: SerializedMousePressureEvent) -> Self {
        Self {
            pressure: value.pressure,
            stage: value.stage.into(),
            position: value.position,
            modifiers: value.modifiers,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedKeyDownEvent {
    pub keystroke: SerializedKeystroke,
    pub is_held: bool,
    pub prefer_character_input: bool,
}

impl From<&KeyDownEvent> for SerializedKeyDownEvent {
    fn from(value: &KeyDownEvent) -> Self {
        Self {
            keystroke: value.keystroke.clone().into(),
            is_held: value.is_held,
            prefer_character_input: value.prefer_character_input,
        }
    }
}

impl From<SerializedKeyDownEvent> for KeyDownEvent {
    fn from(value: SerializedKeyDownEvent) -> Self {
        Self {
            keystroke: value.keystroke.into(),
            is_held: value.is_held,
            prefer_character_input: value.prefer_character_input,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedKeyUpEvent {
    pub keystroke: SerializedKeystroke,
}

impl From<&KeyUpEvent> for SerializedKeyUpEvent {
    fn from(value: &KeyUpEvent) -> Self {
        Self {
            keystroke: value.keystroke.clone().into(),
        }
    }
}

impl From<SerializedKeyUpEvent> for KeyUpEvent {
    fn from(value: SerializedKeyUpEvent) -> Self {
        Self {
            keystroke: value.keystroke.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedModifiersChangedEvent {
    pub modifiers: Modifiers,
    pub capslock: Capslock,
}

impl From<&ModifiersChangedEvent> for SerializedModifiersChangedEvent {
    fn from(value: &ModifiersChangedEvent) -> Self {
        Self {
            modifiers: value.modifiers,
            capslock: value.capslock,
        }
    }
}

impl From<SerializedModifiersChangedEvent> for ModifiersChangedEvent {
    fn from(value: SerializedModifiersChangedEvent) -> Self {
        Self {
            modifiers: value.modifiers,
            capslock: value.capslock,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedScrollWheelEvent {
    pub position: Point<Pixels>,
    pub delta: SerializedScrollDelta,
    pub modifiers: Modifiers,
    pub touch_phase: SerializedTouchPhase,
}

impl From<&ScrollWheelEvent> for SerializedScrollWheelEvent {
    fn from(value: &ScrollWheelEvent) -> Self {
        Self {
            position: value.position,
            delta: value.delta.into(),
            modifiers: value.modifiers,
            touch_phase: value.touch_phase.into(),
        }
    }
}

impl From<SerializedScrollWheelEvent> for ScrollWheelEvent {
    fn from(value: SerializedScrollWheelEvent) -> Self {
        Self {
            position: value.position,
            delta: value.delta.into(),
            modifiers: value.modifiers,
            touch_phase: value.touch_phase.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedPinchEvent {
    pub position: Point<Pixels>,
    pub delta: f32,
    pub modifiers: Modifiers,
    pub phase: SerializedTouchPhase,
}

impl From<&PinchEvent> for SerializedPinchEvent {
    fn from(value: &PinchEvent) -> Self {
        Self {
            position: value.position,
            delta: value.delta,
            modifiers: value.modifiers,
            phase: value.phase.into(),
        }
    }
}

impl From<SerializedPinchEvent> for PinchEvent {
    fn from(value: SerializedPinchEvent) -> Self {
        Self {
            position: value.position,
            delta: value.delta,
            modifiers: value.modifiers,
            phase: value.phase.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SerializedClickEvent {
    Mouse {
        down: SerializedMouseDownEvent,
        up: SerializedMouseUpEvent,
    },
    Keyboard {
        button: SerializedKeyboardButton,
        bounds: Bounds<Pixels>,
    },
}

impl From<&ClickEvent> for SerializedClickEvent {
    fn from(value: &ClickEvent) -> Self {
        match value {
            ClickEvent::Mouse(event) => Self::Mouse {
                down: (&event.down).into(),
                up: (&event.up).into(),
            },
            ClickEvent::Keyboard(event) => Self::Keyboard {
                button: event.button.into(),
                bounds: event.bounds,
            },
        }
    }
}

impl From<SerializedClickEvent> for ClickEvent {
    fn from(value: SerializedClickEvent) -> Self {
        match value {
            SerializedClickEvent::Mouse { down, up } => ClickEvent::Mouse(MouseClickEvent {
                down: down.into(),
                up: up.into(),
            }),
            SerializedClickEvent::Keyboard { button, bounds } => {
                ClickEvent::Keyboard(gpui::KeyboardClickEvent {
                    button: button.into(),
                    bounds,
                })
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedActionEvent {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostThemeSnapshot {
    pub id: String,
    pub name: String,
    pub appearance: Appearance,
    pub colors: ThemeColorsRefinement,
    pub status: StatusColorsRefinement,
}

impl PartialEq for HostThemeSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.appearance == other.appearance
            // The refinement types do not currently implement `PartialEq`, so compare the
            // serialized shapes and treat serialization failure as inequality.
            && serialized_value_eq(&self.colors, &other.colors)
            && serialized_value_eq(&self.status, &other.status)
    }
}

fn serialized_value_eq<T: Serialize>(left: &T, right: &T) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left_value), Ok(right_value)) => left_value == right_value,
        _ => false,
    }
}

fn is_default_style_refinement(style: &StyleRefinement) -> bool {
    *style == StyleRefinement::default()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginHostRequest {
    SecureStorageLoad { key: String },
    SecureStorageStore { key: String, value: String },
    SecureStorageClear { key: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginHostResponse {
    SecureStorageLoad { value: Option<String> },
    SecureStorageStore,
    SecureStorageClear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginToHost {
    Register {
        plugin: PluginMetadata,
    },
    HostRequest {
        request_id: u64,
        request: PluginHostRequest,
    },
    Render {
        panel_id: String,
        panel_instance_id: PanelInstanceId,
        root: UiNode,
    },
    RenderDelta {
        panel_id: String,
        panel_instance_id: PanelInstanceId,
        patches: Vec<UiPatch>,
    },
    ClosePanel {
        panel_instance_id: PanelInstanceId,
    },
    ReportError {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        panel_instance_id: Option<PanelInstanceId>,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToPlugin {
    OpenPanel {
        panel_id: String,
        panel_instance_id: PanelInstanceId,
        theme: HostThemeSnapshot,
    },
    ThemeChanged {
        theme: HostThemeSnapshot,
    },
    DispatchEvent {
        event: UiEvent,
    },
    ClosePanel {
        panel_instance_id: PanelInstanceId,
    },
    HostResponse {
        request_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response: Option<PluginHostResponse>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Shutdown,
}
