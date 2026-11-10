//! App-wide settings shared by every window, stored as a GPUI global.

use gpui::{App, Font, FontFallbacks, FontFeatures, Global, Pixels, px};

use crate::config::Config;
use crate::theme::Theme;

pub struct Settings {
    pub config: Config,
    /// Error from the last config load, shown in a banner until fixed.
    pub config_error: Option<String>,
    /// Font size change from ctrl-+ / ctrl-- (applies to all tabs).
    pub zoom: f32,
    font: Font,
    /// Proportional font for tabs and other chrome.
    pub ui_font_family: gpui::SharedString,
}

impl Global for Settings {}

/// See [`Settings::chrome_theme`]; set with the tokens.
struct ChromeTheme(Theme);

impl Global for ChromeTheme {}

/// Glyph sources tried when the primary font lacks a character.
const FALLBACK_FAMILIES: &[&str] = &[
    "Symbols Nerd Font Mono",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Noto Color Emoji",
    "Noto Sans CJK SC",
];

impl Settings {
    pub fn new(config: Config, config_error: Option<String>, cx: &App) -> Self {
        let font_family = resolve_family(&config.font.family, cx);
        log::info!("using font family {font_family:?}");
        // Only installed ones: GPUI logs an error for each missing family.
        let installed = cx.text_system().all_font_names();
        let fallbacks = FALLBACK_FAMILIES
            .iter()
            .filter(|f| **f != font_family && installed.iter().any(|have| have == **f))
            .map(|f| f.to_string())
            .collect();
        let font = Font {
            family: font_family.into(),
            // Ligatures would break the one-glyph-per-cell grid.
            features: FontFeatures::disable_ligatures(),
            fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
            weight: Default::default(),
            style: Default::default(),
        };
        let ui_font_family = resolve_ui_family(cx).into();
        Self { config, config_error, zoom: 0.0, font, ui_font_family }
    }

    /// Makes the config's global theme the active `brindle-ui` token set.
    pub fn apply_ui_theme(cx: &mut App) {
        let theme = Self::get(cx).config.theme(None);
        Self::set_ui_theme(&theme, cx);
    }

    /// Makes `theme` the active token set (a preview, or the config's theme),
    /// and the theme chrome takes its status colours from
    /// ([`Settings::chrome_theme`]).
    pub fn set_ui_theme(theme: &Theme, cx: &mut App) {
        brindle_ui::UiTheme::set(theme.ui_tokens(), cx);
        cx.set_global(ChromeTheme(theme.clone()));
    }

    /// The theme the window chrome's tokens come from: the config's global
    /// theme, or the one the settings dialog previews. The strip and the
    /// zone headers draw program status dots in its colours
    /// ([`Theme::status_color`]), so a tab's own theme never tints them.
    pub fn chrome_theme(cx: &App) -> &Theme {
        &cx.global::<ChromeTheme>().0
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// The terminal font (built once; cloning is cheap).
    pub fn font(&self) -> Font {
        self.font.clone()
    }

    pub fn font_size(&self, profile_size: Option<f32>) -> Pixels {
        px((profile_size.unwrap_or(self.config.font.size) + self.zoom).clamp(5.0, 96.0))
    }
}

fn resolve_family(preferred: &[String], cx: &App) -> String {
    let installed = cx.text_system().all_font_names();
    for want in preferred {
        if let Some(found) = installed.iter().find(|have| have.eq_ignore_ascii_case(want)) {
            return found.clone();
        }
    }
    // Linux families, then Windows' (Cascadia Mono ships with Windows 11).
    for fallback in ["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "Ubuntu Mono", "Cascadia Mono", "Consolas"] {
        if installed.iter().any(|have| have == fallback) {
            return fallback.into();
        }
    }
    installed
        .into_iter()
        .find(|name| name.to_lowercase().contains("mono"))
        .unwrap_or_else(|| "monospace".into())
}

fn resolve_ui_family(cx: &App) -> String {
    let installed = cx.text_system().all_font_names();
    [
        "Inter",
        "Cantarell",
        "Ubuntu Sans",
        "Ubuntu",
        "Noto Sans",
        "DejaVu Sans",
        "Liberation Sans",
        "Segoe UI Variable Text",
        "Segoe UI",
    ]
        .iter()
        .find(|want| installed.iter().any(|have| have == *want))
        .map(|s| s.to_string())
        .unwrap_or_else(|| ".SystemUIFont".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin_theme;
    use brindle_ui::ActiveTheme;
    use std::{sync::mpsc, thread, time::Duration};

    /// Runs `check` inside a headless GPUI app, which needs no display. The
    /// headless event loop doesn't return on `quit`, so the app lives on its
    /// own thread, leaked once the verdict is in.
    fn in_headless_app(check: impl FnOnce(&mut App) + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            gpui::Application::headless().run(move |cx: &mut App| {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(cx)));
                tx.send(outcome.map_err(|e| {
                    e.downcast_ref::<String>()
                        .cloned()
                        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default()
                }))
                .ok();
            });
        });
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => panic!("{message}"),
            Err(_) => panic!("the headless app never ran the check"),
        }
    }

    /// A preview re-themes the components, and cancelling it (applying the
    /// config's theme again) brings back the tokens from before.
    #[test]
    fn cancelling_a_preview_restores_the_tokens() {
        in_headless_app(|cx| {
            let config = Config::default();
            let config_theme = config.theme(None);
            cx.set_global(Settings::new(config, None, cx));

            Settings::apply_ui_theme(cx);
            let before = *cx.ui_theme();
            assert_eq!(before, config_theme.ui_tokens());

            let preview = builtin_theme("brindle-light").unwrap();
            assert_ne!(preview, config_theme);
            assert_eq!(*Settings::chrome_theme(cx), config_theme);
            Settings::set_ui_theme(&preview, cx);
            assert_eq!(*cx.ui_theme(), preview.ui_tokens());
            assert_ne!(*cx.ui_theme(), before);
            // Chrome's status colours follow the preview too.
            assert_eq!(*Settings::chrome_theme(cx), preview);

            Settings::apply_ui_theme(cx);
            assert_eq!(*cx.ui_theme(), before);
            assert_eq!(*Settings::chrome_theme(cx), config_theme);
        });
    }
}
