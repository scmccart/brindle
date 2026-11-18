//! What the settings dialog previews on the window's tabs, apart from
//! GPUI: a theme for the tabs that follow the global one, and the profile
//! being edited, whose tabs show its theme and font size.

use crate::config::{Config, Profile};
use crate::theme::Theme;

#[derive(Debug, Default)]
pub struct Previews {
    /// The theme previewed for the tabs that follow the global theme.
    pub theme: Option<Theme>,
    /// The profile being edited, by its saved name, as its tabs preview it.
    pub profile: Option<(String, Profile)>,
}

impl Previews {
    /// Whether anything is previewed, so a new tab needs its look set.
    pub fn any(&self) -> bool {
        self.theme.is_some() || self.profile.is_some()
    }

    /// The theme a tab with `profile` shows: the previewed one when the
    /// profile follows the global theme, else the profile's theme in `config`.
    pub fn theme_for(&self, profile: &Profile, config: &Config) -> Theme {
        match (&self.theme, &profile.theme) {
            (Some(preview), None) => preview.clone(),
            (Some(_), Some(_)) | (None, _) => config.profile_theme(profile),
        }
    }

    /// A tab's profile, or the preview of it when it is the one being
    /// edited (names match ignoring case, as profile lookups do).
    pub fn profile_for<'a>(&'a self, profile: &'a Profile) -> &'a Profile {
        match &self.profile {
            Some((name, preview)) if name.eq_ignore_ascii_case(&profile.name) => preview,
            Some(_) | None => profile,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin_theme;

    fn theme(name: &str) -> Theme {
        builtin_theme(name).expect("a built-in theme")
    }

    fn profile(name: &str, theme: Option<&str>) -> Profile {
        Profile { name: name.into(), theme: theme.map(Into::into), ..Default::default() }
    }

    fn config() -> Config {
        let mut config = Config::default();
        config.theme = "brindle-dark".into();
        config
    }

    /// The previewed theme only on tabs that follow the global theme; a
    /// profile's own theme stands, preview or not.
    #[test]
    fn a_previewed_theme_shows_on_tabs_that_follow_the_global_one() {
        let (follows, own) = (profile("Shell", None), profile("Logs", Some("gruvbox-dark")));
        let none = Previews::default();
        assert_eq!(none.theme_for(&follows, &config()), theme("brindle-dark"));
        assert_eq!(none.theme_for(&own, &config()), theme("gruvbox-dark"));
        let previewing = Previews { theme: Some(theme("tokyo-night")), profile: None };
        assert_eq!(previewing.theme_for(&follows, &config()), theme("tokyo-night"));
        assert_eq!(previewing.theme_for(&own, &config()), theme("gruvbox-dark"));
    }

    /// The edited profile's preview stands in for it on its tabs, found by
    /// name ignoring case; other tabs keep their own.
    #[test]
    fn the_edited_profile_shows_on_its_tabs() {
        let edited = Profile { font_size: Some(18.0), ..profile("Work", Some("solarized-dark")) };
        let previews = Previews { theme: None, profile: Some(("work".into(), edited)) };
        let (work, shell) = (profile("Work", None), profile("Shell", None));
        let preview = previews.profile.as_ref().map(|(_, preview)| preview).expect("a preview");
        assert!(std::ptr::eq(previews.profile_for(&work), preview));
        assert_eq!(previews.profile_for(&work).font_size, Some(18.0));
        assert!(std::ptr::eq(previews.profile_for(&shell), &shell));
        assert!(std::ptr::eq(Previews::default().profile_for(&work), &work));
        // Its theme then comes from the preview too.
        assert_eq!(previews.theme_for(previews.profile_for(&work), &config()), theme("solarized-dark"));
    }

    #[test]
    fn any_preview_counts() {
        assert!(!Previews::default().any());
        assert!(Previews { theme: Some(theme("brindle-light")), profile: None }.any());
        assert!(Previews { theme: None, profile: Some(("Shell".into(), profile("Shell", None))) }.any());
    }
}
