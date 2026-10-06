//! Color themes. Every field is optional in config files; omitted colors fall
//! back to the `brindle-dark` palette.

use std::fmt;

use gpui::{Hsla, Rgba};
use serde::{Deserialize, Deserializer};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn hex(v: u32) -> Self {
        Self { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().strip_prefix('#').unwrap_or(s.trim());
        match s.len() {
            6 => u32::from_str_radix(s, 16).ok().map(Self::hex),
            3 => {
                let v = u32::from_str_radix(s, 16).ok()?;
                let expand = |n: u32| ((n & 0xf) * 0x11) as u8;
                Some(Self { r: expand(v >> 8), g: expand(v >> 4), b: expand(v) })
            }
            _ => None,
        }
    }

    pub fn hsla(self) -> Hsla {
        self.rgba(1.0).into()
    }

    pub fn rgba(self, alpha: f32) -> Rgba {
        Rgba {
            r: self.r as f32 / 255.0,
            g: self.g as f32 / 255.0,
            b: self.b as f32 / 255.0,
            a: alpha,
        }
    }

    /// Linear blend towards `other` by `t` in `[0, 1]`.
    pub fn mix(self, other: Color, t: f32) -> Color {
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color { r: lerp(self.r, other.r), g: lerp(self.g, other.g), b: lerp(self.b, other.b) }
    }

    pub fn luminance(self) -> f32 {
        (0.2126 * self.r as f32 + 0.7152 * self.g as f32 + 0.0722 * self.b as f32) / 255.0
    }
}

impl From<Color> for alacritty_terminal::vte::ansi::Rgb {
    fn from(c: Color) -> Self {
        Self { r: c.r, g: c.g, b: c.b }
    }
}

impl From<alacritty_terminal::vte::ansi::Rgb> for Color {
    fn from(c: alacritty_terminal::vte::ansi::Rgb) -> Self {
        Self { r: c.r, g: c.g, b: c.b }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color {s:?}")))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    pub cursor_text: Option<Color>,
    pub selection_background: Color,
    pub selection_foreground: Option<Color>,
    /// The 16 ANSI colors: black, red, green, yellow, blue, magenta, cyan,
    /// white, then the bright variants in the same order.
    pub ansi: [Color; 16],
    /// Accent used for the active tab marker and focus rings.
    pub accent: Option<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        builtin_theme("brindle-dark").unwrap()
    }
}

impl Theme {
    pub fn accent(&self) -> Color {
        self.accent.unwrap_or(self.ansi[4])
    }

    /// The color for an xterm palette index: the theme's 16 ANSI colors,
    /// then the standard 6x6x6 cube and gray ramp.
    pub fn indexed(&self, index: u8) -> Color {
        let i = index as usize;
        match i {
            0..=15 => self.ansi[i],
            16..=231 => {
                let i = i - 16;
                let step = |v: usize| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
                Color { r: step(i / 36), g: step((i / 6) % 6), b: step(i % 6) }
            }
            _ => {
                let v = ((i - 232) * 10 + 8) as u8;
                Color { r: v, g: v, b: v }
            }
        }
    }

    /// This theme with `cursor_text` and `accent` filled in with the colors
    /// they fall back to, so they survive being written to a config (where
    /// an omitted key means brindle-dark's color instead).
    pub fn with_effective_colors(&self) -> Theme {
        Theme { cursor_text: Some(self.cursor_text.unwrap_or(self.background)), accent: Some(self.accent()), ..self.clone() }
    }

    pub fn is_dark(&self) -> bool {
        self.background.luminance() < 0.5
    }

    /// Background of the tab strip, slightly offset from the terminal background.
    pub fn chrome_background(&self) -> Color {
        let target = if self.is_dark() { Color::hex(0x000000) } else { Color::hex(0xffffff) };
        self.background.mix(target, 0.35)
    }

    pub fn chrome_border(&self) -> Color {
        self.background.mix(self.foreground, 0.12)
    }

    pub fn muted_foreground(&self) -> Color {
        self.foreground.mix(self.background, 0.45)
    }

    pub fn hover_background(&self) -> Color {
        self.chrome_background().mix(self.foreground, 0.08)
    }
}

const fn palette(colors: [u32; 16]) -> [Color; 16] {
    let mut out = [Color { r: 0, g: 0, b: 0 }; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Color::hex(colors[i]);
        i += 1;
    }
    out
}

pub const BUILTIN_THEMES: &[&str] =
    &["brindle-dark", "brindle-light", "tokyo-night", "gruvbox-dark", "solarized-dark"];

pub fn builtin_theme(name: &str) -> Option<Theme> {
    let theme = match name {
        "brindle-dark" => Theme {
            foreground: Color::hex(0xd8d4cc),
            background: Color::hex(0x1b1a18),
            cursor: Color::hex(0xe6a756),
            cursor_text: Some(Color::hex(0x1b1a18)),
            selection_background: Color::hex(0x4a4238),
            selection_foreground: None,
            ansi: palette([
                0x2a2826, 0xe0675f, 0x9fbf6a, 0xe6b450, 0x6fa3d6, 0xc38ad0, 0x6cc3b8, 0xc9c4ba,
                0x5c5853, 0xf2847c, 0xb7d685, 0xf2c96e, 0x8cbbe8, 0xd6a5e0, 0x8ad6cc, 0xf2eee6,
            ]),
            accent: Some(Color::hex(0xe6a756)),
        },
        "brindle-light" => Theme {
            foreground: Color::hex(0x2e2b27),
            background: Color::hex(0xf7f3ec),
            cursor: Color::hex(0xb56a12),
            cursor_text: Some(Color::hex(0xf7f3ec)),
            selection_background: Color::hex(0xe3d6c2),
            selection_foreground: None,
            ansi: palette([
                0x2e2b27, 0xc0392b, 0x4f7f1f, 0x9a6b00, 0x2a62a8, 0x8e44ad, 0x16817a, 0xd8d2c8,
                0x6b665f, 0xd9534a, 0x5f9a2a, 0xb8860b, 0x3d7fd1, 0xa55dc2, 0x1fa39a, 0xffffff,
            ]),
            accent: Some(Color::hex(0xb56a12)),
        },
        "tokyo-night" => Theme {
            foreground: Color::hex(0xc0caf5),
            background: Color::hex(0x1a1b26),
            cursor: Color::hex(0xc0caf5),
            cursor_text: Some(Color::hex(0x1a1b26)),
            selection_background: Color::hex(0x33467c),
            selection_foreground: None,
            ansi: palette([
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ]),
            accent: Some(Color::hex(0x7aa2f7)),
        },
        "gruvbox-dark" => Theme {
            foreground: Color::hex(0xebdbb2),
            background: Color::hex(0x282828),
            cursor: Color::hex(0xebdbb2),
            cursor_text: Some(Color::hex(0x282828)),
            selection_background: Color::hex(0x504945),
            selection_foreground: None,
            ansi: palette([
                0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
            ]),
            accent: Some(Color::hex(0xfabd2f)),
        },
        "solarized-dark" => Theme {
            foreground: Color::hex(0x839496),
            background: Color::hex(0x002b36),
            cursor: Color::hex(0x93a1a1),
            cursor_text: Some(Color::hex(0x002b36)),
            selection_background: Color::hex(0x073642),
            selection_foreground: Some(Color::hex(0x93a1a1)),
            ansi: palette([
                0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
            ]),
            accent: Some(Color::hex(0x268bd2)),
        },
        _ => return None,
    };
    Some(theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_colors() {
        assert_eq!(Color::parse("#ff8000"), Some(Color { r: 255, g: 128, b: 0 }));
        assert_eq!(Color::parse("0f0"), Some(Color { r: 0, g: 255, b: 0 }));
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("zzzzzz"), None);
    }

    #[test]
    fn all_builtins_exist() {
        for name in BUILTIN_THEMES {
            assert!(builtin_theme(name).is_some(), "{name}");
        }
        assert!(builtin_theme("brindle-dark").unwrap().is_dark());
        assert!(!builtin_theme("brindle-light").unwrap().is_dark());
    }

    #[test]
    fn partial_theme_inherits_defaults() {
        let theme: Theme = toml::from_str("background = \"#000000\"").unwrap();
        assert_eq!(theme.background, Color::hex(0));
        assert_eq!(theme.foreground, builtin_theme("brindle-dark").unwrap().foreground);
    }
}
