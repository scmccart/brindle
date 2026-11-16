//! App-wide settings shared by every window, stored as a GPUI global.

use gpui::{App, Font, FontFallbacks, FontFeatures, Global, Pixels, SharedString, px};

use crate::config::{Config, FontConfig};
use crate::theme::Theme;

pub struct Settings {
    pub config: Config,
    /// Error from the last config load, shown in a banner until fixed.
    pub config_error: Option<String>,
    /// Font size change from ctrl-+ / ctrl-- (applies to all tabs).
    pub zoom: f32,
    font: Font,
    /// Proportional font for tabs and other chrome.
    ui_font_family: SharedString,
    /// The installed families when the config was loaded, which a preview
    /// resolves against.
    installed: Vec<String>,
    /// The settings dialog's unapplied font changes, read before the config.
    preview: Option<Preview>,
}

impl Global for Settings {}

/// A font family the settings dialog chooses: Brindle's built-in list (no
/// `family` key in the config), or one installed family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyChoice {
    Default,
    Family(String),
}

/// The Fonts section's unapplied changes. Each field is `None` when it
/// isn't changed, so the config's value (resolved as on a load) applies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontPreview {
    pub terminal_family: Option<FamilyChoice>,
    pub size: Option<f32>,
    pub line_height: Option<f32>,
    pub ui_family: Option<FamilyChoice>,
}

/// A [`FontPreview`] with its families resolved.
struct Preview {
    fonts: FontPreview,
    font: Option<Font>,
    ui_font_family: Option<SharedString>,
}

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

/// The chrome's families when `[ui_font]` names none that is installed:
/// Linux desktops' UI fonts, then Windows'.
pub const BUILTIN_UI_FAMILIES: &[&str] = &[
    "Inter",
    "Cantarell",
    "Ubuntu Sans",
    "Ubuntu",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Segoe UI Variable Text",
    "Segoe UI",
];

/// GPUI's name for the platform's UI font, the chrome's when nothing else
/// is installed.
pub const SYSTEM_UI_FONT: &str = ".SystemUIFont";

impl Settings {
    pub fn new(config: Config, config_error: Option<String>, cx: &App) -> Self {
        let installed = cx.text_system().all_font_names();
        let font_family = resolve_family(&config.font.family, &installed);
        log::info!("using font family {font_family:?}");
        let font = terminal_font(font_family, &installed);
        let ui_font_family = resolve_ui_family(&config.ui_font.family, &installed);
        log::info!("using UI font family {ui_font_family:?}");
        Self {
            config,
            config_error,
            zoom: 0.0,
            font,
            ui_font_family: ui_font_family.into(),
            installed,
            preview: None,
        }
    }

    /// Makes the config's global theme the active `brindle-ui` token set,
    /// and the chrome's family (preview included) its font.
    pub fn apply_ui_theme(cx: &mut App) {
        let theme = Self::get(cx).config.theme(None);
        Self::set_ui_theme(&theme, cx);
        Self::apply_ui_font(cx);
    }

    /// Makes `theme` the active token set (a preview, or the config's theme),
    /// and the theme chrome takes its status colours from
    /// ([`Settings::chrome_theme`]).
    pub fn set_ui_theme(theme: &Theme, cx: &mut App) {
        brindle_ui::UiTheme::set(theme.ui_tokens(), cx);
        cx.set_global(ChromeTheme(theme.clone()));
    }

    /// Makes [`Settings::ui_font_family`] (preview included) the family the
    /// chrome draws in, `brindle-ui`'s own roots included.
    pub fn apply_ui_font(cx: &mut App) {
        let family = Self::get(cx).ui_font_family();
        if cx.try_global::<brindle_ui::UiFont>().map(|font| &font.family) != Some(&family) {
            brindle_ui::UiFont::set(family, cx);
        }
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

    /// The terminal font (built once; cloning is cheap), or the previewed one.
    pub fn font(&self) -> Font {
        match self.preview.as_ref().and_then(|p| p.font.as_ref()) {
            Some(font) => font.clone(),
            None => self.font.clone(),
        }
    }

    /// A terminal's font size: its profile's, else the previewed or the
    /// configured size, plus the zoom.
    pub fn font_size(&self, profile_size: Option<f32>) -> Pixels {
        let size = self.preview.as_ref().and_then(|p| p.fonts.size).unwrap_or(self.config.font.size);
        px((profile_size.unwrap_or(size) + self.zoom).clamp(5.0, 96.0))
    }

    /// The terminal's line height, as a multiple of its font size: the
    /// previewed one, else the configured one.
    pub fn line_height(&self) -> f32 {
        self.preview.as_ref().and_then(|p| p.fonts.line_height).unwrap_or(self.config.font.line_height)
    }

    /// The chrome's family: the previewed one, else the configured one.
    pub fn ui_font_family(&self) -> SharedString {
        match self.preview.as_ref().and_then(|p| p.ui_font_family.as_ref()) {
            Some(family) => family.clone(),
            None => self.ui_font_family.clone(),
        }
    }

    pub fn font_preview(&self) -> Option<&FontPreview> {
        self.preview.as_ref().map(|p| &p.fonts)
    }

    /// Shows `preview`'s fonts in place of the config's, or, with `None` (or
    /// a preview that changes nothing), the config's again. The caller
    /// re-fits the grids (`cx.refresh_windows()`) and applies the UI font
    /// ([`Settings::apply_ui_font`]).
    pub fn set_font_preview(&mut self, preview: Option<FontPreview>) {
        let installed = &self.installed;
        self.preview = preview.filter(|p| *p != FontPreview::default()).map(|fonts| {
            let font = fonts.terminal_family.as_ref().map(|choice| {
                let family = match choice {
                    FamilyChoice::Default => resolve_family(&FontConfig::default().family, installed),
                    FamilyChoice::Family(name) => resolve_family(std::slice::from_ref(name), installed),
                };
                terminal_font(family, installed)
            });
            let ui_font_family = fonts.ui_family.as_ref().map(|choice| {
                let preferred = match choice {
                    FamilyChoice::Default => Vec::new(),
                    FamilyChoice::Family(name) => vec![name.clone()],
                };
                resolve_ui_family(&preferred, installed).into()
            });
            Preview { fonts, font, ui_font_family }
        });
    }
}

/// The terminal font in `family`: no ligatures, and the installed glyph
/// fallbacks.
fn terminal_font(family: String, installed: &[String]) -> Font {
    // Only installed ones: GPUI logs an error for each missing family.
    let fallbacks = FALLBACK_FAMILIES
        .iter()
        .filter(|f| **f != family && installed.iter().any(|have| have == **f))
        .map(|f| f.to_string())
        .collect();
    Font {
        family: family.into(),
        // Ligatures would break the one-glyph-per-cell grid.
        features: FontFeatures::disable_ligatures(),
        fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
        weight: Default::default(),
        style: Default::default(),
    }
}

/// The first of `preferred` that is installed, matched case-insensitively,
/// in its installed spelling.
pub fn first_installed(preferred: &[String], installed: &[String]) -> Option<String> {
    preferred
        .iter()
        .find_map(|want| installed.iter().find(|have| have.eq_ignore_ascii_case(want)))
        .cloned()
}

/// The terminal font's family: the first installed of `preferred`
/// (`[font]`'s list), else a common monospace family, else any installed
/// family named "mono".
fn resolve_family(preferred: &[String], installed: &[String]) -> String {
    if let Some(found) = first_installed(preferred, installed) {
        return found;
    }
    // Linux families, then Windows' (Cascadia Mono ships with Windows 11).
    for fallback in ["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "Ubuntu Mono", "Cascadia Mono", "Consolas"] {
        if installed.iter().any(|have| have == fallback) {
            return fallback.into();
        }
    }
    installed
        .iter()
        .find(|name| name.to_lowercase().contains("mono"))
        .cloned()
        .unwrap_or_else(|| "monospace".into())
}

/// The chrome's family: the first installed of `preferred` (`[ui_font]`'s
/// list), else of [`BUILTIN_UI_FAMILIES`], else the platform's UI font.
pub fn resolve_ui_family(preferred: &[String], installed: &[String]) -> String {
    if let Some(found) = first_installed(preferred, installed) {
        return found;
    }
    BUILTIN_UI_FAMILIES
        .iter()
        .find(|want| installed.iter().any(|have| have == *want))
        .map(|s| s.to_string())
        .unwrap_or_else(|| SYSTEM_UI_FONT.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin_theme;
    use brindle_ui::ActiveTheme;
    use std::{sync::mpsc, thread, time::Duration};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_ui_family_is_the_first_installed_of_the_config_list() {
        let installed = names(&["Arial", "Inter", "Segoe UI", "Consolas"]);
        // Matched case-insensitively, in the installed spelling.
        assert_eq!(resolve_ui_family(&names(&["Missing", "arial"]), &installed), "Arial");
        assert_eq!(resolve_ui_family(&names(&["Segoe UI", "Arial"]), &installed), "Segoe UI");
    }

    #[test]
    fn a_ui_list_with_nothing_installed_falls_back_to_the_builtin_list() {
        let installed = names(&["Arial", "Segoe UI"]);
        assert_eq!(resolve_ui_family(&names(&["Nonexistent"]), &installed), "Segoe UI");
        assert_eq!(resolve_ui_family(&names(&["Nonexistent"]), &names(&["Inter", "Segoe UI"])), "Inter");
    }

    #[test]
    fn an_empty_ui_list_takes_the_builtin_list_then_the_system_font() {
        assert_eq!(resolve_ui_family(&[], &names(&["Cantarell", "Segoe UI"])), "Cantarell");
        assert_eq!(resolve_ui_family(&[], &names(&["Arial"])), SYSTEM_UI_FONT);
    }

    #[test]
    fn the_terminal_family_falls_back_as_before() {
        let installed = names(&["Cascadia Mono", "Consolas", "Arial"]);
        assert_eq!(resolve_family(&names(&["consolas"]), &installed), "Consolas");
        assert_eq!(resolve_family(&names(&["Nonexistent"]), &installed), "Cascadia Mono");
        assert_eq!(resolve_family(&[], &names(&["Iosevka Mono"])), "Iosevka Mono");
        assert_eq!(resolve_family(&[], &names(&["Arial"])), "monospace");
        assert_eq!(first_installed(&names(&["Nonexistent", "cascadia mono"]), &installed).as_deref(), Some("Cascadia Mono"));
        assert_eq!(first_installed(&names(&["Nonexistent"]), &installed), None);
    }

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

    /// A font preview is read before the config, a profile's size still
    /// wins over its size, and clearing it brings back the config's fonts.
    #[test]
    fn a_font_preview_overrides_the_config_until_cleared() {
        in_headless_app(|cx| {
            let mut settings = Settings::new(Config::default(), None, cx);
            settings.installed = names(&["Cascadia Mono", "Consolas", "Arial", "Segoe UI"]);
            let (font, ui) = (settings.font(), settings.ui_font_family());
            settings.set_font_preview(Some(FontPreview {
                terminal_family: Some(FamilyChoice::Family("Consolas".into())),
                size: Some(20.0),
                line_height: Some(1.5),
                ui_family: Some(FamilyChoice::Family("Arial".into())),
            }));
            assert_eq!(settings.font().family.as_ref(), "Consolas");
            assert_eq!(settings.font_size(None), px(20.0));
            assert_eq!(settings.font_size(Some(11.0)), px(11.0));
            assert_eq!(settings.line_height(), 1.5);
            assert_eq!(settings.ui_font_family().as_ref(), "Arial");
            assert!(settings.font_preview().is_some());

            // Only what the preview changes is overridden.
            settings.set_font_preview(Some(FontPreview { size: Some(18.0), ..Default::default() }));
            assert_eq!(settings.font(), font);
            assert_eq!(settings.ui_font_family(), ui);
            assert_eq!(settings.line_height(), 1.25);
            settings.set_font_preview(Some(FontPreview {
                ui_family: Some(FamilyChoice::Default),
                ..Default::default()
            }));
            assert_eq!(settings.ui_font_family().as_ref(), "Segoe UI");

            settings.set_font_preview(None);
            assert_eq!(settings.font(), font);
            assert_eq!(settings.font_size(None), px(14.0));
            assert_eq!(settings.ui_font_family(), ui);
            // A preview that changes nothing is none.
            settings.set_font_preview(Some(FontPreview::default()));
            assert!(settings.font_preview().is_none());
        });
    }
}
