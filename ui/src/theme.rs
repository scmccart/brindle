//! Design tokens, after shadcn/ui's CSS variables, and the active set.
//!
//! [`Tokens`] holds one set of colours and the corner radius. The app picks a
//! set, puts it in the [`UiTheme`] global, and components read it through
//! [`ActiveTheme::ui_theme`]. Light and dark are separate sets; a set's
//! [`Tokens::dark`] hint says which it is, for the few colours shadcn picks
//! with a `dark:` variant instead of a token.
//!
//! Hover and pressed states aren't tokens. Components fade a token instead, as
//! shadcn does (`bg-primary/90` is `primary.opacity(0.9)`).

use gpui::{App, Global, Hsla, Rems, hsla, rems, rgb};

/// One set of design tokens. Colours come in background/foreground pairs, as
/// in shadcn/ui, plus the border, input and ring colours and the radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub background: Hsla,
    pub foreground: Hsla,
    pub card: Hsla,
    pub card_foreground: Hsla,
    pub popover: Hsla,
    pub popover_foreground: Hsla,
    pub primary: Hsla,
    pub primary_foreground: Hsla,
    pub secondary: Hsla,
    pub secondary_foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    pub destructive: Hsla,
    /// Text on a destructive background. shadcn v4 dropped this token and
    /// writes `text-white`; it is kept so the colour comes from the set.
    pub destructive_foreground: Hsla,
    pub border: Hsla,
    /// The border of form controls.
    pub input: Hsla,
    /// The focus ring.
    pub ring: Hsla,
    /// The base corner radius (`--radius`); see [`Tokens::radius_md`] and
    /// its siblings for the steps components use.
    pub radius: Rems,
    /// Whether this is a dark set: a hint, not a colour. It is set by the
    /// token source, from the background's luminance for derived sets.
    /// Only the switch's thumb reads it (shadcn's `dark:` thumb colours,
    /// without which an unchecked thumb in `background` all but vanishes on
    /// a dark set's translucent track); shadcn's other `dark:` refinements
    /// are left out.
    pub dark: bool,
}

/// A colour from a `0xRRGGBB` constant.
fn hex(rgb_hex: u32) -> Hsla {
    rgb(rgb_hex).into()
}

/// shadcn's radius steps are `--radius` ± a few pixels. They are taken in
/// rems at the 16px root size (4px = 0.25rem), so they scale with the window's
/// text size like everything else.
const RADIUS_STEP: f32 = 0.25;

impl Tokens {
    /// shadcn/ui's neutral theme, light (`:root`).
    ///
    /// The colours are shadcn's oklch values converted to sRGB once; the
    /// comment on each line is the source value.
    pub fn neutral_light() -> Self {
        Self {
            background: hex(0xffffff),             // oklch(1 0 0)
            foreground: hex(0x0a0a0a),             // oklch(0.145 0 0)
            card: hex(0xffffff),                   // oklch(1 0 0)
            card_foreground: hex(0x0a0a0a),        // oklch(0.145 0 0)
            popover: hex(0xffffff),                // oklch(1 0 0)
            popover_foreground: hex(0x0a0a0a),     // oklch(0.145 0 0)
            primary: hex(0x171717),                // oklch(0.205 0 0)
            primary_foreground: hex(0xfafafa),     // oklch(0.985 0 0)
            secondary: hex(0xf5f5f5),              // oklch(0.97 0 0)
            secondary_foreground: hex(0x171717),   // oklch(0.205 0 0)
            muted: hex(0xf5f5f5),                  // oklch(0.97 0 0)
            muted_foreground: hex(0x737373),       // oklch(0.556 0 0)
            accent: hex(0xf5f5f5),                 // oklch(0.97 0 0)
            accent_foreground: hex(0x171717),      // oklch(0.205 0 0)
            destructive: hex(0xe7000b),            // oklch(0.577 0.245 27.325), clamped to sRGB
            destructive_foreground: hex(0xffffff), // text-white
            border: hex(0xe5e5e5),                 // oklch(0.922 0 0)
            input: hex(0xe5e5e5),                  // oklch(0.922 0 0)
            ring: hex(0xa1a1a1),                   // oklch(0.708 0 0)
            radius: rems(0.625),
            dark: false,
        }
    }

    /// shadcn/ui's neutral theme, dark (`.dark`). Its border and input are
    /// white at low opacity, as in shadcn.
    pub fn neutral_dark() -> Self {
        Self {
            background: hex(0x0a0a0a),             // oklch(0.145 0 0)
            foreground: hex(0xfafafa),             // oklch(0.985 0 0)
            card: hex(0x171717),                   // oklch(0.205 0 0)
            card_foreground: hex(0xfafafa),        // oklch(0.985 0 0)
            popover: hex(0x171717),                // oklch(0.205 0 0)
            popover_foreground: hex(0xfafafa),     // oklch(0.985 0 0)
            primary: hex(0xe5e5e5),                // oklch(0.922 0 0)
            primary_foreground: hex(0x171717),     // oklch(0.205 0 0)
            secondary: hex(0x262626),              // oklch(0.269 0 0)
            secondary_foreground: hex(0xfafafa),   // oklch(0.985 0 0)
            muted: hex(0x262626),                  // oklch(0.269 0 0)
            muted_foreground: hex(0xa1a1a1),       // oklch(0.708 0 0)
            accent: hex(0x262626),                 // oklch(0.269 0 0)
            accent_foreground: hex(0xfafafa),      // oklch(0.985 0 0)
            destructive: hex(0xff6467),            // oklch(0.704 0.191 22.216), clamped to sRGB
            destructive_foreground: hex(0xffffff), // text-white
            border: hsla(0., 0., 1., 0.10),        // oklch(1 0 0 / 10%)
            input: hsla(0., 0., 1., 0.15),         // oklch(1 0 0 / 15%)
            ring: hex(0x737373),                   // oklch(0.556 0 0)
            radius: rems(0.625),
            dark: true,
        }
    }

    /// `rounded-sm`: radius − 4px.
    pub fn radius_sm(&self) -> Rems {
        rems((self.radius.0 - RADIUS_STEP).max(0.))
    }

    /// `rounded-md`: radius − 2px.
    pub fn radius_md(&self) -> Rems {
        rems((self.radius.0 - RADIUS_STEP / 2.).max(0.))
    }

    /// `rounded-lg`: the radius itself.
    pub fn radius_lg(&self) -> Rems {
        self.radius
    }

    /// `rounded-xl`: radius + 4px.
    pub fn radius_xl(&self) -> Rems {
        rems(self.radius.0 + RADIUS_STEP)
    }
}

/// The active token set, a GPUI global. The app sets it before any component
/// renders (reading it unset panics, as with any GPUI global).
#[derive(Clone, Copy, Debug)]
pub struct UiTheme {
    pub tokens: Tokens,
}

impl Global for UiTheme {}

impl UiTheme {
    /// Makes `tokens` the active set and redraws every window, which is all a
    /// switch takes: components read the set on each render.
    pub fn set(tokens: Tokens, cx: &mut App) {
        cx.set_global(UiTheme { tokens });
        cx.refresh_windows();
    }
}

/// Access to the active token set: `cx.ui_theme().primary`.
pub trait ActiveTheme {
    fn ui_theme(&self) -> &Tokens;
}

impl ActiveTheme for App {
    fn ui_theme(&self) -> &Tokens {
        &self.global::<UiTheme>().tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sRGB bytes of a colour, ignoring alpha.
    fn bytes(color: Hsla) -> [u8; 3] {
        let c = color.to_rgb();
        [c.r, c.g, c.b].map(|v| (v * 255.).round() as u8)
    }

    /// oklch → sRGB bytes (Björn Ottosson's OKLab matrices), clamped into the
    /// gamut the way the hex constants were.
    fn oklch(l: f64, c: f64, h: f64) -> [u8; 3] {
        let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
        let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
        let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
        let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
        let (l, m, s) = (l_.powi(3), m_.powi(3), s_.powi(3));
        let linear = [
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
        ];
        linear.map(|x| {
            let x = x.clamp(0., 1.);
            let v = if x <= 0.0031308 {
                12.92 * x
            } else {
                1.055 * x.powf(1. / 2.4) - 0.055
            };
            (v * 255.).round() as u8
        })
    }

    #[test]
    fn neutral_sets_match_shadcn() {
        let light = Tokens::neutral_light();
        let dark = Tokens::neutral_dark();
        // (name, token, the hex written in the set, shadcn's oklch value)
        let cases = [
            ("light background", light.background, [0xff, 0xff, 0xff], (1.0, 0., 0.)),
            ("light primary", light.primary, [0x17, 0x17, 0x17], (0.205, 0., 0.)),
            ("light border", light.border, [0xe5, 0xe5, 0xe5], (0.922, 0., 0.)),
            ("light ring", light.ring, [0xa1, 0xa1, 0xa1], (0.708, 0., 0.)),
            ("dark background", dark.background, [0x0a, 0x0a, 0x0a], (0.145, 0., 0.)),
            ("dark primary", dark.primary, [0xe5, 0xe5, 0xe5], (0.922, 0., 0.)),
            ("dark border", dark.border, [0xff, 0xff, 0xff], (1.0, 0., 0.)),
            ("dark ring", dark.ring, [0x73, 0x73, 0x73], (0.556, 0., 0.)),
        ];
        for (name, token, expected, (l, c, h)) in cases {
            assert_eq!(bytes(token), expected, "{name}");
            assert_eq!(oklch(l, c, h), expected, "{name} vs shadcn");
        }
        assert_eq!(light.radius, rems(0.625));
        assert_eq!(dark.radius, rems(0.625));
    }

    #[test]
    fn destructive_is_shadcn_red_clamped() {
        let light = Tokens::neutral_light();
        let dark = Tokens::neutral_dark();
        assert_eq!(bytes(light.destructive), oklch(0.577, 0.245, 27.325));
        assert_eq!(bytes(dark.destructive), oklch(0.704, 0.191, 22.216));
    }

    #[test]
    fn dark_border_and_input_are_translucent_white() {
        let dark = Tokens::neutral_dark();
        assert_eq!(dark.border, hsla(0., 0., 1., 0.10));
        assert_eq!(dark.border.a, 0.10);
        assert_eq!(dark.input, hsla(0., 0., 1., 0.15));
        // Opaque in the light set.
        assert_eq!(Tokens::neutral_light().border.a, 1.0);
    }

    /// The `dark` hint follows the background: the dark set's is near black,
    /// the light set's white.
    #[test]
    fn the_dark_hint_matches_the_background() {
        let light = Tokens::neutral_light();
        let dark = Tokens::neutral_dark();
        assert!(!light.dark);
        assert!(dark.dark);
        assert!(light.background.l > 0.5 && dark.background.l < 0.5);
    }

    #[test]
    fn radius_steps() {
        let t = Tokens::neutral_light();
        assert_eq!(t.radius_sm(), rems(0.375));
        assert_eq!(t.radius_md(), rems(0.5));
        assert_eq!(t.radius_lg(), rems(0.625));
        assert_eq!(t.radius_xl(), rems(0.875));
        let square = Tokens { radius: rems(0.), ..t };
        assert_eq!(square.radius_sm(), rems(0.));
    }
}
