//! tmux styles: the `#[fg=blue,bold]` runs in expanded formats such as
//! `pane-border-format`, and style options such as `window-style`.

use crate::theme::Color;

/// A tmux color other than `default`, which is `None` wherever a color is
/// optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxColor {
    /// An xterm palette index: names are 0-15, `colourN` is any index.
    Indexed(u8),
    Rgb(Color),
}

impl TmuxColor {
    /// Parses `blue`, `brightred`, `colour208`/`color208` or `#rrggbb`.
    /// `default`, `terminal` and anything unknown give `None`.
    pub fn parse(s: &str) -> Option<TmuxColor> {
        const NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
        let s = s.trim().to_ascii_lowercase();
        if let Some(hex) = s.strip_prefix('#') {
            return (hex.len() == 6).then(|| Color::parse(hex)).flatten().map(TmuxColor::Rgb);
        }
        if let Some(n) = s.strip_prefix("colour").or_else(|| s.strip_prefix("color")) {
            return n.parse().ok().map(TmuxColor::Indexed);
        }
        let (bright, name) = s.strip_prefix("bright").map_or((0, s.as_str()), |n| (8, n));
        NAMES.iter().position(|&n| n == name).map(|i| TmuxColor::Indexed(i as u8 + bright))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    /// `None` is the base color of whatever the style is drawn on.
    pub fg: Option<TmuxColor>,
    pub bg: Option<TmuxColor>,
    pub bold: bool,
    pub reverse: bool,
}

impl Style {
    /// Applies a list of style directives, separated by commas or spaces,
    /// as in `#[fg=blue,bold]` or a style option's value. Directives other
    /// than colors, bold, reverse, `default` and `none` are ignored.
    pub fn apply(&mut self, directives: &str) {
        for directive in directives.split([',', ' ']).filter(|d| !d.is_empty()) {
            match directive {
                "default" => *self = Style::default(),
                "none" => (self.bold, self.reverse) = (false, false),
                "bold" | "bright" => self.bold = true,
                "nobold" => self.bold = false,
                "reverse" => self.reverse = true,
                "noreverse" => self.reverse = false,
                d => {
                    if let Some(color) = d.strip_prefix("fg=") {
                        self.fg = TmuxColor::parse(color);
                    } else if let Some(color) = d.strip_prefix("bg=") {
                        self.bg = TmuxColor::parse(color);
                    }
                }
            }
        }
    }
}

impl Style {
    /// The style a style option's value (`fg=blue,bg=default`) describes.
    pub fn parse(value: &str) -> Style {
        let mut style = Style::default();
        style.apply(value);
        style
    }
}

/// A pane's default foreground and background under tmux's pane styles:
/// the active pane takes `window-active-style`'s colors where it sets them,
/// and `window-style`'s otherwise. `None` is the theme's color.
pub fn pane_defaults(
    window_style: &Style,
    active_style: &Style,
    active: bool,
) -> (Option<TmuxColor>, Option<TmuxColor>) {
    let pick = |own: Option<TmuxColor>, active_own: Option<TmuxColor>| active_own.filter(|_| active).or(own);
    (pick(window_style.fg, active_style.fg), pick(window_style.bg, active_style.bg))
}

/// tmux's built-in values for the border style options. Brindle draws its
/// own theme colors for them, rather than tmux's green active border.
pub const DEFAULT_BORDER_STYLE: &str = "default";
pub const DEFAULT_ACTIVE_BORDER_STYLE: &str = "#{?pane_in_mode,fg=yellow,#{?synchronize-panes,fg=red,fg=green}}";

/// The color a border style option sets, from its raw value (to recognise
/// tmux's built-in default) and its expanded value.
pub fn border_color(raw: &str, expanded: &str, builtin: &str) -> Option<TmuxColor> {
    if raw.trim() == builtin {
        return None;
    }
    Style::parse(expanded).fg
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledRun {
    pub text: String,
    pub style: Style,
}

/// Splits an expanded tmux format into runs of text with their styles.
/// `##` is a literal `#`; an unterminated `#[` is kept as text.
pub fn parse_format(format: &str) -> Vec<StyledRun> {
    let mut runs: Vec<StyledRun> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut rest = format;
    let flush = |runs: &mut Vec<StyledRun>, text: &mut String, style: Style| {
        if text.is_empty() {
            return;
        }
        match runs.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => runs.push(StyledRun { text: text.clone(), style }),
        }
        text.clear();
    };
    while let Some(at) = rest.find('#') {
        text.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(after) = after.strip_prefix('#') {
            text.push('#');
            rest = after;
        } else if let Some(directives) = after.strip_prefix('[')
            && let Some(end) = directives.find(']')
        {
            flush(&mut runs, &mut text, style);
            style.apply(&directives[..end]);
            rest = &directives[end + 1..];
        } else {
            text.push('#');
            rest = after;
        }
    }
    text.push_str(rest);
    flush(&mut runs, &mut text, style);
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, style: Style) -> StyledRun {
        StyledRun { text: text.into(), style }
    }

    #[test]
    fn claude_code_teammate_title() {
        let blue_bold = Style { fg: Some(TmuxColor::Indexed(4)), bold: true, ..Default::default() };
        assert_eq!(
            parse_format("#[fg=blue,bold] @researcher #[default]"),
            vec![run(" @researcher ", blue_bold)]
        );
    }

    #[test]
    fn default_border_format() {
        let reverse = Style { reverse: true, ..Default::default() };
        assert_eq!(
            parse_format("#[reverse]0#[default] \"bash\""),
            vec![run("0", reverse), run(" \"bash\"", Style::default())]
        );
        assert_eq!(parse_format("1#[default] \"bash\""), vec![run("1 \"bash\"", Style::default())]);
        assert_eq!(parse_format(""), vec![]);
    }

    #[test]
    fn literal_hashes_and_unknown_directives() {
        assert_eq!(parse_format("a##b #c"), vec![run("a#b #c", Style::default())]);
        assert_eq!(
            parse_format("#[align=right,range=pane|%1]x#[norange]"),
            vec![run("x", Style::default())]
        );
        assert_eq!(parse_format("a #[fg=red"), vec![run("a #[fg=red", Style::default())]);
    }

    #[test]
    fn colors() {
        assert_eq!(TmuxColor::parse("brightred"), Some(TmuxColor::Indexed(9)));
        assert_eq!(TmuxColor::parse("colour208"), Some(TmuxColor::Indexed(208)));
        assert_eq!(TmuxColor::parse("color0"), Some(TmuxColor::Indexed(0)));
        assert_eq!(TmuxColor::parse("#1a1b26"), Some(TmuxColor::Rgb(Color::hex(0x1a1b26))));
        assert_eq!(TmuxColor::parse("default"), None);
        assert_eq!(TmuxColor::parse("colour300"), None);
        assert_eq!(TmuxColor::parse("#abc"), None);
    }

    #[test]
    fn pane_default_colors() {
        let blue = Some(TmuxColor::Indexed(4));
        let dark = Some(TmuxColor::Rgb(Color::hex(0x202020)));
        let window = Style::parse("fg=blue");
        let active = Style::parse("bg=#202020");
        assert_eq!(pane_defaults(&window, &active, false), (blue, None));
        assert_eq!(pane_defaults(&window, &active, true), (blue, dark));
        assert_eq!(pane_defaults(&window, &Style::parse("fg=red"), true), (Some(TmuxColor::Indexed(1)), None));
        let none = Style::parse("default");
        assert_eq!(pane_defaults(&none, &Style::parse(""), true), (None, None));
    }

    #[test]
    fn border_colors_ignore_tmux_defaults() {
        assert_eq!(border_color("default", "default", DEFAULT_BORDER_STYLE), None);
        assert_eq!(border_color(DEFAULT_ACTIVE_BORDER_STYLE, "fg=green", DEFAULT_ACTIVE_BORDER_STYLE), None);
        assert_eq!(border_color("fg=magenta", "fg=magenta", DEFAULT_BORDER_STYLE), Some(TmuxColor::Indexed(5)));
        assert_eq!(border_color("fg=green", "fg=green", DEFAULT_ACTIVE_BORDER_STYLE), Some(TmuxColor::Indexed(2)));
        assert_eq!(border_color("bold", "bold", DEFAULT_BORDER_STYLE), None);
    }

    #[test]
    fn style_directives() {
        let parse = |value| {
            let mut style = Style { bold: true, ..Default::default() };
            style.apply(value);
            style
        };
        assert_eq!(parse("fg=blue,nobold"), Style { fg: Some(TmuxColor::Indexed(4)), ..Default::default() });
        assert_eq!(
            parse("bg=#102030 fg=default none"),
            Style { bg: Some(TmuxColor::Rgb(Color::hex(0x102030))), ..Default::default() }
        );
        assert_eq!(parse("default"), Style::default());
    }
}
