//! `Shimmer`: AI Elements' shimmering text, a muted line with a lighter
//! band at the position its owner gives.
//!
//! gpui can't fill text with a gradient, so the band is colour runs: each
//! character takes one of four [`Level`]s from `muted_foreground` to
//! `foreground` by its distance to the band's centre ([`levels`]). Mixing
//! only between those two tokens keeps every character at least as legible
//! as the muted foreground. The owner's clock moves the band; the
//! component asks for no animation frames and takes no focus. One line,
//! cut with an ellipsis.

use std::ops::Range;

use gpui::{
    App, HighlightStyle, Hsla, IntoElement, ParentElement, RenderOnce, Rgba, SharedString, Styled, StyledText, Window,
    div,
};

use crate::theme::ActiveTheme;

/// How light a character of a shimmer is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// `muted_foreground`, outside the band.
    Muted,
    Low,
    High,
    /// `foreground`, at the band's centre.
    Full,
}

impl Level {
    /// How far from `muted_foreground` towards `foreground` it is drawn.
    pub fn weight(self) -> f32 {
        match self {
            Self::Muted => 0.,
            Self::Low => 1. / 3.,
            Self::High => 2. / 3.,
            Self::Full => 1.,
        }
    }

    /// The level of a character whose centre is `distance` characters from
    /// the band's.
    fn at(distance: f32) -> Self {
        if distance < 1. {
            Self::Full
        } else if distance < 2. {
            Self::High
        } else if distance < 3. {
            Self::Low
        } else {
            Self::Muted
        }
    }
}

/// The runs of a text of `len` characters with the band centred at
/// `position` (in characters from its start), as character ranges, equal
/// neighbours merged: at most seven, the band's three levels either side of
/// its centre between muted stretches.
pub fn levels(len: usize, position: f32) -> Vec<(Range<usize>, Level)> {
    let mut runs: Vec<(Range<usize>, Level)> = Vec::new();
    for index in 0..len {
        let level = Level::at((index as f32 + 0.5 - position).abs());
        match runs.last_mut() {
            Some((range, last)) if *last == level => range.end = index + 1,
            Some(_) | None => runs.push((index..index + 1, level)),
        }
    }
    runs
}

/// `from` mixed towards `to` by `t` in `[0, 1]`, channel by channel.
fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(from), Rgba::from(to));
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Rgba { r: lerp(a.r, b.r), g: lerp(a.g, b.g), b: lerp(a.b, b.b), a: lerp(a.a, b.a) }.into()
}

/// The colour of `level` between `muted` and `full`.
pub fn level_color(level: Level, muted: Hsla, full: Hsla) -> Hsla {
    match level {
        Level::Muted => muted,
        Level::Full => full,
        Level::Low | Level::High => mix(muted, full, level.weight()),
    }
}

#[derive(IntoElement)]
pub struct Shimmer {
    text: SharedString,
    position: f32,
}

impl Shimmer {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), position: f32::NEG_INFINITY }
    }

    /// The band's centre, in characters from the text's start (before the
    /// start, the whole line is muted).
    pub fn position(mut self, position: f32) -> Self {
        self.position = position;
        self
    }
}

impl RenderOnce for Shimmer {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let starts: Vec<usize> =
            self.text.char_indices().map(|(at, _)| at).chain(std::iter::once(self.text.len())).collect();
        let highlights: Vec<(Range<usize>, HighlightStyle)> = levels(starts.len() - 1, self.position)
            .into_iter()
            .filter(|(_, level)| *level != Level::Muted)
            .map(|(range, level)| {
                let color = level_color(level, t.muted_foreground, t.foreground);
                (starts[range.start]..starts[range.end], HighlightStyle { color: Some(color), ..Default::default() })
            })
            .collect();
        div()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .whitespace_normal()
            .line_clamp(1)
            .text_ellipsis()
            .text_color(t.muted_foreground)
            .child(StyledText::new(self.text).with_highlights(highlights))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Tokens;

    fn shown(runs: &[(Range<usize>, Level)]) -> Vec<(usize, usize, Level)> {
        runs.iter().map(|(range, level)| (range.start, range.end, *level)).collect()
    }

    /// The spec's "The band moves with the owner's position": at the start
    /// the first characters are lightest, in the middle the middle ones,
    /// and the rest stay muted.
    #[test]
    fn the_band_follows_its_position() {
        // `Working…` is 8 characters.
        assert_eq!(
            shown(&levels(8, 0.)),
            [(0, 1, Level::Full), (1, 2, Level::High), (2, 3, Level::Low), (3, 8, Level::Muted)]
        );
        assert_eq!(
            shown(&levels(8, 4.)),
            [
                (0, 1, Level::Muted),
                (1, 2, Level::Low),
                (2, 3, Level::High),
                (3, 5, Level::Full),
                (5, 6, Level::High),
                (6, 7, Level::Low),
                (7, 8, Level::Muted),
            ]
        );
        assert_eq!(
            shown(&levels(8, 8.)),
            [(0, 5, Level::Muted), (5, 6, Level::Low), (6, 7, Level::High), (7, 8, Level::Full)]
        );
    }

    /// The spec's "Outside the text": before the start or past the end,
    /// every character is muted; an empty text has no runs.
    #[test]
    fn outside_the_text_every_character_is_muted() {
        assert_eq!(shown(&levels(8, -3.)), [(0, 8, Level::Muted)]);
        assert_eq!(shown(&levels(8, 11.)), [(0, 8, Level::Muted)]);
        assert_eq!(shown(&levels(8, f32::NEG_INFINITY)), [(0, 8, Level::Muted)]);
        assert!(levels(0, 0.).is_empty());
        for position in [-3., 0., 2.5, 4., 7.9, 11.] {
            assert!(levels(40, position).len() <= 7, "{position}");
        }
    }

    /// No run is dimmer than the muted foreground: each level lies between
    /// it and the foreground, the muted one exactly on it, and lightness
    /// moves one way from muted to full in both neutral sets.
    #[test]
    fn no_run_is_dimmer_than_the_muted_foreground() {
        for t in [Tokens::neutral_light(), Tokens::neutral_dark()] {
            let (muted, full) = (t.muted_foreground, t.foreground);
            assert_eq!(level_color(Level::Muted, muted, full), muted);
            assert_eq!(level_color(Level::Full, muted, full), full);
            let lightness: Vec<f32> = [Level::Muted, Level::Low, Level::High, Level::Full]
                .into_iter()
                .map(|level| level_color(level, muted, full).l)
                .collect();
            let towards = (full.l - muted.l).signum();
            assert!(lightness.windows(2).all(|pair| (pair[1] - pair[0]) * towards >= 0.), "{lightness:?}");
        }
    }
}
