//! `Badge`: shadcn's Badge, outline variant, in one colour the owner gives
//! (its text and border), placed after a tab trigger's label
//! ([`TabTrigger::badge`](crate::TabTrigger::badge)) or a card header's
//! title ([`CardHeader::badge`](crate::CardHeader::badge)).
//!
//! Beside a title it gives way in two steps: the title is cut first, down
//! to its first [`TITLE_KEEPS`] characters, and only then is the badge cut
//! with an ellipsis. Its height fits the 28px header and the 30px face, so
//! neither grows. Unlike shadcn's, which never shrinks (`shrink-0`), it can
//! be cut; the crate's README lists the difference.

use gpui::{
    App, Div, FontWeight, Hsla, ParentElement, Pixels, Rems, SharedString, Styled, TextRun, Window, div, font, px,
    rems,
};

use crate::components::tabs::trigger_label;
use crate::theme::{ActiveFont, ActiveTheme};

/// How many of a title's characters stay before a badge beside it is cut.
pub const TITLE_KEEPS: usize = 8;

/// How much faster a badge shrinks than the title beside it, once the title
/// is down to what it keeps: the badge is cut first, and the title takes up
/// the rest only when the badge has reached its border. (The title keeps a
/// shrink factor of 1, so on its own it takes all of the rest: a factor
/// under 1 would take only that share of it.)
const BADGE_SHRINK: f32 = 1000.;

/// A badge's label and colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    label: SharedString,
    color: Hsla,
}

impl Badge {
    /// A badge reading `label`, its text and border in `color`.
    pub fn new(label: impl Into<SharedString>, color: impl Into<Hsla>) -> Self {
        Self { label: label.into(), color: color.into() }
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }

    pub fn color(&self) -> Hsla {
        self.color
    }

    /// The badge as a flex item that shrinks (its label cut with an
    /// ellipsis) after the title beside it has given way.
    fn element(self, cx: &App) -> Div {
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
            .border_color(self.color)
            .text_color(self.color)
            .text_size(rems(0.75))
            .line_height(rems(1.))
            .font_weight(FontWeight::MEDIUM)
            .child(trigger_label(self.label))
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
) -> (Div, Option<Div>) {
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
    fn a_badge_keeps_its_label_and_colour() {
        let badge = Badge::new("root", gpui::red());
        assert_eq!(badge.label().as_ref(), "root");
        assert_eq!(badge.color(), gpui::red());
    }
}
