use gpui_plugin as gpui;
use std::time::Duration;
use ui_plugin as ui;

use gpui::prelude::*;
use ui::prelude::*;

gpui::actions!(deploy_panel, [TriggerDeploy]);

struct PreviewComponent;

impl ui::Component for PreviewComponent {
    fn scope() -> ui::ComponentScope {
        ui::ComponentScope::Status
    }

    fn preview(_window: &mut gpui::Window, _cx: &mut gpui::App) -> Option<gpui::AnyElement> {
        Some(
            ui::example_group_with_title(
                "Preview",
                vec![ui::single_example(
                    "Default",
                    gpui::IntoElement::into_any_element(ui::Label::new("Preview")),
                )],
            )
            .into_any_element(),
        )
    }
}

struct DeployPanel;

impl gpui::Render for DeployPanel {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        ui::v_flex()
            .bg(cx.theme().colors().surface_background)
            .gap_4()
            .p_4()
            .when(true, |this| this.rounded_md())
            .child(
                gpui::div()
                    .text_ui_sm(cx)
                    .border_primary(cx)
                    .px(DynamicSpacing::Base04.rems(cx))
                    .w(vw(0.5, _window))
                    .child("Metrics"),
            )
            .child(
                Headline::new("Overview")
                    .size(HeadlineSize::Small)
                    .color(Color::Muted),
            )
            .child(
                Label::new("Deploy Log")
                    .size(LabelSize::Large)
                    .weight(gpui::FontWeight::BOLD)
                    .color(Color::Muted)
                    .single_line(),
            )
            .child(LoadingLabel::new("Deploying").size(LabelSize::Small))
            .child(
                ui::h_flex()
                    .gap_1()
                    .child(Label::new("Row"))
                    .child(Divider::vertical())
                    .child(Label::new("Actions")),
            )
            .child(
                h_group()
                    .child(Label::new("Primary"))
                    .child(Button::new("secondary", "Secondary").visible_on_hover("actions")),
            )
            .child(
                Button::new("deploy", "Deploy Now")
                    .style(ButtonStyle::Filled)
                    .size(ButtonSize::Large)
                    .color(Color::Accent)
                    .label_size(Some(LabelSize::Small))
                    .width(gpui::px(220.))
                    .tooltip(|_, cx| cx.new(|_| DeployPanel).into())
                    .tab_index(2isize)
                    .layer(ElevationIndex::ElevatedSurface)
                    .track_focus(&gpui::FocusHandle)
                    .end_icon(Icon::new(IconName::ArrowRight))
                    .key_binding(Some(KeyBinding::for_action(&TriggerDeploy, cx)))
                    .key_binding_position(KeybindingPosition::Start)
                    .toggle_state(true)
                    .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                    .selected_label(Some("Deploying"))
                    .truncate(true)
                    .on_click(cx.listener(|_this, _event, _window, _cx| {})),
            )
            .child(ui::Clickable::on_click(
                ui::ButtonCommon::size(
                    IconButton::new("deploy-icon", IconName::ArrowRight)
                        .shape(IconButtonShape::Square)
                        .style(ButtonStyle::Subtle),
                    ButtonSize::Compact,
                )
                .indicator(Indicator::dot().color(Color::Success))
                .visible_on_hover("deploy_group"),
                cx.listener(|_this, _event, _window, _cx| {}),
            ))
            .child(Indicator::bar().color(Color::Warning))
            .child(
                Divider::horizontal_dashed()
                    .inset()
                    .color(DividerColor::Border),
            )
            .child(vertical_divider().color(DividerColor::BorderVariant))
            .child(
                ProgressBar::new("deploy-progress", 9.0, 100.0, cx)
                    .value(12.0)
                    .bg_color(cx.theme().colors().background)
                    .fg_color(cx.theme().status().info)
                    .over_color(cx.theme().status().error),
            )
            .child(AnyIcon::from(
                Icon::new(IconName::ArrowRight).size(IconSize::Small),
            ))
            .child(AnyIcon::from(gpui::AnimationExt::with_animation(
                Icon::new(IconName::ArrowRight),
                "animated-icon",
                gpui::Animation::new(Duration::from_millis(120)),
                |this, _delta| this,
            )))
            .child(
                IconWithIndicator::new(
                    Icon::new(IconName::ArrowRight).size(IconSize::Small),
                    Some(Indicator::dot().color(Color::Accent)),
                )
                .indicator_border_color(Some(cx.theme().colors().border)),
            )
            .child(
                ButtonLike::new("custom-button-like")
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Default)
                    .tooltip(|_, cx| cx.new(|_| DeployPanel).into())
                    .tab_index(3isize)
                    .layer(ElevationIndex::Surface)
                    .track_focus(&gpui::FocusHandle)
                    .visible_on_hover("deploy_group")
                    .child(Label::new("Custom"))
                    .on_click(cx.listener(|_this, _event, _window, _cx| {})),
            )
            .child(
                ButtonLink::new("Docs", "https://example.com")
                    .label_size(LabelSize::Small)
                    .label_color(Color::Accent),
            )
            .child(
                CopyButton::new("copy-log", "deploy-log")
                    .icon_size(IconSize::XSmall)
                    .tooltip_label("Copy deploy log")
                    .visible_on_hover("deploy_group")
                    .custom_on_click(|_window, _cx| {}),
            )
            .child(
                SplitButton::new(
                    ButtonLike::new("split-left").child(Label::new("Split")),
                    gpui::IntoElement::into_any_element(
                        IconButton::new("split-right", IconName::ArrowRight)
                            .shape(IconButtonShape::Square),
                    ),
                )
                .style(SplitButtonStyle::Outlined),
            )
            .child(
                KeybindingHint::with_prefix(
                    "Run",
                    KeyBinding::for_action(&TriggerDeploy, cx),
                    cx.theme().colors().background,
                )
                .size(Some(gpui::px(12.0))),
            )
            .child(
                CircularProgress::new(25.0, 100.0, gpui::px(32.0), cx)
                    .stroke_width(gpui::px(3.0))
                    .bg_color(cx.theme().colors().border_variant)
                    .progress_color(cx.theme().status().info),
            )
            .child(ui::DefaultAnimations::animate_in_from_bottom(
                gpui::div().size_4().bg(cx.theme().status().info),
                true,
            ))
            .child(AnyIcon::from(
                ui::CommonAnimationExt::with_rotate_animation(Icon::new(IconName::ArrowRight), 1),
            ))
            .child(
                checkbox("deploy-checkbox", ToggleState::Selected)
                    .label("Confirm deploy")
                    .label_size(LabelSize::Small)
                    .label_color(Color::Muted)
                    .style(ToggleStyle::Ghost)
                    .fill()
                    .on_click(|_state, _window, _cx| {}),
            )
            .child(
                switch("deploy-switch", ToggleState::Unselected)
                    .label("Enable retries")
                    .label_position(Some(SwitchLabelPosition::End))
                    .label_size(LabelSize::Small)
                    .key_binding(Some(KeyBinding::for_action(&TriggerDeploy, cx)))
                    .color(SwitchColor::Accent)
                    .tab_index(4isize)
                    .on_click(|_state, _window, _cx| {}),
            )
            .child(
                SwitchField::new(
                    "deploy-switch-field",
                    Some("Auto refresh"),
                    Some("Refresh metrics after deploy".into()),
                    ToggleState::Selected,
                    |_state, _window, _cx| {},
                )
                .color(SwitchColor::Accent)
                .tab_index(5),
            )
    }
}

#[test]
fn ui_plugin_exposes_ui_style_surface() {
    let mut runtime = gpui::Runtime::new();
    let panel = runtime.new_entity(|_| DeployPanel);
    let render_output = runtime.render_root(&panel).expect("render succeeds");

    assert_eq!(render_output.tree.kind, gpui::NodeKind::Div);
    assert_eq!(
        <PreviewComponent as ui::Component>::scope(),
        ui::ComponentScope::Status
    );
}
