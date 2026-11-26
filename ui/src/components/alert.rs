//! `Alert`: shadcn's alert, a bordered block with an icon, a title and a
//! description.
//!
//! The default variant is on the card in the card's colours with a muted
//! description; the destructive variant draws its title and icon in the
//! destructive colour and its description in it at nine tenths. The
//! description is the children, so it can hold more than a line of text
//! (a command in the monospace font on a line of its own). It takes no
//! focus. shadcn clamps the title to one line; here it wraps, since an
//! app's titles name a failure and its cause and are read whole.

use gpui::{
    AnyElement, App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, rems,
};
use gpui::InteractiveElement;

use crate::icon::Icon;
use crate::theme::{ActiveTheme, Tokens};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlertVariant {
    #[default]
    Default,
    Destructive,
}

/// An alert's colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertStyle {
    pub background: Hsla,
    pub border: Hsla,
    pub title: Hsla,
    pub icon: Hsla,
    pub description: Hsla,
}

/// shadcn's variants: `bg-card text-card-foreground` with
/// `text-muted-foreground` descriptions, or `text-destructive bg-card`
/// with `text-destructive/90` descriptions.
pub fn alert_style(variant: AlertVariant, t: &Tokens) -> AlertStyle {
    match variant {
        AlertVariant::Default => AlertStyle {
            background: t.card,
            border: t.border,
            title: t.card_foreground,
            icon: t.card_foreground,
            description: t.muted_foreground,
        },
        AlertVariant::Destructive => AlertStyle {
            background: t.card,
            border: t.border,
            title: t.destructive,
            icon: t.destructive,
            description: t.destructive.opacity(0.9),
        },
    }
}

#[derive(IntoElement)]
pub struct Alert {
    title: SharedString,
    icon: Icon,
    variant: AlertVariant,
    children: Vec<AnyElement>,
}

impl Alert {
    /// An alert with lucide `circle-alert`.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), icon: Icon::CircleAlert, variant: AlertVariant::Default, children: Vec::new() }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    pub fn variant(mut self, variant: AlertVariant) -> Self {
        self.variant = variant;
        self
    }

    /// A description of plain text.
    pub fn description(self, text: impl Into<SharedString>) -> Self {
        self.child(text.into())
    }
}

impl ParentElement for Alert {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Alert {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = &cx.ui_theme();
        let style = alert_style(self.variant, t);
        // rounded-lg border px-4 py-3 text-sm, a 1rem icon column and
        // gap-x-3 gap-y-0.5.
        div()
            .debug_selector(|| "alert".into())
            .flex()
            .items_start()
            .gap_3()
            .w_full()
            .min_w_0()
            .px_4()
            .py_3()
            .rounded(t.radius_lg())
            .border_1()
            .border_color(style.border)
            .bg(style.background)
            .text_size(rems(0.875))
            // translate-y-0.5: the icon on the title's first line.
            .child(div().pt_0p5().child(self.icon.size(rems(1.)).color(style.icon)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .flex_1()
                    .min_w_0()
                    .child(div().font_weight(FontWeight::MEDIUM).text_color(style.title).child(self.title))
                    .child(div().flex().flex_col().gap_1().text_color(style.description).children(self.children)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec's "Destructive alert": title and icon in the destructive
    /// colour; the default variant in the card's.
    #[test]
    fn the_destructive_variant_draws_in_the_destructive_colour() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let destructive = alert_style(AlertVariant::Destructive, &t);
            assert_eq!((destructive.title, destructive.icon), (t.destructive, t.destructive));
            assert_eq!(destructive.description, t.destructive.opacity(0.9));
            let default = alert_style(AlertVariant::Default, &t);
            assert_eq!((default.title, default.icon, default.description), (t.card_foreground, t.card_foreground, t.muted_foreground));
            assert_eq!((default.background, destructive.background), (t.card, t.card));
        }
    }
}
