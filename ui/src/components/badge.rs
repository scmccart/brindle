//! `Badge`: shadcn's Badge, outline variant, in one colour the owner gives
//! (its text and border), placed after a tab trigger's label
//! ([`TabTrigger::badge`](crate::TabTrigger::badge)) or a card header's
//! title ([`CardHeader::badge`](crate::CardHeader::badge)). An icon badge
//! ([`Badge::icon`]) is the icon alone in that colour, with no border, and
//! either kind can carry a tooltip.
//!
//! Beside a title it gives way in two steps: the title is cut first, down
//! to its first [`TITLE_KEEPS`] characters, and only then is a label badge
//! cut with an ellipsis (an icon badge keeps its size, so the title goes on
//! giving way). Its height fits the 28px header and the 30px face, so
//! neither grows. Unlike shadcn's, which never shrinks (`shrink-0`), a label
//! badge can be cut; the crate's README lists the difference.

use gpui::{
    AnyElement, App, Div, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Rems, SharedString,
    Styled, TextRun, Window, div, font, px, rems,
};

use crate::components::tabs::trigger_label;
use crate::components::tooltip::{Tooltip, TooltipFor};
use crate::icon::{Icon, LEGIBLE_ICON_STROKE};
use crate::theme::{ActiveFont, ActiveTheme};

/// How many of a title's characters stay before a badge beside it is cut.
pub const TITLE_KEEPS: usize = 8;

/// How much faster a badge shrinks than the title beside it, once the title
/// is down to what it keeps: the badge is cut first, and the title takes up
/// the rest only when the badge has reached its border. (The title keeps a
/// shrink factor of 1, so on its own it takes all of the rest: a factor
/// under 1 would take only that share of it.)
const BADGE_SHRINK: f32 = 1000.;

/// The size an icon badge's icon is drawn at: shadcn's `size-4`, as the
/// strip's other icons, at their legible stroke.
const BADGE_ICON_SIZE: Rems = Rems(1.);

/// What a badge shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BadgeContent {
    /// Text in an outlined, rounded box.
    Label(SharedString),
    /// An icon alone, no box.
    Icon(Icon),
}

/// A badge's content, colour and optional tooltip.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    content: BadgeContent,
    color: Hsla,
    tooltip: Option<SharedString>,
}

impl Badge {
    /// A badge reading `label`, its text and border in `color`.
    pub fn new(label: impl Into<SharedString>, color: impl Into<Hsla>) -> Self {
        Self { content: BadgeContent::Label(label.into()), color: color.into(), tooltip: None }
    }

    /// A badge that is `icon` alone, drawn in `color`.
    pub fn icon(icon: Icon, color: impl Into<Hsla>) -> Self {
        Self { content: BadgeContent::Icon(icon), color: color.into(), tooltip: None }
    }

    /// A tooltip shown while the pointer rests on the badge.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn content(&self) -> &BadgeContent {
        &self.content
    }

    pub fn color(&self) -> Hsla {
        self.color
    }

    pub fn tooltip_text(&self) -> Option<&SharedString> {
        self.tooltip.as_ref()
    }

    /// The badge as a flex item: a label badge shrinks (its label cut with
    /// an ellipsis) after the title beside it has given way, and an icon
    /// badge keeps its size.
    fn element(self, cx: &App) -> AnyElement {
        let Self { content, color, tooltip } = self;
        let element = match content {
            BadgeContent::Label(label) => {
                let mut element = div();
                element.style().flex_shrink = Some(BADGE_SHRINK);
                element
                    .flex()
                    .min_w_0()
                    .overflow_hidden()
                    .items_center()
                    .px(rems(0.375))
                    .py(px(1.))
                    .rounded(cx.ui_theme().radius_md())
                    .border_1()
                    .border_color(color)
                    .text_color(color)
                    .text_size(rems(0.75))
                    .line_height(rems(1.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(trigger_label(label))
            }
            BadgeContent::Icon(icon) => div()
                .flex()
                .flex_none()
                .items_center()
                .child(icon.size(BADGE_ICON_SIZE).color(color).min_stroke(LEGIBLE_ICON_STROKE)),
        };
        // Drawn in the theme scope the badge is drawn in, if any.
        let scope = cx.scoped_theme();
        match tooltip {
            // The id is the badge's own, under its trigger's or header's.
            Some(text) => element
                .id("badge")
                .tooltip_with(move |_, _| Tooltip::new(text.clone()).in_scope(scope))
                .into_any_element(),
            None => element.into_any_element(),
        }
    }
}

/// What of `title` stays before a badge beside it is cut: all of it when it
/// has at most `keep` characters, else its first `keep` and an ellipsis
/// (which the cut title shows in their place).
pub fn kept_title(title: &str, keep: usize) -> SharedString {
    if title.chars().count() <= keep {
        return SharedString::from(title.to_owned());
    }
    let mut kept: String = title.chars().take(keep).collect();
    kept.push('…');
    kept.into()
}

/// The text style a title is drawn in, for measuring what of it stays.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TitleText {
    pub size: Rems,
    pub weight: FontWeight,
}

/// A one-line title (`trigger_label`) followed by `badge`, if any, for a
/// flex row: with a badge, the title starts at the width of what it keeps
/// ([`kept_title`], measured in `text`), grows into the room left up to its
/// whole width, so the badge sits right after it, and shrinks past what it
/// keeps only after the badge has. Whatever follows them in the row should
/// take the room left over with an auto margin, since the title no longer
/// fills it.
pub(crate) fn title_and_badge(
    title: SharedString,
    badge: Option<Badge>,
    text: TitleText,
    window: &mut Window,
    cx: &App,
) -> (Div, Option<AnyElement>) {
    let Some(badge) = badge else {
        return (trigger_label(title), None);
    };
    let kept = text_width(kept_title(&title, TITLE_KEEPS), text, window, cx);
    // A pixel over, so rounding never cuts a title that fits.
    let whole = text_width(title.clone(), text, window, cx) + px(1.);
    let label = trigger_label(title).flex_basis(kept.min(whole)).max_w(whole);
    (label, Some(badge.element(cx)))
}

/// How wide `text` is in the chrome's font at `style`.
fn text_width(text: SharedString, style: TitleText, window: &mut Window, cx: &App) -> Pixels {
    let size = style.size.to_pixels(window.rem_size());
    let run = TextRun {
        len: text.len(),
        font: gpui::Font { weight: style.weight, ..font(cx.ui_font()) },
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, size, &[run], None).width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_keeps_its_first_eight_characters() {
        assert_eq!(kept_title("bash", TITLE_KEEPS).as_ref(), "bash");
        assert_eq!(kept_title("12345678", TITLE_KEEPS).as_ref(), "12345678");
        assert_eq!(kept_title("123456789", TITLE_KEEPS).as_ref(), "12345678…");
        assert_eq!(kept_title("ééééééééé", TITLE_KEEPS).as_ref(), "éééééééé…");
        assert_eq!(kept_title("", TITLE_KEEPS).as_ref(), "");
    }

    #[test]
    fn a_badge_keeps_its_content_colour_and_tooltip() {
        let badge = Badge::new("web", gpui::red());
        assert_eq!(badge.content(), &BadgeContent::Label("web".into()));
        assert_eq!(badge.color(), gpui::red());
        assert_eq!(badge.tooltip_text(), None);
        let warning = Badge::icon(Icon::TriangleAlert, gpui::yellow()).tooltip("Running as root");
        assert_eq!(warning.content(), &BadgeContent::Icon(Icon::TriangleAlert));
        assert_eq!(warning.color(), gpui::yellow());
        assert_eq!(warning.tooltip_text().map(|text| text.as_ref()), Some("Running as root"));
    }
}
