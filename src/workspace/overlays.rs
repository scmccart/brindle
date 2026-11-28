//! The window's overlays, apart from GPUI: which ones there are, and what
//! asking for one does while another is open. At most one shows at a time.

/// What a palette is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteRole {
    /// The profiles, to open one in a new tab.
    NewTab,
    /// The commands the focused tab can run.
    Commands,
    /// One line of text (`Rename Tab`).
    Prompt,
    /// The detached sessions, to attach or end one.
    Sessions,
    /// The profiles a split asks which of (OpenSpec change
    /// `zones-mixed-tabs`, decision 5), to split a zone with one.
    SplitProfile,
    /// An agent profile's earlier conversations (OpenSpec change
    /// `agents-session-list`), or which agent profile's to list.
    Conversations,
    /// An agent zone's session options, or one option's choices (OpenSpec
    /// change `agents-commands`).
    AgentOptions,
    /// Which agent zone takes a terminal's output (OpenSpec change
    /// `agents-prompt-context`).
    AgentTarget,
}

/// An overlay the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    Palette(PaletteRole),
    /// The settings dialog.
    Settings,
    /// The strip's `⌄` menu.
    ProfileMenu,
    /// A tab's or a zone header's menu.
    ContextMenu,
    /// The close confirmation for a busy agent (it replaces what is open).
    CloseConfirm,
    /// The confirmation for a link an agent wrote (it replaces what is
    /// open).
    LinkConfirm,
}

/// How asking for an overlay treats the one open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    /// Asked for again while it is open, it closes (a palette's or the
    /// settings dialog's key, the profile menu's).
    Toggles,
    /// It takes the place of whatever is open (a prompt a command asks
    /// for, a tab or zone menu, the sessions palette once its listings are
    /// in).
    Replaces,
}

/// What to do with the overlay asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// Nothing is open: open it.
    Open,
    /// Close what is open, then open it.
    CloseThenOpen,
    /// It is open and toggles: close it, and open nothing.
    CloseOnly,
}

/// What asking for `wanted` does while `open` shows (`None`: nothing).
pub fn toggle(open: Option<OverlayKind>, wanted: OverlayKind, how: Opens) -> Toggle {
    match (open, how) {
        (None, Opens::Toggles | Opens::Replaces) => Toggle::Open,
        (Some(open), Opens::Toggles) if open == wanted => Toggle::CloseOnly,
        (Some(_), Opens::Toggles | Opens::Replaces) => Toggle::CloseThenOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [OverlayKind; 13] = [
        OverlayKind::Palette(PaletteRole::NewTab),
        OverlayKind::Palette(PaletteRole::Commands),
        OverlayKind::Palette(PaletteRole::Prompt),
        OverlayKind::Palette(PaletteRole::Sessions),
        OverlayKind::Palette(PaletteRole::SplitProfile),
        OverlayKind::Palette(PaletteRole::Conversations),
        OverlayKind::Palette(PaletteRole::AgentOptions),
        OverlayKind::Palette(PaletteRole::AgentTarget),
        OverlayKind::Settings,
        OverlayKind::ProfileMenu,
        OverlayKind::ContextMenu,
        OverlayKind::CloseConfirm,
        OverlayKind::LinkConfirm,
    ];

    /// The workspace's rule before this function, for a palette asked for
    /// by its key (`close_palette_toggling`), the profile menu
    /// (`open_profile_menu`) and the settings dialog (`open_settings`):
    /// with an overlay open, it closes, and the one asked for opens unless
    /// it was that one.
    fn before_toggling(open: Option<OverlayKind>, wanted: OverlayKind) -> Toggle {
        let Some(open) = open else { return Toggle::Open };
        if open == wanted { Toggle::CloseOnly } else { Toggle::CloseThenOpen }
    }

    #[test]
    fn asking_again_for_the_open_overlay_closes_it() {
        for open in std::iter::once(None).chain(KINDS.map(Some)) {
            for wanted in KINDS {
                assert_eq!(toggle(open, wanted, Opens::Toggles), before_toggling(open, wanted), "{open:?} {wanted:?}");
            }
        }
        let commands = OverlayKind::Palette(PaletteRole::Commands);
        assert_eq!(toggle(Some(commands), commands, Opens::Toggles), Toggle::CloseOnly);
        assert_eq!(
            toggle(Some(commands), OverlayKind::Palette(PaletteRole::NewTab), Opens::Toggles),
            Toggle::CloseThenOpen,
            "another palette replaces it"
        );
        assert_eq!(toggle(Some(OverlayKind::ProfileMenu), OverlayKind::ProfileMenu, Opens::Toggles), Toggle::CloseOnly);
        assert_eq!(toggle(Some(OverlayKind::Settings), OverlayKind::Settings, Opens::Toggles), Toggle::CloseOnly);
        assert_eq!(toggle(None, OverlayKind::Settings, Opens::Toggles), Toggle::Open);
    }

    /// A prompt, a context menu, the close confirmation and the sessions
    /// palette once its listings are in replace what is open, even one of
    /// their own kind.
    #[test]
    fn an_overlay_that_replaces_never_only_closes() {
        for wanted in KINDS {
            assert_eq!(toggle(None, wanted, Opens::Replaces), Toggle::Open);
            for open in KINDS {
                assert_eq!(toggle(Some(open), wanted, Opens::Replaces), Toggle::CloseThenOpen, "{open:?} {wanted:?}");
            }
        }
    }
}
