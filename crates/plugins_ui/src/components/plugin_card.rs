use gpui::{AnyElement, prelude::*};
use ui::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginCardVariant {
    Default,
    Development,
}

#[derive(IntoElement)]
pub struct PluginCard {
    variant: PluginCardVariant,
    children: Vec<AnyElement>,
}

impl PluginCard {
    pub fn new() -> Self {
        Self {
            variant: PluginCardVariant::Default,
            children: Vec::new(),
        }
    }

    pub fn development() -> Self {
        Self {
            variant: PluginCardVariant::Development,
            children: Vec::new(),
        }
    }
}

impl ParentElement for PluginCard {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for PluginCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (background, border_color) = match self.variant {
            PluginCardVariant::Default => (
                cx.theme().colors().elevated_surface_background.opacity(0.5),
                cx.theme().colors().border_variant,
            ),
            PluginCardVariant::Development => (
                cx.theme()
                    .colors()
                    .elevated_surface_background
                    .blend(cx.theme().status().warning_background.opacity(0.12)),
                cx.theme().status().warning_border,
            ),
        };

        div().w_full().child(
            v_flex()
                .w_full()
                .mt_4()
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(border_color)
                .bg(background)
                .children(self.children),
        )
    }
}
