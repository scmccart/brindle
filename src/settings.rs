//! App-wide settings shared by every window, stored as a GPUI global.

use gpui::{App, Font, FontFallbacks, FontFeatures, Global, Pixels, px};

use crate::config::Config;

pub struct Settings {
    pub config: Config,
    /// Error from the last config load, shown in a banner until fixed.
    pub config_error: Option<String>,
    /// Font size change from ctrl-+ / ctrl-- (applies to all tabs).
    pub zoom: f32,
    /// The first installed family from `config.font.family`.
    pub font_family: String,
    /// Proportional font for tabs and other chrome.
    pub ui_font_family: String,
}

impl Global for Settings {}

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
        let ui_font_family = resolve_ui_family(cx);
        Self { config, config_error, zoom: 0.0, font_family, ui_font_family }
    }

    pub fn get(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn font(&self) -> Font {
        let fallbacks = FALLBACK_FAMILIES
            .iter()
            .filter(|f| **f != self.font_family)
            .map(|f| f.to_string())
            .collect();
        Font {
            family: self.font_family.clone().into(),
            // Ligatures would break the one-glyph-per-cell grid.
            features: FontFeatures::disable_ligatures(),
            fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
            weight: Default::default(),
            style: Default::default(),
        }
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
    for fallback in ["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "Ubuntu Mono"] {
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
    ["Inter", "Cantarell", "Ubuntu Sans", "Ubuntu", "Noto Sans", "DejaVu Sans", "Liberation Sans"]
        .iter()
        .find(|want| installed.iter().any(|have| have == *want))
        .map(|s| s.to_string())
        .unwrap_or_else(|| ".SystemUIFont".into())
}
