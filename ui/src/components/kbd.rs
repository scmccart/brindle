//! `Kbd`: shadcn's keycaps, drawn from a keymap string.
//!
//! `Kbd::new("ctrl-shift-t")` shows one keycap per key (`ctrl`, `shift`,
//! `t`), with the names unchanged: it draws the strings the keymap prints,
//! and mapping `cmd` or `super` per platform is the keymap's job. Strokes
//! separated by a space (`ctrl-k ctrl-c`) are separate groups of keycaps
//! with a wider gap between them. A key that is a dash (`ctrl--`) is the
//! dash, not a separator.
//!
//! A keycap handles no pointer events and takes no focus.

use gpui::{
    App, FontWeight, Hsla, IntoElement, ParentElement, Refineable, Rems, RenderOnce, SharedString, StyleRefinement,
    Styled, Window, div, rems,
};

use crate::theme::{ActiveTheme, Tokens};

/// A keycap group's resolved look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KbdStyle {
    /// `bg-muted`.
    pub background: Hsla,
    /// `text-muted-foreground`.
    pub foreground: Hsla,
    /// `h-5`.
    pub height: Rems,
    /// `min-w-5`: a single character is a square.
    pub min_width: Rems,
    /// `px-1`.
    pub padding_x: Rems,
    /// `rounded-sm`.
    pub radius: Rems,
    /// `text-xs`.
    pub text_size: Rems,
    /// `gap-1`: between the keycaps of one stroke.
    pub key_gap: Rems,
    /// Between two strokes.
    pub stroke_gap: Rems,
}

/// shadcn's `kbd` (`bg-muted text-muted-foreground h-5 min-w-5 rounded-sm
/// px-1 text-xs font-medium`), in the colours of `t`.
pub fn kbd_style(t: &Tokens) -> KbdStyle {
    KbdStyle {
        background: t.muted,
        foreground: t.muted_foreground,
        height: rems(1.25),
        min_width: rems(1.25),
        padding_x: rems(0.25),
        radius: t.radius_sm(),
        text_size: rems(0.75),
        key_gap: rems(0.25),
        stroke_gap: rems(0.5),
    }
}

/// Splits a keymap string into strokes of key names: strokes are separated
/// by whitespace and the keys of one stroke by `-`. A dash with no key
/// before it is the key itself, so `ctrl--` is `ctrl` and `-`.
pub fn keycaps(keys: &str) -> Vec<Vec<String>> {
    keys.split_whitespace()
        .map(|stroke| {
            let mut caps = Vec::new();
            let mut current = String::new();
            for c in stroke.chars() {
                if c == '-' && !current.is_empty() {
                    caps.push(std::mem::take(&mut current));
                } else {
                    current.push(c);
                }
            }
            if !current.is_empty() {
                caps.push(current);
            }
            caps
        })
        .filter(|caps| !caps.is_empty())
        .collect()
}

/// Keycaps for a keymap string.
#[derive(IntoElement)]
pub struct Kbd {
    keys: SharedString,
    colors: Option<(Hsla, Hsla)>,
    style: StyleRefinement,
}

impl Kbd {
    pub fn new(keys: impl Into<SharedString>) -> Self {
        Self { keys: keys.into(), colors: None, style: StyleRefinement::default() }
    }

    /// Draws the keycaps in `background` and `foreground` instead of the
    /// muted tokens (a tooltip's inverted keycaps).
    pub fn colors(mut self, background: Hsla, foreground: Hsla) -> Self {
        self.colors = Some((background, foreground));
        self
    }
}

impl Styled for Kbd {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Kbd {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = kbd_style(cx.ui_theme());
        let (background, foreground) = self.colors.unwrap_or((style.background, style.foreground));
        let mut group = div().flex().flex_none().items_center().gap(style.stroke_gap);
        for stroke in keycaps(&self.keys) {
            let mut strokes = div().flex().items_center().gap(style.key_gap);
            for cap in stroke {
                strokes = strokes.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .h(style.height)
                        .min_w(style.min_width)
                        .px(style.padding_x)
                        .rounded(style.radius)
                        .bg(background)
                        .text_color(foreground)
                        .text_size(style.text_size)
                        .font_weight(FontWeight::MEDIUM)
                        .whitespace_nowrap()
                        .child(cap),
                );
            }
            group = group.child(strokes);
        }
        group.style().refine(&self.style);
        group
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(keys: &str) -> Vec<Vec<String>> {
        keycaps(keys)
    }

    fn owned(strokes: &[&[&str]]) -> Vec<Vec<String>> {
        strokes.iter().map(|s| s.iter().map(|k| k.to_string()).collect()).collect()
    }

    #[test]
    fn a_stroke_splits_on_dashes() {
        assert_eq!(split("ctrl-shift-t"), owned(&[&["ctrl", "shift", "t"]]));
        assert_eq!(split("ctrl-alt-1"), owned(&[&["ctrl", "alt", "1"]]));
    }

    #[test]
    fn strokes_split_on_spaces() {
        assert_eq!(split("ctrl-k ctrl-c"), owned(&[&["ctrl", "k"], &["ctrl", "c"]]));
    }

    #[test]
    fn a_lone_key_is_one_keycap() {
        assert_eq!(split("enter"), owned(&[&["enter"]]));
        assert_eq!(split("f11"), owned(&[&["f11"]]));
    }

    #[test]
    fn a_dash_key_is_the_dash() {
        assert_eq!(split("ctrl--"), owned(&[&["ctrl", "-"]]));
        assert_eq!(split("-"), owned(&[&["-"]]));
        assert_eq!(split("ctrl-shift--"), owned(&[&["ctrl", "shift", "-"]]));
    }

    #[test]
    fn empty_and_blank_strings_have_no_keycaps() {
        assert!(split("").is_empty());
        assert!(split("  ").is_empty());
    }

    #[test]
    fn extra_space_between_strokes_is_ignored() {
        assert_eq!(split("  ctrl-k   c "), owned(&[&["ctrl", "k"], &["c"]]));
    }

    #[test]
    fn keycaps_are_muted_xs_squares_for_one_character() {
        let t = Tokens::neutral_light();
        let s = kbd_style(&t);
        assert_eq!((s.background, s.foreground), (t.muted, t.muted_foreground));
        assert_eq!((s.height, s.min_width), (rems(1.25), rems(1.25)));
        assert_eq!((s.text_size, s.radius), (rems(0.75), t.radius_sm()));
        assert!(s.stroke_gap.0 > s.key_gap.0, "strokes are further apart than the keys of one");
    }

    #[test]
    fn colours_override_the_tokens() {
        let kbd = Kbd::new("a").colors(gpui::black(), gpui::white());
        assert_eq!(kbd.colors, Some((gpui::black(), gpui::white())));
        assert_eq!(Kbd::new("a").colors, None);
    }
}
