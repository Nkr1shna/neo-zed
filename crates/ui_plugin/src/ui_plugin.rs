use gpui_api::{
    Action, ActiveTheme, AnyView, App, ClickEvent, CursorStyle, DefiniteLength, Element, ElementId,
    EventPhase, FluentBuilder, FocusHandle, FontWeight, InteractiveElement, Interactivity,
    RenderContext, SharedString, StyleMap, Styled, UiEvent, UiEventKind, UiNode, UiNodeKind,
    Window,
};
use strum::{Display, EnumString};
pub use ui::{
    ButtonSize, ButtonStyle, Color, DividerColor, ElevationIndex, HeadlineSize, IconButtonShape,
    IconName, IconPosition, IconSize, KeybindingPosition, LabelSize, LineHeightStyle,
    PlatformStyle, Severity, SwitchColor, SwitchLabelPosition, TintColor, ToggleState, ToggleStyle,
};
pub use ui_macros::RegisterComponent;

pub mod component_prelude {
    pub use crate::{
        Component, ComponentId, ComponentScope, ComponentStatus, RegisterComponent, example_group,
        example_group_with_title, single_example,
    };
}

pub mod prelude {
    pub use crate::component_prelude::*;
    pub use crate::{
        AnimationDirection, AnimationDuration, AnyIcon, Button, ButtonCommon, ButtonLike,
        ButtonLink, ButtonSize, ButtonStyle, Checkbox, CircularProgress, Clickable, Color,
        Component, ComponentId, ComponentScope, ComponentStatus, CopyButton, DefaultAnimations,
        Disableable, Divider, DividerColor, DynamicSpacing, ElevationIndex, FixedWidth, Headline,
        HeadlineSize, Icon, IconButton, IconButtonShape, IconName, IconPosition, IconSize,
        IconWithIndicator, Indicator, KeyBinding, KeybindingHint, KeybindingPosition, Label,
        LabelCommon, LabelSize, LineHeightStyle, LoadingLabel, MenuItem, PlatformStyle,
        ProgressBar, SelectableButton, Severity, SplitButton, SplitButtonKind, SplitButtonStyle,
        StyledExt, StyledTypography, Switch, SwitchColor, SwitchField, SwitchLabelPosition,
        TextSize, TintColor, ToggleState, ToggleStyle, Toggleable, VisibleOnHover, checkbox,
        divider, example_group, example_group_with_title, h_flex, h_group, h_group_lg, h_group_sm,
        h_group_xl, rems_from_px, single_example, switch, v_flex, v_group, v_group_lg, v_group_sm,
        v_group_xl, vertical_divider, vh, vw,
    };
    pub use gpui_plugin::prelude::*;
    pub use gpui_plugin::{
        AbsoluteLength, AnyElement, App, Context, DefiniteLength, Div, Element, ElementId, Pixels,
        SharedString, Window, div, px, relative, rems,
    };
}

pub mod utils {
    pub use ui::utils::*;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationDuration {
    Instant = 50,
    Fast = 150,
    Slow = 300,
}

impl AnimationDuration {
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(*self as u64)
    }
}

impl From<AnimationDuration> for std::time::Duration {
    fn from(value: AnimationDuration) -> Self {
        value.duration()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationDirection {
    FromBottom,
    FromLeft,
    FromRight,
    FromTop,
}

mod transformable {
    use gpui_plugin::Transformation;

    pub trait Transformable {
        fn transform(self, transformation: Transformation) -> Self;
    }
}

pub trait CommonAnimationExt: gpui_plugin::AnimationExt {
    #[track_caller]
    fn with_rotate_animation(self, duration: u64) -> gpui_plugin::AnimationElement<Self>
    where
        Self: transformable::Transformable + Sized,
    {
        self.with_keyed_rotate_animation(
            ElementId::from(format!(
                "rotate-animation:{}:{}",
                std::panic::Location::caller().file(),
                std::panic::Location::caller().line()
            )),
            duration,
        )
    }

    fn with_keyed_rotate_animation(
        self,
        id: impl Into<ElementId>,
        duration: u64,
    ) -> gpui_plugin::AnimationElement<Self>
    where
        Self: transformable::Transformable + Sized,
    {
        gpui_plugin::AnimationExt::with_animation(
            self,
            id,
            gpui_plugin::Animation::new(std::time::Duration::from_secs(duration)).repeat(),
            |component, delta| {
                component.transform(gpui_plugin::Transformation::rotate(gpui_plugin::radians(
                    delta,
                )))
            },
        )
    }
}

impl<T: gpui_plugin::AnimationExt> CommonAnimationExt for T {}

pub trait DefaultAnimations: Styled + Sized + Element + gpui_plugin::AnimationExt {
    fn animate_in(
        self,
        animation_type: AnimationDirection,
        fade_in: bool,
    ) -> gpui_plugin::AnimationElement<Self> {
        let animation_name = match animation_type {
            AnimationDirection::FromBottom => "animate_from_bottom",
            AnimationDirection::FromLeft => "animate_from_left",
            AnimationDirection::FromRight => "animate_from_right",
            AnimationDirection::FromTop => "animate_from_top",
        };

        gpui_plugin::AnimationExt::with_animation(
            self,
            animation_name,
            gpui_plugin::Animation::new(AnimationDuration::Fast.into()),
            move |mut this, delta| {
                let start_opacity = 0.4;
                let start_pos = 0.0;
                let end_pos = 40.0;

                if fade_in {
                    this = this.opacity(start_opacity + delta * (1.0 - start_opacity));
                }

                match animation_type {
                    AnimationDirection::FromBottom => {
                        this.bottom(gpui_plugin::px(start_pos + delta * (end_pos - start_pos)))
                    }
                    AnimationDirection::FromLeft => {
                        this.left(gpui_plugin::px(start_pos + delta * (end_pos - start_pos)))
                    }
                    AnimationDirection::FromRight => {
                        this.right(gpui_plugin::px(start_pos + delta * (end_pos - start_pos)))
                    }
                    AnimationDirection::FromTop => {
                        this.top(gpui_plugin::px(start_pos + delta * (end_pos - start_pos)))
                    }
                }
            },
        )
    }

    fn animate_in_from_bottom(self, fade: bool) -> gpui_plugin::AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromBottom, fade)
    }

    fn animate_in_from_left(self, fade: bool) -> gpui_plugin::AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromLeft, fade)
    }

    fn animate_in_from_right(self, fade: bool) -> gpui_plugin::AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromRight, fade)
    }

    fn animate_in_from_top(self, fade: bool) -> gpui_plugin::AnimationElement<Self> {
        self.animate_in(AnimationDirection::FromTop, fade)
    }
}

impl<E: Styled + Sized + Element + gpui_plugin::AnimationExt> DefaultAnimations for E {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ComponentId(pub &'static str);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Display, EnumString)]
pub enum ComponentStatus {
    #[strum(serialize = "Work In Progress")]
    WorkInProgress,
    #[strum(serialize = "Ready To Build")]
    EngineeringReady,
    Live,
    Deprecated,
}

impl ComponentStatus {
    pub fn description(&self) -> &str {
        match self {
            Self::WorkInProgress => {
                "These components are still being designed or refined. They shouldn't be used in the app yet."
            }
            Self::EngineeringReady => {
                "These components are design complete or partially implemented, and are ready for an engineer to complete their implementation."
            }
            Self::Live => "These components are ready for use in the app.",
            Self::Deprecated => {
                "These components are no longer recommended for use in the app, and may be removed in a future release."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Display, EnumString)]
pub enum ComponentScope {
    Agent,
    Collaboration,
    #[strum(serialize = "Data Display")]
    DataDisplay,
    Editor,
    #[strum(serialize = "Images & Icons")]
    Images,
    #[strum(serialize = "Forms & Input")]
    Input,
    #[strum(serialize = "Layout & Structure")]
    Layout,
    #[strum(serialize = "Loading & Progress")]
    Loading,
    Navigation,
    #[strum(serialize = "Unsorted")]
    None,
    Notification,
    #[strum(serialize = "Overlays & Layering")]
    Overlays,
    Onboarding,
    Status,
    Typography,
    Utilities,
    #[strum(serialize = "Version Control")]
    VersionControl,
}

pub trait Component {
    fn id() -> ComponentId {
        ComponentId(Self::name())
    }

    fn scope() -> ComponentScope {
        ComponentScope::None
    }

    fn status() -> ComponentStatus {
        ComponentStatus::Live
    }

    fn name() -> &'static str {
        std::any::type_name::<Self>()
    }

    fn sort_name() -> &'static str {
        Self::name()
    }

    fn description() -> Option<&'static str> {
        None
    }

    fn preview(_window: &mut Window, _cx: &mut App) -> Option<gpui_plugin::AnyElement> {
        None
    }
}

pub struct ComponentExample {
    variant_name: SharedString,
    description: Option<SharedString>,
    element: gpui_plugin::AnyElement,
    width: Option<gpui_api::Pixels>,
}

impl ComponentExample {
    pub fn new(variant_name: impl Into<SharedString>, element: gpui_plugin::AnyElement) -> Self {
        Self {
            variant_name: variant_name.into(),
            description: None,
            element,
            width: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn width(mut self, width: gpui_api::Pixels) -> Self {
        self.width = Some(width);
        self
    }
}

impl gpui_plugin::RenderOnce for ComponentExample {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl gpui_plugin::IntoElement {
        gpui_plugin::div()
            .pt_2()
            .map(|this| {
                if let Some(width) = self.width {
                    this.w(width)
                } else {
                    this.w_full()
                }
            })
            .flex()
            .flex_col()
            .gap_3()
            .child(
                gpui_plugin::div()
                    .flex()
                    .flex_col()
                    .child(
                        gpui_plugin::div()
                            .child(self.variant_name.clone())
                            .text_size(gpui_api::rems(1.0))
                            .text_color(cx.theme().colors().text),
                    )
                    .when_some(self.description, |this, description| {
                        this.child(
                            gpui_plugin::div()
                                .text_size(gpui_api::rems(0.875))
                                .text_color(cx.theme().colors().text_muted)
                                .child(description),
                        )
                    }),
            )
            .child(
                gpui_plugin::div()
                    .min_h(gpui_api::px(100.))
                    .w_full()
                    .p_8()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_xl()
                    .border_1()
                    .border_color(cx.theme().colors().border.opacity(0.5))
                    .bg(cx.theme().colors().surface_background.opacity(0.25))
                    .map(|mut this| {
                        gpui_api::ParentElement::extend(&mut this, std::iter::once(self.element));
                        this
                    }),
            )
    }
}

impl gpui_plugin::IntoElement for ComponentExample {
    type Element = gpui_plugin::Component<Self>;

    fn into_element(self) -> Self::Element {
        gpui_plugin::Component::new(self)
    }
}

pub struct ComponentExampleGroup {
    title: Option<SharedString>,
    examples: Vec<ComponentExample>,
    width: Option<gpui_api::Pixels>,
    grow: bool,
    vertical: bool,
}

impl ComponentExampleGroup {
    pub fn new(examples: Vec<ComponentExample>) -> Self {
        Self {
            title: None,
            examples,
            width: None,
            grow: false,
            vertical: false,
        }
    }

    pub fn with_title(title: impl Into<SharedString>, examples: Vec<ComponentExample>) -> Self {
        Self {
            title: Some(title.into()),
            examples,
            width: None,
            grow: false,
            vertical: false,
        }
    }

    pub fn width(mut self, width: gpui_api::Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn grow(mut self) -> Self {
        self.grow = true;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }
}

impl gpui_plugin::RenderOnce for ComponentExampleGroup {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl gpui_plugin::IntoElement {
        gpui_plugin::div()
            .flex_col()
            .text_sm()
            .text_color(cx.theme().colors().text_muted)
            .map(|this| {
                let this = if let Some(width) = self.width {
                    this.w(width)
                } else {
                    this.w_full()
                };
                if self.grow { this.flex_1() } else { this }
            })
            .when_some(self.title, |this, title| {
                this.gap_4().child(
                    gpui_plugin::div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .mt_4()
                        .mb_1()
                        .child(
                            gpui_plugin::div()
                                .flex_none()
                                .text_size(gpui_api::px(10.))
                                .child(title.to_uppercase()),
                        )
                        .child(
                            gpui_plugin::div()
                                .h_px()
                                .w_full()
                                .flex_1()
                                .bg(cx.theme().colors().border),
                        ),
                )
            })
            .child({
                let container = if self.vertical {
                    gpui_plugin::div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .w_full()
                        .gap_6()
                } else {
                    gpui_plugin::div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .w_full()
                        .gap_6()
                };
                container.children(self.examples)
            })
    }
}

impl gpui_plugin::IntoElement for ComponentExampleGroup {
    type Element = gpui_plugin::Component<Self>;

    fn into_element(self) -> Self::Element {
        gpui_plugin::Component::new(self)
    }
}

pub fn single_example(
    variant_name: impl Into<SharedString>,
    example: gpui_plugin::AnyElement,
) -> ComponentExample {
    ComponentExample::new(variant_name, example)
}

pub fn example_group(examples: Vec<ComponentExample>) -> ComponentExampleGroup {
    ComponentExampleGroup::new(examples)
}

pub fn example_group_with_title(
    title: impl Into<SharedString>,
    examples: Vec<ComponentExample>,
) -> ComponentExampleGroup {
    ComponentExampleGroup::with_title(title, examples)
}

pub trait Clickable {
    fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self;
    fn cursor_style(self, cursor_style: CursorStyle) -> Self;
}

pub trait Disableable {
    fn disabled(self, disabled: bool) -> Self;
}

pub trait Toggleable {
    fn toggle_state(self, selected: bool) -> Self;
}

pub trait SelectableButton: Toggleable {
    fn selected_style(self, style: ButtonStyle) -> Self;
}

pub trait VisibleOnHover {
    fn visible_on_hover(self, group_name: impl Into<SharedString>) -> Self;
}

pub trait StyledTypography: Styled + Sized {
    fn text_ui_size(self, size: TextSize, cx: &App) -> Self {
        self.text_size(size.rems(cx))
    }

    fn text_ui_lg(self, cx: &App) -> Self {
        self.text_size(TextSize::Large.rems(cx))
    }

    fn text_ui(self, cx: &App) -> Self {
        self.text_size(TextSize::Default.rems(cx))
    }

    fn text_ui_sm(self, cx: &App) -> Self {
        self.text_size(TextSize::Small.rems(cx))
    }

    fn text_ui_xs(self, cx: &App) -> Self {
        self.text_size(TextSize::XSmall.rems(cx))
    }
}

pub trait StyledExt: Styled + Sized {
    fn h_flex(self) -> Self {
        self.flex().flex_row().items_center()
    }

    fn v_flex(self) -> Self {
        self.flex().flex_col()
    }

    fn elevation_1(self, cx: &App) -> Self {
        elevated(self, cx, ElevationIndex::Surface)
    }

    fn elevation_1_borderless(self, cx: &mut App) -> Self {
        elevated_borderless(self, cx, ElevationIndex::Surface)
    }

    fn elevation_2(self, cx: &App) -> Self {
        elevated(self, cx, ElevationIndex::ElevatedSurface)
    }

    fn elevation_2_borderless(self, cx: &mut App) -> Self {
        elevated_borderless(self, cx, ElevationIndex::ElevatedSurface)
    }

    fn elevation_3(self, cx: &App) -> Self {
        elevated(self, cx, ElevationIndex::ModalSurface)
    }

    fn elevation_3_borderless(self, cx: &mut App) -> Self {
        elevated_borderless(self, cx, ElevationIndex::ModalSurface)
    }

    fn border_primary(self, cx: &mut App) -> Self {
        self.border_color(cx.theme().colors().border)
    }

    fn border_muted(self, cx: &mut App) -> Self {
        self.border_color(cx.theme().colors().border_variant)
    }

    fn debug_bg_red(self) -> Self {
        self.bg(gpui_api::hsla(0.0, 1.0, 0.5, 1.0))
    }

    fn debug_bg_green(self) -> Self {
        self.bg(gpui_api::hsla(120.0 / 360.0, 1.0, 0.5, 1.0))
    }

    fn debug_bg_blue(self) -> Self {
        self.bg(gpui_api::hsla(240.0 / 360.0, 1.0, 0.5, 1.0))
    }

    fn debug_bg_yellow(self) -> Self {
        self.bg(gpui_api::hsla(60.0 / 360.0, 1.0, 0.5, 1.0))
    }

    fn debug_bg_cyan(self) -> Self {
        self.bg(gpui_api::hsla(160.0 / 360.0, 1.0, 0.5, 1.0))
    }

    fn debug_bg_magenta(self) -> Self {
        self.bg(gpui_api::hsla(300.0 / 360.0, 1.0, 0.5, 1.0))
    }
}

pub trait FixedWidth {
    fn width(self, width: impl Into<DefiniteLength>) -> Self;
    fn full_width(self) -> Self;
}

pub trait ButtonCommon: Clickable + Disableable {
    fn id(&self) -> &ElementId;
    fn style(self, style: ButtonStyle) -> Self;
    fn size(self, size: ButtonSize) -> Self;
    fn tooltip(self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self;
    fn tab_index(self, tab_index: impl Into<isize>) -> Self;
    fn layer(self, elevation: ElevationIndex) -> Self;
    fn track_focus(self, focus_handle: &FocusHandle) -> Self;
}

pub trait LabelCommon {
    fn size(self, size: LabelSize) -> Self;
    fn weight(self, weight: FontWeight) -> Self;
    fn line_height_style(self, line_height_style: LineHeightStyle) -> Self;
    fn color(self, color: Color) -> Self;
    fn strikethrough(self) -> Self;
    fn italic(self) -> Self;
    fn underline(self) -> Self;
    fn alpha(self, alpha: f32) -> Self;
    fn truncate(self) -> Self;
    fn single_line(self) -> Self;
    fn buffer_font(self, cx: &App) -> Self;
    fn inline_code(self, cx: &App) -> Self;
}

#[derive(Default)]
pub struct Label {
    text: SharedString,
    styles: StyleMap,
    label_size: Option<LabelSize>,
    font_weight: Option<FontWeight>,
    label_color: Option<Color>,
    line_height_style: Option<LineHeightStyle>,
    strikethrough: bool,
    italic: bool,
    underline: bool,
    alpha: Option<f32>,
}

impl Label {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            styles: StyleMap::default(),
            label_size: None,
            font_weight: None,
            label_color: None,
            line_height_style: None,
            strikethrough: false,
            italic: false,
            underline: false,
            alpha: None,
        }
    }

    pub fn size(self, size: LabelSize) -> Self {
        LabelCommon::size(self, size)
    }

    pub fn weight(self, weight: FontWeight) -> Self {
        LabelCommon::weight(self, weight)
    }

    pub fn line_height_style(self, line_height_style: LineHeightStyle) -> Self {
        LabelCommon::line_height_style(self, line_height_style)
    }

    pub fn color(self, color: Color) -> Self {
        LabelCommon::color(self, color)
    }

    pub fn strikethrough(self) -> Self {
        LabelCommon::strikethrough(self)
    }

    pub fn italic(self) -> Self {
        LabelCommon::italic(self)
    }

    pub fn underline(self) -> Self {
        LabelCommon::underline(self)
    }

    pub fn alpha(self, alpha: f32) -> Self {
        LabelCommon::alpha(self, alpha)
    }

    pub fn truncate(self) -> Self {
        LabelCommon::truncate(self)
    }

    pub fn single_line(self) -> Self {
        LabelCommon::single_line(self)
    }

    pub fn buffer_font(self, cx: &App) -> Self {
        LabelCommon::buffer_font(self, cx)
    }

    pub fn inline_code(self, cx: &App) -> Self {
        LabelCommon::inline_code(self, cx)
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>) {
        self.text = text.into();
    }

    pub fn truncate_start(self) -> Self {
        self.whitespace_nowrap().text_ellipsis_start()
    }

    pub fn flex_1(mut self) -> Self {
        self.styles.flex_grow = Some(1.);
        self.styles.flex_shrink = Some(1.);
        self.styles.flex_basis = Some(gpui_api::relative(0.).into());
        self
    }

    pub fn flex_none(mut self) -> Self {
        self.styles.flex_grow = Some(0.);
        self.styles.flex_shrink = Some(0.);
        self
    }

    pub fn flex_grow(mut self) -> Self {
        self.styles.flex_grow = Some(1.);
        self
    }

    pub fn flex_shrink(mut self) -> Self {
        self.styles.flex_shrink = Some(1.);
        self
    }

    pub fn flex_shrink_0(mut self) -> Self {
        self.styles.flex_shrink = Some(0.);
        self
    }
}

impl LabelCommon for Label {
    fn size(mut self, size: LabelSize) -> Self {
        self.label_size = Some(size);
        self
    }

    fn weight(mut self, weight: FontWeight) -> Self {
        self.font_weight = Some(weight);
        self
    }

    fn line_height_style(mut self, line_height_style: LineHeightStyle) -> Self {
        self.line_height_style = Some(line_height_style);
        self
    }

    fn color(mut self, color: Color) -> Self {
        self.label_color = Some(color);
        self
    }

    fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }

    fn truncate(self) -> Self {
        self.whitespace_nowrap().text_ellipsis()
    }

    fn single_line(self) -> Self {
        self.whitespace_nowrap()
    }

    fn buffer_font(self, _cx: &App) -> Self {
        self
    }

    fn inline_code(self, cx: &App) -> Self {
        self.bg(cx.theme().colors().element_background)
            .rounded_sm()
            .px_0p5()
    }
}

impl Element for Label {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Label);
        node.text = Some(self.text.to_string());
        node.styles = self.styles;
        if let Some(size) = self.label_size {
            node.props
                .insert("label_size".to_string(), serialize_label_size(size).into());
        }
        if let Some(weight) = self.font_weight {
            node.props
                .insert("font_weight".to_string(), weight.0.into());
        }
        if let Some(color) = self.label_color {
            node.props
                .insert("label_color".to_string(), serialize_color(color).into());
        }
        if let Some(line_height_style) = self.line_height_style {
            node.props.insert(
                "line_height_style".to_string(),
                serialize_line_height_style(line_height_style).into(),
            );
        }
        if self.strikethrough {
            node.props.insert("strikethrough".to_string(), true.into());
        }
        if self.italic {
            node.props.insert("italic".to_string(), true.into());
        }
        if self.underline {
            node.props.insert("underline".to_string(), true.into());
        }
        if let Some(alpha) = self.alpha {
            node.props.insert("alpha".to_string(), alpha.into());
        }
        node
    }
}

impl Styled for Label {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

#[derive(Default)]
pub struct Icon {
    name: String,
    styles: StyleMap,
    icon_size: Option<IconSize>,
    icon_color: Option<Color>,
    transformation: Option<gpui_plugin::Transformation>,
}

impl Icon {
    pub fn new(name: impl Into<IconDescriptor>) -> Self {
        Self {
            name: name.into().0,
            styles: StyleMap::default(),
            icon_size: None,
            icon_color: None,
            transformation: None,
        }
    }

    pub fn size(mut self, size: IconSize) -> Self {
        self.icon_size = Some(size);
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.icon_color = Some(color);
        self
    }

    fn icon_name(&self) -> Option<IconName> {
        self.name.parse().ok()
    }
}

impl Element for Icon {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Icon);
        node.text = Some(self.name);
        node.styles = self.styles;
        if let Some(size) = self.icon_size {
            node.props
                .insert("icon_size".to_string(), serialize_icon_size(size).into());
        }
        if let Some(color) = self.icon_color {
            node.props
                .insert("icon_color".to_string(), serialize_color(color).into());
        }
        if self.transformation.is_some() {
            node.props
                .insert("icon_transformation".to_string(), true.into());
        }
        node
    }
}

impl Styled for Icon {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

impl transformable::Transformable for Icon {
    fn transform(mut self, transformation: gpui_plugin::Transformation) -> Self {
        self.transformation = Some(transformation);
        self
    }
}

pub enum AnyIcon {
    Icon(Icon),
    AnimatedIcon(gpui_plugin::AnimationElement<Icon>),
}

impl AnyIcon {
    pub fn map(self, f: impl FnOnce(Icon) -> Icon) -> Self {
        match self {
            Self::Icon(icon) => Self::Icon(f(icon)),
            Self::AnimatedIcon(icon) => Self::AnimatedIcon(icon.map_element(f)),
        }
    }
}

impl From<Icon> for AnyIcon {
    fn from(value: Icon) -> Self {
        Self::Icon(value)
    }
}

impl From<gpui_plugin::AnimationElement<Icon>> for AnyIcon {
    fn from(value: gpui_plugin::AnimationElement<Icon>) -> Self {
        Self::AnimatedIcon(value)
    }
}

impl Element for AnyIcon {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        match self {
            Self::Icon(icon) => icon.into_node(context),
            Self::AnimatedIcon(icon) => icon.into_node(context),
        }
    }
}

enum IndicatorKind {
    Dot,
    Bar,
    Icon(Icon),
}

pub struct Indicator {
    kind: IndicatorKind,
    border_color: Option<Color>,
    pub color: Color,
}

impl Indicator {
    pub fn dot() -> Self {
        Self {
            kind: IndicatorKind::Dot,
            border_color: None,
            color: Color::Default,
        }
    }

    pub fn bar() -> Self {
        Self {
            kind: IndicatorKind::Bar,
            border_color: None,
            color: Color::Default,
        }
    }

    pub fn icon(icon: impl Into<Icon>) -> Self {
        Self {
            kind: IndicatorKind::Icon(icon.into()),
            border_color: None,
            color: Color::Default,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn border_color(mut self, color: Color) -> Self {
        self.border_color = Some(color);
        self
    }
}

impl Element for Indicator {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Indicator);
        node.props.insert(
            "indicator_color".to_string(),
            serialize_color(self.color).into(),
        );
        if let Some(border_color) = self.border_color {
            node.props.insert(
                "indicator_border_color".to_string(),
                serialize_color(border_color).into(),
            );
        }
        match self.kind {
            IndicatorKind::Dot => {
                node.props
                    .insert("indicator_kind".to_string(), "dot".into());
            }
            IndicatorKind::Bar => {
                node.props
                    .insert("indicator_kind".to_string(), "bar".into());
            }
            IndicatorKind::Icon(icon) => {
                node.props
                    .insert("indicator_kind".to_string(), "icon".into());
                node.text = Some(icon.name);
                if let Some(size) = icon.icon_size {
                    node.props
                        .insert("icon_size".to_string(), serialize_icon_size(size).into());
                }
                if let Some(color) = icon.icon_color {
                    node.props
                        .insert("icon_color".to_string(), serialize_color(color).into());
                }
            }
        }
        node
    }
}

pub struct IconWithIndicator {
    icon: Icon,
    indicator: Option<Indicator>,
    indicator_border_color: Option<gpui_api::Hsla>,
}

impl IconWithIndicator {
    pub fn new(icon: Icon, indicator: Option<Indicator>) -> Self {
        Self {
            icon,
            indicator,
            indicator_border_color: None,
        }
    }

    pub fn indicator(mut self, indicator: Option<Indicator>) -> Self {
        self.indicator = indicator;
        self
    }

    pub fn indicator_color(mut self, color: Color) -> Self {
        if let Some(indicator) = self.indicator.as_mut() {
            indicator.color = color;
        }
        self
    }

    pub fn indicator_border_color(mut self, color: Option<gpui_api::Hsla>) -> Self {
        self.indicator_border_color = color;
        self
    }
}

impl Element for IconWithIndicator {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        gpui_plugin::div()
            .relative()
            .child(self.icon)
            .when_some(self.indicator, |this, indicator| {
                this.child(
                    gpui_plugin::div()
                        .absolute()
                        .size_2p5()
                        .rounded_full()
                        .border_2()
                        .when_some(self.indicator_border_color, |this, color| {
                            this.border_color(color)
                        })
                        .bottom_neg_0p5()
                        .right_neg_0p5()
                        .child(indicator),
                )
            })
            .into_node(context)
    }
}

impl From<Icon> for Indicator {
    fn from(value: Icon) -> Self {
        Self::icon(value)
    }
}

impl From<IconName> for Icon {
    fn from(value: IconName) -> Self {
        Icon::new(value)
    }
}

#[derive(Clone)]
pub struct KeyBinding {
    display: SharedString,
    size: Option<gpui_api::AbsoluteLength>,
    platform_style: PlatformStyle,
    disabled: bool,
}

impl KeyBinding {
    pub fn for_action(action: &dyn Action, cx: &App) -> Self {
        Self::new(action, None, cx)
    }

    pub fn for_action_in(action: &dyn Action, focus: &FocusHandle, cx: &App) -> Self {
        Self::new(action, Some(*focus), cx)
    }

    pub fn has_binding(&self, _window: &Window) -> bool {
        !self.display.is_empty()
    }

    pub fn set_vim_mode(_cx: &mut App, _enabled: bool) {}

    pub fn new(action: &dyn Action, _focus_handle: Option<FocusHandle>, _cx: &App) -> Self {
        Self {
            display: SharedString::new(action.name()),
            size: None,
            platform_style: PlatformStyle::platform(),
            disabled: false,
        }
    }

    pub fn from_keystrokes(
        keystrokes: std::rc::Rc<[gpui_api::KeybindingKeystroke]>,
        _vim_mode: bool,
    ) -> Self {
        let display = Iterator::map(keystrokes.iter().cloned(), |keystroke| {
            keystroke.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ");
        Self {
            display: SharedString::new(display),
            size: None,
            platform_style: PlatformStyle::platform(),
            disabled: false,
        }
    }

    pub fn platform_style(mut self, platform_style: PlatformStyle) -> Self {
        self.platform_style = platform_style;
        self
    }

    pub fn size(mut self, size: impl Into<gpui_api::AbsoluteLength>) -> Self {
        self.size = Some(size.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Element for KeyBinding {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut label = Label::new(self.display);
        label = label.size(LabelSize::Small);
        label = label.color(if self.disabled {
            Color::Disabled
        } else {
            match self.platform_style {
                PlatformStyle::Mac | PlatformStyle::Linux | PlatformStyle::Windows => Color::Muted,
            }
        });
        if let Some(size) = self.size {
            label = label.text_size(size);
        }
        label.into_node(context)
    }
}

pub struct KeybindingHint {
    prefix: Option<SharedString>,
    suffix: Option<SharedString>,
    keybinding: KeyBinding,
    size: Option<gpui_api::Pixels>,
    background_color: gpui_api::Hsla,
}

impl KeybindingHint {
    pub fn new(keybinding: KeyBinding, background_color: gpui_api::Hsla) -> Self {
        Self {
            prefix: None,
            suffix: None,
            keybinding,
            size: None,
            background_color,
        }
    }

    pub fn with_prefix(
        prefix: impl Into<SharedString>,
        keybinding: KeyBinding,
        background_color: gpui_api::Hsla,
    ) -> Self {
        Self::new(keybinding, background_color).prefix(prefix)
    }

    pub fn with_suffix(
        keybinding: KeyBinding,
        suffix: impl Into<SharedString>,
        background_color: gpui_api::Hsla,
    ) -> Self {
        Self::new(keybinding, background_color).suffix(suffix)
    }

    pub fn prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    pub fn suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    pub fn size(mut self, size: impl Into<Option<gpui_api::Pixels>>) -> Self {
        self.size = size.into();
        self
    }
}

impl Element for KeybindingHint {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut keybinding = self.keybinding;
        if let Some(size) = self.size {
            keybinding = keybinding.size(size);
        }
        h_flex()
            .gap_1()
            .px_1()
            .py_0p5()
            .rounded_sm()
            .bg(self.background_color.opacity(0.15))
            .children(self.prefix)
            .child(keybinding)
            .children(self.suffix)
            .into_node(context)
    }
}

pub struct Divider {
    orientation: &'static str,
    styles: StyleMap,
    inset: bool,
    dashed: bool,
    color: Option<DividerColor>,
}

impl Divider {
    pub fn horizontal() -> Self {
        Self {
            orientation: "horizontal",
            styles: StyleMap::default(),
            inset: false,
            dashed: false,
            color: None,
        }
    }

    pub fn vertical() -> Self {
        Self {
            orientation: "vertical",
            styles: StyleMap::default(),
            inset: false,
            dashed: false,
            color: None,
        }
    }

    pub fn horizontal_dashed() -> Self {
        Self::horizontal().dashed()
    }

    pub fn vertical_dashed() -> Self {
        Self::vertical().dashed()
    }

    pub fn inset(mut self) -> Self {
        self.inset = true;
        self
    }

    pub fn dashed(mut self) -> Self {
        self.dashed = true;
        self
    }

    pub fn color(mut self, color: DividerColor) -> Self {
        self.color = Some(color);
        self
    }
}

impl Element for Divider {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Divider);
        node.styles = self.styles;
        node.props
            .insert("orientation".to_string(), self.orientation.into());
        if self.inset {
            node.props.insert("inset".to_string(), true.into());
        }
        if self.dashed {
            node.props.insert("dashed".to_string(), true.into());
        }
        if let Some(color) = self.color {
            node.props.insert(
                "divider_color".to_string(),
                serialize_divider_color(color).into(),
            );
        }
        node
    }
}

impl Styled for Divider {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

pub struct ButtonLike {
    id: ElementId,
    styles: StyleMap,
    interactivity: Interactivity,
    style: ButtonStyle,
    disabled: bool,
    selected: bool,
    selected_style: Option<ButtonStyle>,
    width: Option<DefiniteLength>,
    height: Option<DefiniteLength>,
    layer: Option<ElevationIndex>,
    tab_index: Option<isize>,
    size: ButtonSize,
    cursor_style: CursorStyle,
    children: Vec<gpui_plugin::AnyElement>,
    has_tooltip: bool,
    has_hoverable_tooltip: bool,
}

impl ButtonLike {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            styles: StyleMap::default(),
            interactivity: Interactivity::default(),
            style: ButtonStyle::default(),
            disabled: false,
            selected: false,
            selected_style: None,
            width: None,
            height: None,
            layer: None,
            tab_index: None,
            size: ButtonSize::Default,
            cursor_style: CursorStyle::PointingHand,
            children: Vec::new(),
            has_tooltip: false,
            has_hoverable_tooltip: false,
        }
    }

    pub fn new_rounded_left(id: impl Into<ElementId>) -> Self {
        Self::new(id)
    }

    pub fn new_rounded_right(id: impl Into<ElementId>) -> Self {
        Self::new(id)
    }

    pub fn new_rounded_all(id: impl Into<ElementId>) -> Self {
        Self::new(id)
    }

    pub fn child(self, child: impl gpui_api::IntoElement) -> Self {
        gpui_api::ParentElement::child(self, child)
    }

    pub fn children(self, children: impl IntoIterator<Item = impl gpui_api::IntoElement>) -> Self {
        gpui_api::ParentElement::children(self, children)
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.styles.opacity = Some(opacity);
        self
    }

    pub fn height(mut self, height: DefiniteLength) -> Self {
        self.height = Some(height);
        self
    }

    #[allow(dead_code)]
    pub(crate) fn rounding(self, _rounding: impl Into<Option<()>>) -> Self {
        self
    }

    pub fn on_right_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity.push_handler(
            UiEventKind::AuxClick,
            EventPhase::Bubble,
            None,
            None,
            std::rc::Rc::new(move |event, window, app| {
                if let UiEvent::AuxClick(click_event) = event {
                    handler(click_event, window, app);
                }
            }),
        );
        self
    }

    pub fn hoverable_tooltip(
        mut self,
        _tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.has_hoverable_tooltip = true;
        self
    }
}

impl Disableable for ButtonLike {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Toggleable for ButtonLike {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl SelectableButton for ButtonLike {
    fn selected_style(mut self, style: ButtonStyle) -> Self {
        self.selected_style = Some(style);
        self
    }
}

impl Clickable for ButtonLike {
    fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.interactivity.push_handler(
            UiEventKind::Click,
            EventPhase::Bubble,
            None,
            None,
            std::rc::Rc::new(move |event, window, app| {
                if let UiEvent::Click(click_event) = event {
                    handler(click_event, window, app);
                }
            }),
        );
        self
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.cursor_style = cursor_style;
        self
    }
}

impl FixedWidth for ButtonLike {
    fn width(mut self, width: impl Into<DefiniteLength>) -> Self {
        self.width = Some(width.into());
        self
    }

    fn full_width(mut self) -> Self {
        self.width = Some(gpui_api::relative(1.));
        self
    }
}

impl ButtonCommon for ButtonLike {
    fn id(&self) -> &ElementId {
        &self.id
    }

    fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    fn tooltip(mut self, _tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.has_tooltip = true;
        self
    }

    fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.tab_index = Some(tab_index.into());
        self
    }

    fn layer(mut self, elevation: ElevationIndex) -> Self {
        self.layer = Some(elevation);
        self
    }

    fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.interactivity.tracked_focus_handle = Some(*focus_handle);
        self
    }
}

impl VisibleOnHover for ButtonLike {
    fn visible_on_hover(mut self, group_name: impl Into<SharedString>) -> Self {
        self.styles.opacity = Some(0.);
        let mut style = StyleMap::default();
        style.opacity = Some(1.);
        self.interactivity.group_hover_style = Some(gpui_api::GroupStyle {
            group: group_name.into(),
            style,
        });
        self
    }
}

impl gpui_api::ParentElement for ButtonLike {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui_plugin::AnyElement>) {
        self.children.extend(elements);
    }
}

impl Element for ButtonLike {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Div);
        let mut styles = self.styles;
        styles.display = Some(gpui_api::Display::Flex);
        styles.flex_direction = Some(gpui_api::FlexDirection::Row);
        styles.align_items = Some(gpui_api::AlignItems::Center);
        styles.mouse_cursor = Some(self.cursor_style);
        if let Some(width) = self.width {
            styles.size.width = Some(width.into());
        }
        if let Some(height) = self.height {
            styles.size.height = Some(height.into());
        } else {
            styles.size.height = Some(self.size.rems().into());
        }
        node.styles = styles;
        let element_id = self.id.to_string();
        node.element_id = Some(element_id.clone());
        node.props
            .insert("element_id".to_string(), element_id.into());
        node.props.insert(
            "button_like_style".to_string(),
            serialize_button_style(self.style).into(),
        );
        if self.disabled {
            node.props.insert("disabled".to_string(), true.into());
        }
        if self.selected {
            node.props.insert("selected".to_string(), true.into());
        }
        if let Some(style) = self.selected_style {
            node.props.insert(
                "selected_button_style".to_string(),
                serialize_button_style(style).into(),
            );
        }
        if let Some(layer) = self.layer {
            node.props.insert(
                "button_layer".to_string(),
                serialize_elevation(layer).into(),
            );
        }
        if let Some(tab_index) = self.tab_index {
            node.props
                .insert("tab_index".to_string(), (tab_index as f32).into());
        }
        if self.has_tooltip {
            node.props.insert("has_tooltip".to_string(), true.into());
        }
        if self.has_hoverable_tooltip {
            node.props
                .insert("has_hoverable_tooltip".to_string(), true.into());
        }
        node.events = Iterator::map(self.interactivity.handlers().iter().cloned(), |handler| {
            context.register_event_handler(handler)
        })
        .collect();
        node.children =
            Iterator::map(self.children.into_iter(), |child| child.into_node(context)).collect();
        node
    }
}

pub struct ButtonLink {
    label: SharedString,
    label_size: LabelSize,
    label_color: Color,
    link: String,
    no_icon: bool,
}

impl ButtonLink {
    pub fn new(label: impl Into<SharedString>, link: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            label_size: LabelSize::Default,
            label_color: Color::Default,
            link: link.into(),
            no_icon: false,
        }
    }

    pub fn no_icon(mut self, no_icon: bool) -> Self {
        self.no_icon = no_icon;
        self
    }

    pub fn label_size(mut self, label_size: LabelSize) -> Self {
        self.label_size = label_size;
        self
    }

    pub fn label_color(mut self, label_color: Color) -> Self {
        self.label_color = label_color;
        self
    }
}

impl Element for ButtonLink {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let id = format!("{}-{}", self.label, self.link);
        gpui_api::ParentElement::child(
            ButtonLike::new(id).size(ButtonSize::None),
            h_flex()
                .gap_0p5()
                .child(
                    Label::new(self.label)
                        .size(self.label_size)
                        .color(self.label_color)
                        .underline(),
                )
                .when(!self.no_icon, |this| {
                    this.child(
                        Icon::new(IconName::ArrowUpRight)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    )
                }),
        )
        .into_node(context)
    }
}

pub struct CopyButton {
    id: ElementId,
    message: SharedString,
    icon_size: IconSize,
    disabled: bool,
    tooltip_label: SharedString,
    visible_on_hover: Option<SharedString>,
    custom_on_click: Option<std::rc::Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl CopyButton {
    pub fn new(id: impl Into<ElementId>, message: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            message: message.into(),
            icon_size: IconSize::Small,
            disabled: false,
            tooltip_label: SharedString::new("Copy"),
            visible_on_hover: None,
            custom_on_click: None,
        }
    }

    pub fn icon_size(mut self, icon_size: IconSize) -> Self {
        self.icon_size = icon_size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tooltip_label(mut self, tooltip_label: impl Into<SharedString>) -> Self {
        self.tooltip_label = tooltip_label.into();
        self
    }

    pub fn visible_on_hover(mut self, visible_on_hover: impl Into<SharedString>) -> Self {
        self.visible_on_hover = Some(visible_on_hover.into());
        self
    }

    pub fn custom_on_click(
        mut self,
        custom_on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.custom_on_click = Some(std::rc::Rc::new(custom_on_click));
        self
    }
}

impl Element for CopyButton {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let message = self.message;
        let tooltip_label = self.tooltip_label;
        let custom_on_click = self.custom_on_click;
        let mut button = IconButton::new(self.id, IconName::Copy)
            .icon_color(Color::Muted)
            .icon_size(self.icon_size)
            .disabled(self.disabled)
            .on_click(move |_event, window, cx| {
                if let Some(custom_on_click) = custom_on_click.as_ref() {
                    custom_on_click(window, cx);
                } else {
                    let _ = &message;
                    let _ = &tooltip_label;
                }
            });

        if let Some(group) = self.visible_on_hover {
            button = button.visible_on_hover(group);
        }

        button.into_node(context)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum SplitButtonStyle {
    Filled,
    Outlined,
    Transparent,
}

pub enum SplitButtonKind {
    ButtonLike(ButtonLike),
    IconButton(IconButton),
}

impl From<IconButton> for SplitButtonKind {
    fn from(icon_button: IconButton) -> Self {
        Self::IconButton(icon_button)
    }
}

impl From<ButtonLike> for SplitButtonKind {
    fn from(button_like: ButtonLike) -> Self {
        Self::ButtonLike(button_like)
    }
}

pub struct SplitButton {
    left: SplitButtonKind,
    right: gpui_plugin::AnyElement,
    style: SplitButtonStyle,
}

impl SplitButton {
    pub fn new(left: impl Into<SplitButtonKind>, right: gpui_plugin::AnyElement) -> Self {
        Self {
            left: left.into(),
            right,
            style: SplitButtonStyle::Filled,
        }
    }

    pub fn style(mut self, style: SplitButtonStyle) -> Self {
        self.style = style;
        self
    }
}

impl Element for SplitButton {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        h_flex()
            .when(self.style != SplitButtonStyle::Transparent, |this| {
                this.rounded_sm().border_1()
            })
            .map(|mut this| {
                gpui_api::ParentElement::extend(
                    &mut this,
                    std::iter::once(match self.left {
                        SplitButtonKind::ButtonLike(button) => button.into_any_element(),
                        SplitButtonKind::IconButton(button) => button.into_any_element(),
                    }),
                );
                gpui_api::ParentElement::extend(
                    &mut this,
                    std::iter::once(vertical_divider().into_any_element()),
                );
                gpui_api::ParentElement::extend(&mut this, std::iter::once(self.right));
                this
            })
            .into_node(context)
    }
}

pub struct Button {
    id: ElementId,
    text: SharedString,
    disabled: bool,
    styles: StyleMap,
    interactivity: Interactivity,
    button_style: Option<ButtonStyle>,
    button_icon: Option<IconName>,
    button_icon_position: Option<IconPosition>,
    selected: bool,
    selected_style: Option<ButtonStyle>,
    label_color: Option<Color>,
    label_size: Option<LabelSize>,
    button_size: Option<ButtonSize>,
    selected_label: Option<SharedString>,
    selected_label_color: Option<Color>,
    layer: Option<ElevationIndex>,
    tab_index: Option<isize>,
    tracks_focus: bool,
    has_tooltip: bool,
    key_binding: Option<KeyBinding>,
    key_binding_position: Option<KeybindingPosition>,
    alpha: Option<f32>,
    truncate: bool,
    loading: bool,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            disabled: false,
            styles: StyleMap::default(),
            interactivity: Interactivity::default(),
            button_style: None,
            button_icon: None,
            button_icon_position: None,
            selected: false,
            selected_style: None,
            label_color: None,
            label_size: None,
            button_size: None,
            selected_label: None,
            selected_label_color: None,
            layer: None,
            tab_index: None,
            tracks_focus: false,
            has_tooltip: false,
            key_binding: None,
            key_binding_position: None,
            alpha: None,
            truncate: false,
            loading: false,
        }
    }

    pub fn disabled(self, disabled: bool) -> Self {
        Disableable::disabled(self, disabled)
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.button_style = Some(style);
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.button_size = Some(size);
        self
    }

    pub fn color(mut self, color: impl Into<Option<Color>>) -> Self {
        self.label_color = color.into();
        self
    }

    pub fn label_size(mut self, label_size: impl Into<Option<LabelSize>>) -> Self {
        self.label_size = label_size.into();
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.button_icon = Some(icon);
        self
    }

    pub fn icon_position(mut self, position: IconPosition) -> Self {
        self.button_icon_position = Some(position);
        self
    }

    pub fn start_icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        if let Some(icon) = icon.into() {
            self.button_icon = icon.icon_name();
            self.button_icon_position = Some(IconPosition::Start);
        }
        self
    }

    pub fn end_icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        if let Some(icon) = icon.into() {
            self.button_icon = icon.icon_name();
            self.button_icon_position = Some(IconPosition::End);
        }
        self
    }

    pub fn selected_label<L: Into<SharedString>>(mut self, label: Option<L>) -> Self {
        self.selected_label = label.map(Into::into);
        self
    }

    pub fn selected_label_color(mut self, color: Option<Color>) -> Self {
        self.selected_label_color = color;
        self
    }

    pub fn tooltip(
        mut self,
        _tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.has_tooltip = true;
        self
    }

    pub fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.tab_index = Some(tab_index.into());
        self
    }

    pub fn layer(mut self, elevation: ElevationIndex) -> Self {
        self.layer = Some(elevation);
        self
    }

    pub fn track_focus(mut self, _focus_handle: &FocusHandle) -> Self {
        self.tracks_focus = true;
        self
    }

    pub fn key_binding(mut self, key_binding: impl Into<Option<KeyBinding>>) -> Self {
        self.key_binding = key_binding.into();
        self
    }

    pub fn key_binding_position(mut self, position: KeybindingPosition) -> Self {
        self.key_binding_position = Some(position);
        self
    }

    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }

    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity.push_handler(
            UiEventKind::Click,
            EventPhase::Bubble,
            None,
            None,
            std::rc::Rc::new(move |event, window, app| {
                if let UiEvent::Click(click_event) = event {
                    handler(click_event, window, app);
                }
            }),
        );
        self
    }
}

impl Element for Button {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::Button);
        node.text = Some(self.text.to_string());
        node.styles = self.styles;
        let element_id = self.id.to_string();
        node.element_id = Some(element_id.clone());
        node.props
            .insert("element_id".to_string(), element_id.into());
        if self.disabled {
            node.props.insert("disabled".to_string(), true.into());
        }
        if self.selected {
            node.props.insert("selected".to_string(), true.into());
        }
        if let Some(selected_label) = self.selected_label {
            node.props.insert(
                "selected_label".to_string(),
                selected_label.to_string().into(),
            );
        }
        if let Some(selected_label_color) = self.selected_label_color {
            node.props.insert(
                "selected_label_color".to_string(),
                serialize_color(selected_label_color).into(),
            );
        }
        if let Some(style) = self.button_style {
            node.props.insert(
                "button_style".to_string(),
                serialize_button_style(style).into(),
            );
        }
        if let Some(style) = self.selected_style {
            node.props.insert(
                "selected_button_style".to_string(),
                serialize_button_style(style).into(),
            );
        }
        if let Some(color) = self.label_color {
            node.props
                .insert("label_color".to_string(), serialize_color(color).into());
        }
        if let Some(size) = self.label_size {
            node.props
                .insert("label_size".to_string(), serialize_label_size(size).into());
        }
        if let Some(size) = self.button_size {
            node.props.insert(
                "button_size".to_string(),
                serialize_button_size(size).into(),
            );
        }
        if let Some(icon) = self.button_icon {
            let icon_name: &'static str = icon.into();
            node.props
                .insert("button_icon".to_string(), icon_name.into());
        }
        if let Some(position) = self.button_icon_position {
            node.props.insert(
                "button_icon_position".to_string(),
                match position {
                    IconPosition::Start => "start".into(),
                    IconPosition::End => "end".into(),
                },
            );
        }
        if let Some(alpha) = self.alpha {
            node.props.insert("alpha".to_string(), alpha.into());
        }
        if let Some(tab_index) = self.tab_index {
            node.props
                .insert("tab_index".to_string(), (tab_index as f32).into());
        }
        if let Some(layer) = self.layer {
            node.props.insert(
                "button_layer".to_string(),
                serialize_elevation(layer).into(),
            );
        }
        if self.tracks_focus {
            node.props.insert("tracks_focus".to_string(), true.into());
        }
        if self.has_tooltip {
            node.props.insert("has_tooltip".to_string(), true.into());
        }
        if let Some(key_binding) = self.key_binding {
            node.props.insert(
                "key_binding_text".to_string(),
                key_binding.display.to_string().into(),
            );
        }
        if let Some(position) = self.key_binding_position {
            node.props.insert(
                "key_binding_position".to_string(),
                serialize_keybinding_position(position).into(),
            );
        }
        if self.truncate {
            node.props.insert("truncate".to_string(), true.into());
        }
        if self.loading {
            node.props.insert("loading".to_string(), true.into());
        }
        node.events = Iterator::map(self.interactivity.handlers().iter().cloned(), |handler| {
            context.register_event_handler(handler)
        })
        .collect();
        node
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Clickable for Button {
    fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Button::on_click(self, handler)
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.styles.mouse_cursor = Some(cursor_style);
        self
    }
}

impl Disableable for Button {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl FixedWidth for Button {
    fn width(mut self, width: impl Into<DefiniteLength>) -> Self {
        self.styles.size.width = Some(width.into().into());
        self
    }

    fn full_width(mut self) -> Self {
        self.styles.size.width = Some(gpui_api::relative(1.).into());
        self
    }
}

impl ButtonCommon for Button {
    fn id(&self) -> &ElementId {
        &self.id
    }

    fn style(self, style: ButtonStyle) -> Self {
        Button::style(self, style)
    }

    fn size(self, size: ButtonSize) -> Self {
        Button::size(self, size)
    }

    fn tooltip(self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        Button::tooltip(self, tooltip)
    }

    fn tab_index(self, tab_index: impl Into<isize>) -> Self {
        Button::tab_index(self, tab_index)
    }

    fn layer(self, elevation: ElevationIndex) -> Self {
        Button::layer(self, elevation)
    }

    fn track_focus(self, focus_handle: &FocusHandle) -> Self {
        Button::track_focus(self, focus_handle)
    }
}

impl Toggleable for Button {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl SelectableButton for Button {
    fn selected_style(mut self, style: ButtonStyle) -> Self {
        self.selected_style = Some(style);
        self
    }
}

pub struct IconDescriptor(String);

impl From<String> for IconDescriptor {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for IconDescriptor {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<IconName> for IconDescriptor {
    fn from(value: IconName) -> Self {
        let icon_name: &'static str = value.into();
        Self(icon_name.to_string())
    }
}

pub struct ProgressBar {
    id: ElementId,
    value: f32,
    max_value: f32,
    styles: StyleMap,
    bg_color: Option<gpui_api::Hsla>,
    fg_color: Option<gpui_api::Hsla>,
    over_color: Option<gpui_api::Hsla>,
}

impl ProgressBar {
    pub fn new(id: impl Into<ElementId>, value: f32, max_value: f32, _cx: &App) -> Self {
        Self {
            id: id.into(),
            value,
            max_value,
            styles: StyleMap::default(),
            bg_color: None,
            fg_color: None,
            over_color: None,
        }
    }

    pub fn value(mut self, value: f32) -> Self {
        self.value = value;
        self
    }

    pub fn max_value(mut self, max_value: f32) -> Self {
        self.max_value = max_value;
        self
    }

    pub fn bg_color(mut self, color: gpui_api::Hsla) -> Self {
        self.bg_color = Some(color);
        self
    }

    pub fn fg_color(mut self, color: gpui_api::Hsla) -> Self {
        self.fg_color = Some(color);
        self
    }

    pub fn over_color(mut self, color: gpui_api::Hsla) -> Self {
        self.over_color = Some(color);
        self
    }
}

impl Element for ProgressBar {
    fn into_node(self, _context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::ProgressBar);
        node.styles = self.styles;
        let element_id = self.id.to_string();
        node.element_id = Some(element_id.clone());
        node.props
            .insert("element_id".to_string(), element_id.into());
        node.props.insert("value".to_string(), self.value.into());
        node.props
            .insert("max_value".to_string(), self.max_value.into());
        if let Some(color) = self.bg_color {
            node.props
                .insert("bg_color".to_string(), serialize_hsla(color).into());
        }
        if let Some(color) = self.fg_color {
            node.props
                .insert("fg_color".to_string(), serialize_hsla(color).into());
        }
        if let Some(color) = self.over_color {
            node.props
                .insert("over_color".to_string(), serialize_hsla(color).into());
        }
        node
    }
}

impl Styled for ProgressBar {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

pub struct CircularProgress {
    value: f32,
    max_value: f32,
    size: gpui_api::Pixels,
    stroke_width: gpui_api::Pixels,
    bg_color: gpui_api::Hsla,
    progress_color: gpui_api::Hsla,
}

impl CircularProgress {
    pub fn new(value: f32, max_value: f32, size: gpui_api::Pixels, cx: &App) -> Self {
        Self {
            value,
            max_value,
            size,
            stroke_width: gpui_api::px(4.0),
            bg_color: cx.theme().colors().border_variant,
            progress_color: cx.theme().status().info,
        }
    }

    pub fn value(mut self, value: f32) -> Self {
        self.value = value;
        self
    }

    pub fn max_value(mut self, max_value: f32) -> Self {
        self.max_value = max_value;
        self
    }

    pub fn size(mut self, size: gpui_api::Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn stroke_width(mut self, stroke_width: gpui_api::Pixels) -> Self {
        self.stroke_width = stroke_width;
        self
    }

    pub fn bg_color(mut self, color: gpui_api::Hsla) -> Self {
        self.bg_color = color;
        self
    }

    pub fn progress_color(mut self, color: gpui_api::Hsla) -> Self {
        self.progress_color = color;
        self
    }
}

impl Element for CircularProgress {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let progress = if self.max_value <= 0.0 {
            0.0
        } else {
            (self.value / self.max_value).clamp(0.0, 1.0)
        };

        gpui_plugin::div()
            .size(self.size)
            .rounded_full()
            .border_1()
            .border_color(self.bg_color)
            .p(self.stroke_width)
            .child(
                gpui_plugin::div()
                    .size_full()
                    .rounded_full()
                    .bg(self.progress_color.opacity(progress.max(0.1))),
            )
            .into_node(context)
    }
}

pub fn checkbox(id: impl Into<ElementId>, toggle_state: ToggleState) -> Checkbox {
    Checkbox::new(id, toggle_state)
}

pub fn switch(id: impl Into<ElementId>, toggle_state: ToggleState) -> Switch {
    Switch::new(id, toggle_state)
}

pub struct Checkbox {
    id: ElementId,
    toggle_state: ToggleState,
    style: ToggleStyle,
    disabled: bool,
    placeholder: bool,
    filled: bool,
    visualization: bool,
    label: Option<SharedString>,
    label_size: LabelSize,
    label_color: Color,
    has_tooltip: bool,
    on_click: Option<std::rc::Rc<dyn Fn(&ToggleState, &ClickEvent, &mut Window, &mut App)>>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, checked: ToggleState) -> Self {
        Self {
            id: id.into(),
            toggle_state: checked,
            style: ToggleStyle::default(),
            disabled: false,
            placeholder: false,
            filled: false,
            visualization: false,
            label: None,
            label_size: LabelSize::Default,
            label_color: Color::Muted,
            has_tooltip: false,
            on_click: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(std::rc::Rc::new(move |state, _event, window, cx| {
            handler(state, window, cx)
        }));
        self
    }

    pub fn on_click_ext(
        mut self,
        handler: impl Fn(&ToggleState, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn fill(mut self) -> Self {
        self.filled = true;
        self
    }

    pub fn visualization_only(mut self, visualization: bool) -> Self {
        self.visualization = visualization;
        self
    }

    pub fn style(mut self, style: ToggleStyle) -> Self {
        self.style = style;
        self
    }

    pub fn elevation(mut self, elevation: ElevationIndex) -> Self {
        self.style = ToggleStyle::ElevationBased(elevation);
        self
    }

    pub fn tooltip(
        mut self,
        _tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.has_tooltip = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label_size(mut self, size: LabelSize) -> Self {
        self.label_size = size;
        self
    }

    pub fn label_color(mut self, color: Color) -> Self {
        self.label_color = color;
        self
    }
}

impl Element for Checkbox {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let base_color = gpui_api::hsla(0.0, 0.0, 0.0, 1.0);
        let icon = match self.toggle_state {
            ToggleState::Selected if !self.placeholder => {
                Some(Icon::new(IconName::Check).size(IconSize::XSmall))
            }
            ToggleState::Indeterminate => Some(Icon::new(IconName::Dash).size(IconSize::XSmall)),
            _ => None,
        };
        let next_state = self.toggle_state.inverse();

        let mut button = ButtonLike::new(self.id).child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    gpui_plugin::div()
                        .size_4()
                        .rounded_sm()
                        .border_1()
                        .when(
                            self.filled || self.toggle_state == ToggleState::Selected,
                            |this| this.bg(base_color.opacity(0.1)),
                        )
                        .when_some(icon, |this, icon| this.child(icon)),
                )
                .when_some(self.label, |this, label| {
                    this.child(
                        Label::new(label)
                            .size(self.label_size)
                            .color(self.label_color),
                    )
                }),
        );

        if self.has_tooltip {
            button.has_tooltip = true;
        }

        if let Some(handler) = self
            .on_click
            .filter(|_| !self.disabled && !self.visualization)
        {
            button =
                button.on_click(move |event, window, cx| handler(&next_state, event, window, cx));
        }

        button.into_node(context)
    }
}

pub struct Switch {
    id: ElementId,
    toggle_state: ToggleState,
    disabled: bool,
    on_click: Option<std::rc::Rc<dyn Fn(&ToggleState, &mut Window, &mut App)>>,
    label: Option<SharedString>,
    label_position: Option<SwitchLabelPosition>,
    label_size: LabelSize,
    full_width: bool,
    key_binding: Option<KeyBinding>,
    color: SwitchColor,
    tab_index: Option<isize>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, state: ToggleState) -> Self {
        Self {
            id: id.into(),
            toggle_state: state,
            disabled: false,
            on_click: None,
            label: None,
            label_position: None,
            label_size: LabelSize::Small,
            full_width: false,
            key_binding: None,
            color: SwitchColor::default(),
            tab_index: None,
        }
    }

    pub fn color(mut self, color: SwitchColor) -> Self {
        self.color = color;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn label_position(
        mut self,
        label_position: impl Into<Option<SwitchLabelPosition>>,
    ) -> Self {
        self.label_position = label_position.into();
        self
    }

    pub fn label_size(mut self, size: LabelSize) -> Self {
        self.label_size = size;
        self
    }

    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    pub fn key_binding(mut self, key_binding: impl Into<Option<KeyBinding>>) -> Self {
        self.key_binding = key_binding.into();
        self
    }

    pub fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.tab_index = Some(tab_index.into());
        self
    }
}

impl Element for Switch {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let base_color = gpui_api::hsla(0.0, 0.0, 0.0, 1.0);
        let next_state = self.toggle_state.inverse();
        let is_on = self.toggle_state == ToggleState::Selected;
        let label_text = self.label;
        let switch_visual = h_flex()
            .w(gpui_api::rems(2.0))
            .h(gpui_api::rems(1.1))
            .rounded_full()
            .border_1()
            .px_0p5()
            .justify_between()
            .when(is_on, |this| this.bg(base_color.opacity(0.12)))
            .child(
                gpui_plugin::div()
                    .size_3()
                    .rounded_full()
                    .bg(base_color.opacity(if is_on { 0.9 } else { 0.35 })),
            );

        let mut button = ButtonLike::new(self.id.clone())
            .when(self.full_width, |this| this.full_width())
            .when_some(self.tab_index, |this, tab_index| this.tab_index(tab_index))
            .child(
                h_flex()
                    .gap_2()
                    .when(self.full_width, |this| this.w_full().justify_between())
                    .when(
                        self.label_position == Some(SwitchLabelPosition::Start),
                        |this| {
                            this.when_some(label_text.clone(), |this, label| {
                                this.child(Label::new(label).size(self.label_size))
                            })
                        },
                    )
                    .child(switch_visual)
                    .when(
                        self.label_position != Some(SwitchLabelPosition::Start),
                        |this| {
                            this.when_some(label_text, |this, label| {
                                this.child(Label::new(label).size(self.label_size))
                            })
                        },
                    )
                    .children(self.key_binding),
            );

        if let Some(handler) = self.on_click.filter(|_| !self.disabled) {
            button = button.on_click(move |_event, window, cx| handler(&next_state, window, cx));
        }

        button.into_node(context)
    }
}

pub struct SwitchField {
    id: ElementId,
    label: Option<SharedString>,
    description: Option<SharedString>,
    toggle_state: ToggleState,
    on_click: std::sync::Arc<dyn Fn(&ToggleState, &mut Window, &mut App)>,
    disabled: bool,
    color: SwitchColor,
    has_tooltip: bool,
    tab_index: Option<isize>,
}

impl SwitchField {
    pub fn new(
        id: impl Into<ElementId>,
        label: Option<impl Into<SharedString>>,
        description: Option<SharedString>,
        toggle_state: impl Into<ToggleState>,
        on_click: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.map(Into::into),
            description,
            toggle_state: toggle_state.into(),
            on_click: std::sync::Arc::new(on_click),
            disabled: false,
            color: SwitchColor::Accent,
            has_tooltip: false,
            tab_index: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn color(mut self, color: SwitchColor) -> Self {
        self.color = color;
        self
    }

    pub fn tooltip(
        mut self,
        _tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.has_tooltip = true;
        self
    }

    pub fn tab_index(mut self, tab_index: isize) -> Self {
        self.tab_index = Some(tab_index);
        self
    }
}

impl Element for SwitchField {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let label_content = if let Some(description) = self.description {
            v_flex()
                .gap_0p5()
                .when_some(self.label, |this, label| this.child(Label::new(label)))
                .child(Label::new(description).color(Color::Muted))
                .into_any_element()
        } else if let Some(label) = self.label {
            Label::new(label).into_any_element()
        } else {
            gpui_plugin::Empty.into_any_element()
        };

        h_flex()
            .w_full()
            .gap_4()
            .justify_between()
            .map(|mut this| {
                gpui_api::ParentElement::extend(&mut this, std::iter::once(label_content));
                this
            })
            .child(
                Switch::new((self.id, "switch"), self.toggle_state)
                    .disabled(self.disabled)
                    .color(self.color)
                    .when_some(self.tab_index, |this, tab_index| this.tab_index(tab_index))
                    .on_click({
                        let on_click = self.on_click.clone();
                        move |state, window, cx| (on_click)(state, window, cx)
                    }),
            )
            .into_node(context)
    }
}

pub struct MenuItem {
    id: ElementId,
    text: SharedString,
    disabled: bool,
    styles: StyleMap,
    interactivity: Interactivity,
}

impl MenuItem {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            disabled: false,
            styles: StyleMap::default(),
            interactivity: Interactivity::default(),
        }
    }

    pub fn disabled(self, disabled: bool) -> Self {
        Disableable::disabled(self, disabled)
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.interactivity.push_handler(
            UiEventKind::Click,
            EventPhase::Bubble,
            None,
            None,
            std::rc::Rc::new(move |event, window, app| {
                if let UiEvent::Click(click_event) = event {
                    handler(click_event, window, app);
                }
            }),
        );
        self
    }
}

impl Element for MenuItem {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut node = UiNode::new(UiNodeKind::MenuItem);
        node.text = Some(self.text.to_string());
        node.styles = self.styles;
        let element_id = self.id.to_string();
        node.element_id = Some(element_id.clone());
        node.props
            .insert("element_id".to_string(), element_id.into());
        if self.disabled {
            node.props.insert("disabled".to_string(), true.into());
        }
        node.events = Iterator::map(self.interactivity.handlers().iter().cloned(), |handler| {
            context.register_event_handler(handler)
        })
        .collect();
        node
    }
}

impl Styled for MenuItem {
    fn style(&mut self) -> &mut StyleMap {
        &mut self.styles
    }
}

impl InteractiveElement for MenuItem {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Clickable for MenuItem {
    fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        MenuItem::on_click(self, handler)
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.styles.mouse_cursor = Some(cursor_style);
        self
    }
}

impl Disableable for MenuItem {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl<E: InteractiveElement + Styled> VisibleOnHover for E {
    fn visible_on_hover(self, group_name: impl Into<SharedString>) -> Self {
        self.invisible()
            .group_hover(group_name, |style| style.visible())
    }
}

impl<E: Styled> StyledTypography for E {}

impl<E: Styled> StyledExt for E {}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Default)]
pub enum TextSize {
    #[default]
    Default,
    Large,
    Small,
    XSmall,
    Ui,
    Editor,
}

impl TextSize {
    pub fn rems(self, _cx: &App) -> gpui_api::AbsoluteLength {
        match self {
            Self::Large => rems_from_px(16.),
            Self::Default => rems_from_px(14.),
            Self::Small => rems_from_px(12.),
            Self::XSmall => rems_from_px(10.),
            Self::Ui => rems_from_px(14.),
            Self::Editor => rems_from_px(14.),
        }
    }

    pub fn pixels(self, _cx: &App) -> gpui_api::Pixels {
        match self {
            Self::Large => gpui_api::px(16.),
            Self::Default => gpui_api::px(14.),
            Self::Small => gpui_api::px(12.),
            Self::XSmall => gpui_api::px(10.),
            Self::Ui => gpui_api::px(14.),
            Self::Editor => gpui_api::px(14.),
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub enum DynamicSpacing {
    Base00,
    Base01,
    Base02,
    Base03,
    Base04,
    Base06,
    Base08,
    Base12,
    Base16,
    Base20,
    Base24,
    Base32,
    Base40,
    Base48,
}

impl DynamicSpacing {
    fn px_value(self) -> f32 {
        match self {
            Self::Base00 => 0.0,
            Self::Base01 => 1.0,
            Self::Base02 => 2.0,
            Self::Base03 => 3.0,
            Self::Base04 => 4.0,
            Self::Base06 => 6.0,
            Self::Base08 => 8.0,
            Self::Base12 => 12.0,
            Self::Base16 => 16.0,
            Self::Base20 => 20.0,
            Self::Base24 => 24.0,
            Self::Base32 => 32.0,
            Self::Base40 => 40.0,
            Self::Base48 => 48.0,
        }
    }

    pub fn px(self, _cx: &App) -> gpui_api::Pixels {
        gpui_api::px(self.px_value())
    }

    pub fn rems(self, _cx: &App) -> gpui_api::AbsoluteLength {
        rems_from_px(self.px_value())
    }
}

pub fn divider() -> Divider {
    Divider::horizontal()
}

pub fn vertical_divider() -> Divider {
    Divider::vertical()
}

#[track_caller]
pub fn h_flex() -> gpui_plugin::Div {
    gpui_plugin::div().h_flex()
}

#[track_caller]
pub fn v_flex() -> gpui_plugin::Div {
    gpui_plugin::div().v_flex()
}

pub fn h_group_sm() -> gpui_plugin::Div {
    gpui_plugin::div().flex().gap_0p5()
}

pub fn h_group() -> gpui_plugin::Div {
    gpui_plugin::div().flex().gap_1()
}

pub fn h_group_lg() -> gpui_plugin::Div {
    gpui_plugin::div().flex().gap_1p5()
}

pub fn h_group_xl() -> gpui_plugin::Div {
    gpui_plugin::div().flex().gap_2()
}

pub fn v_group_sm() -> gpui_plugin::Div {
    gpui_plugin::div().flex().flex_col().gap_0p5()
}

pub fn v_group() -> gpui_plugin::Div {
    gpui_plugin::div().flex().flex_col().gap_1()
}

pub fn v_group_lg() -> gpui_plugin::Div {
    gpui_plugin::div().flex().flex_col().gap_1p5()
}

pub fn v_group_xl() -> gpui_plugin::Div {
    gpui_plugin::div().flex().flex_col().gap_2()
}

pub const BASE_REM_SIZE_IN_PX: f32 = 16.0;

pub fn rems_from_px(px: impl Into<f32>) -> gpui_api::AbsoluteLength {
    gpui_api::rems(px.into() / BASE_REM_SIZE_IN_PX).into()
}

pub fn vw(percent: f32, _window: &mut Window) -> gpui_api::Length {
    gpui_api::Length::Definite(gpui_api::relative(percent))
}

pub fn vh(percent: f32, _window: &mut Window) -> gpui_api::Length {
    gpui_api::Length::Definite(gpui_api::relative(percent))
}

fn elevated<E: Styled>(this: E, cx: &App, index: ElevationIndex) -> E {
    this.bg(elevation_background(index, cx))
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().colors().border_variant)
}

fn elevated_borderless<E: Styled>(this: E, cx: &mut App, index: ElevationIndex) -> E {
    this.bg(elevation_background(index, cx)).rounded_lg()
}

fn elevation_background(index: ElevationIndex, cx: &App) -> gpui_api::Hsla {
    match index {
        ElevationIndex::Background => cx.theme().colors().background,
        ElevationIndex::Surface => cx.theme().colors().surface_background,
        ElevationIndex::EditorSurface => cx.theme().colors().editor_background,
        ElevationIndex::ElevatedSurface => cx.theme().colors().elevated_surface_background,
        ElevationIndex::ModalSurface => cx.theme().colors().background,
    }
}

pub struct LoadingLabel {
    base: Label,
}

impl LoadingLabel {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            base: Label::new(text),
        }
    }
}

impl LabelCommon for LoadingLabel {
    fn size(mut self, size: LabelSize) -> Self {
        self.base = self.base.size(size);
        self
    }

    fn weight(mut self, weight: FontWeight) -> Self {
        self.base = self.base.weight(weight);
        self
    }

    fn line_height_style(mut self, line_height_style: LineHeightStyle) -> Self {
        self.base = self.base.line_height_style(line_height_style);
        self
    }

    fn color(mut self, color: Color) -> Self {
        self.base = self.base.color(color);
        self
    }

    fn strikethrough(mut self) -> Self {
        self.base = self.base.strikethrough();
        self
    }

    fn italic(mut self) -> Self {
        self.base = self.base.italic();
        self
    }

    fn underline(mut self) -> Self {
        self.base = self.base.underline();
        self
    }

    fn alpha(mut self, alpha: f32) -> Self {
        self.base = self.base.alpha(alpha);
        self
    }

    fn truncate(mut self) -> Self {
        self.base = self.base.truncate();
        self
    }

    fn single_line(mut self) -> Self {
        self.base = self.base.single_line();
        self
    }

    fn buffer_font(mut self, cx: &App) -> Self {
        self.base = self.base.buffer_font(cx);
        self
    }

    fn inline_code(mut self, cx: &App) -> Self {
        self.base = self.base.inline_code(cx);
        self
    }
}

impl Element for LoadingLabel {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        self.base.color(Color::Muted).into_node(context)
    }
}

pub struct Headline {
    text: SharedString,
    size: HeadlineSize,
    color: Color,
}

impl Headline {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            size: HeadlineSize::default(),
            color: Color::Default,
        }
    }

    pub fn size(mut self, size: HeadlineSize) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

impl Element for Headline {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let label_size = match self.size {
            HeadlineSize::XSmall => LabelSize::Small,
            HeadlineSize::Small => LabelSize::Default,
            HeadlineSize::Medium => LabelSize::Large,
            HeadlineSize::Large => LabelSize::Large,
            HeadlineSize::XLarge => LabelSize::Large,
        };
        Label::new(self.text)
            .size(label_size)
            .weight(FontWeight::BOLD)
            .color(self.color)
            .into_node(context)
    }
}

pub struct IconButton {
    button: Button,
    icon: IconName,
    shape: IconButtonShape,
    icon_size: Option<IconSize>,
    icon_color: Option<Color>,
    selected_icon: Option<IconName>,
    selected_icon_color: Option<Color>,
    indicator: Option<Indicator>,
    indicator_border_color: Option<gpui_api::Hsla>,
    alpha: Option<f32>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        Self {
            button: Button::new(id, ""),
            icon,
            shape: IconButtonShape::Wide,
            icon_size: None,
            icon_color: None,
            selected_icon: None,
            selected_icon_color: None,
            indicator: None,
            indicator_border_color: None,
            alpha: None,
        }
    }

    pub fn shape(mut self, shape: IconButtonShape) -> Self {
        self.shape = shape;
        self
    }

    pub fn icon_size(mut self, icon_size: IconSize) -> Self {
        self.icon_size = Some(icon_size);
        self
    }

    pub fn icon_color(mut self, icon_color: Color) -> Self {
        self.icon_color = Some(icon_color);
        self
    }

    pub fn alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }

    pub fn selected_icon(mut self, icon: impl Into<Option<IconName>>) -> Self {
        self.selected_icon = icon.into();
        self
    }

    pub fn selected_icon_color(mut self, color: impl Into<Option<Color>>) -> Self {
        self.selected_icon_color = color.into();
        self
    }

    pub fn indicator(mut self, indicator: Indicator) -> Self {
        self.indicator = Some(indicator);
        self
    }

    pub fn indicator_border_color(mut self, color: Option<gpui_api::Hsla>) -> Self {
        self.indicator_border_color = color;
        self
    }

    pub fn on_right_click(
        self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click(handler)
    }
}

impl Disableable for IconButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.button = self.button.disabled(disabled);
        self
    }
}

impl Toggleable for IconButton {
    fn toggle_state(mut self, selected: bool) -> Self {
        self.button = self.button.toggle_state(selected);
        self
    }
}

impl SelectableButton for IconButton {
    fn selected_style(mut self, style: ButtonStyle) -> Self {
        self.button = self.button.selected_style(style);
        self
    }
}

impl Clickable for IconButton {
    fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Self {
            button: self.button.on_click(handler),
            ..self
        }
    }

    fn cursor_style(mut self, cursor_style: CursorStyle) -> Self {
        self.button = self.button.cursor_style(cursor_style);
        self
    }
}

impl FixedWidth for IconButton {
    fn width(mut self, width: impl Into<DefiniteLength>) -> Self {
        self.button = self.button.width(width);
        self
    }

    fn full_width(mut self) -> Self {
        self.button = self.button.full_width();
        self
    }
}

impl ButtonCommon for IconButton {
    fn id(&self) -> &ElementId {
        &self.button.id
    }

    fn style(mut self, style: ButtonStyle) -> Self {
        self.button = self.button.style(style);
        self
    }

    fn size(mut self, size: ButtonSize) -> Self {
        self.button = self.button.size(size);
        self
    }

    fn tooltip(mut self, tooltip: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        self.button = self.button.tooltip(tooltip);
        self
    }

    fn tab_index(mut self, tab_index: impl Into<isize>) -> Self {
        self.button = self.button.tab_index(tab_index);
        self
    }

    fn layer(mut self, elevation: ElevationIndex) -> Self {
        self.button = self.button.layer(elevation);
        self
    }

    fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.button = self.button.track_focus(focus_handle);
        self
    }
}

impl VisibleOnHover for IconButton {
    fn visible_on_hover(mut self, group_name: impl Into<SharedString>) -> Self {
        self.button = self
            .button
            .invisible()
            .group_hover(group_name, |style| style.visible());
        self
    }
}

impl Element for IconButton {
    fn into_node(self, context: &mut RenderContext) -> UiNode {
        let mut button = self.button;
        let icon_name = if button.selected {
            self.selected_icon.unwrap_or(self.icon)
        } else {
            self.icon
        };
        let mut icon = Icon::new(icon_name);
        if let Some(size) = self.icon_size {
            icon = icon.size(size);
        }
        let icon_color = if button.selected {
            self.selected_icon_color.or(self.icon_color)
        } else {
            self.icon_color
        };
        if let Some(color) = icon_color {
            icon = icon.color(color);
        }
        button = button.start_icon(Some(icon));
        if let Some(alpha) = self.alpha {
            button = button.alpha(alpha);
        }
        let mut node = button.into_node(context);
        node.props.insert(
            "icon_button_shape".to_string(),
            serialize_icon_button_shape(self.shape).into(),
        );
        if let Some(color) = self.icon_color {
            node.props.insert(
                "button_icon_color".to_string(),
                serialize_color(color).into(),
            );
        }
        if let Some(icon) = self.selected_icon {
            let icon_name: &'static str = icon.into();
            node.props
                .insert("selected_button_icon".to_string(), icon_name.into());
        }
        if let Some(color) = self.selected_icon_color {
            node.props.insert(
                "selected_button_icon_color".to_string(),
                serialize_color(color).into(),
            );
        }
        if let Some(indicator) = self.indicator {
            let indicator_node = indicator.into_node(context);
            if let Some(gpui_api::StyleValue::Text(kind)) =
                indicator_node.props.get("indicator_kind")
            {
                node.props
                    .insert("indicator_kind".to_string(), kind.clone().into());
            }
            if let Some(gpui_api::StyleValue::Text(color)) =
                indicator_node.props.get("indicator_color")
            {
                node.props
                    .insert("indicator_color".to_string(), color.clone().into());
            }
            if let Some(gpui_api::StyleValue::Text(color)) =
                indicator_node.props.get("indicator_border_color")
            {
                node.props
                    .insert("indicator_border_color".to_string(), color.clone().into());
            }
            if let Some(icon_name) = indicator_node.text {
                node.props
                    .insert("indicator_icon".to_string(), icon_name.into());
            }
        }
        if let Some(color) = self.indicator_border_color {
            node.props.insert(
                "indicator_stroke_color".to_string(),
                serialize_hsla(color).into(),
            );
        }
        node
    }
}

fn serialize_label_size(size: LabelSize) -> String {
    match size {
        LabelSize::Default => "default".into(),
        LabelSize::Large => "large".into(),
        LabelSize::Small => "small".into(),
        LabelSize::XSmall => "x_small".into(),
        LabelSize::Custom(size) => format!("custom:{}", size.0),
    }
}

fn serialize_line_height_style(style: LineHeightStyle) -> String {
    match style {
        LineHeightStyle::TextLabel => "text_label".into(),
        LineHeightStyle::UiLabel => "ui_label".into(),
    }
}

fn serialize_button_size(size: ButtonSize) -> String {
    match size {
        ButtonSize::Large => "large".into(),
        ButtonSize::Medium => "medium".into(),
        ButtonSize::Default => "default".into(),
        ButtonSize::Compact => "compact".into(),
        ButtonSize::None => "none".into(),
    }
}

fn serialize_elevation(elevation: ElevationIndex) -> String {
    match elevation {
        ElevationIndex::Background => "background".into(),
        ElevationIndex::Surface => "surface".into(),
        ElevationIndex::EditorSurface => "editor_surface".into(),
        ElevationIndex::ElevatedSurface => "elevated_surface".into(),
        ElevationIndex::ModalSurface => "modal_surface".into(),
    }
}

fn serialize_keybinding_position(position: KeybindingPosition) -> String {
    match position {
        KeybindingPosition::Start => "start".into(),
        KeybindingPosition::End => "end".into(),
    }
}

fn serialize_divider_color(color: DividerColor) -> String {
    match color {
        DividerColor::Border => "border".into(),
        DividerColor::BorderFaded => "border_faded".into(),
        DividerColor::BorderVariant => "border_variant".into(),
    }
}

fn serialize_icon_button_shape(shape: IconButtonShape) -> String {
    match shape {
        IconButtonShape::Square => "square".into(),
        IconButtonShape::Wide => "wide".into(),
    }
}

fn serialize_hsla(color: gpui_api::Hsla) -> String {
    format!("{},{},{},{}", color.h, color.s, color.l, color.a)
}

fn serialize_button_style(style: ButtonStyle) -> String {
    match style {
        ButtonStyle::Filled => "filled".into(),
        ButtonStyle::Outlined => "outlined".into(),
        ButtonStyle::OutlinedGhost => "outlined_ghost".into(),
        ButtonStyle::Subtle => "subtle".into(),
        ButtonStyle::Transparent => "transparent".into(),
        ButtonStyle::Tinted(tint) => format!(
            "tinted:{}",
            match tint {
                TintColor::Accent => "accent",
                TintColor::Error => "error",
                TintColor::Warning => "warning",
                TintColor::Success => "success",
            }
        ),
        ButtonStyle::OutlinedCustom(_) => "outlined_custom".into(),
    }
}

fn serialize_icon_size(size: IconSize) -> String {
    match size {
        IconSize::Indicator => "indicator".into(),
        IconSize::XSmall => "x_small".into(),
        IconSize::Small => "small".into(),
        IconSize::Medium => "medium".into(),
        IconSize::XLarge => "x_large".into(),
        IconSize::Custom(size) => format!("custom:{}", size.0),
    }
}

fn serialize_color(color: Color) -> String {
    match color {
        Color::Default => "default".into(),
        Color::Accent => "accent".into(),
        Color::Conflict => "conflict".into(),
        Color::Created => "created".into(),
        Color::Debugger => "debugger".into(),
        Color::Deleted => "deleted".into(),
        Color::Disabled => "disabled".into(),
        Color::Error => "error".into(),
        Color::Hidden => "hidden".into(),
        Color::Hint => "hint".into(),
        Color::Ignored => "ignored".into(),
        Color::Info => "info".into(),
        Color::Modified => "modified".into(),
        Color::Muted => "muted".into(),
        Color::Placeholder => "placeholder".into(),
        Color::Selected => "selected".into(),
        Color::Success => "success".into(),
        Color::VersionControlAdded => "version_control_added".into(),
        Color::VersionControlConflict => "version_control_conflict".into(),
        Color::VersionControlDeleted => "version_control_deleted".into(),
        Color::VersionControlIgnored => "version_control_ignored".into(),
        Color::VersionControlModified => "version_control_modified".into(),
        Color::Warning => "warning".into(),
        Color::Custom(_) | Color::Player(_) => "default".into(),
    }
}
