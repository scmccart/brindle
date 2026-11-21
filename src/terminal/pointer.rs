//! The mouse pointer over a terminal's grid: the platform pointer each
//! `OSC 22` shape shows as, which pointer wins ([`choose`]), and when a
//! frame's pointer must be shown at once ([`Shown`]). The stacks, names and
//! replies are `brindle-term-core`'s, re-exported here.

pub use brindle_term_core::pointer::{Op, Screen, Shape, Stacks};

use gpui::CursorStyle;

/// The platform pointer `shape` shows as. Every shape has one of its own
/// meaning (GPUI's names follow CSS); `no-drop` shares `not-allowed`'s,
/// as GPUI has no other.
pub fn cursor_style(shape: Shape) -> CursorStyle {
    match shape {
        Shape::Default => CursorStyle::Arrow,
        Shape::Text => CursorStyle::IBeam,
        Shape::VerticalText => CursorStyle::IBeamCursorForVerticalLayout,
        Shape::Pointer => CursorStyle::PointingHand,
        Shape::Crosshair => CursorStyle::Crosshair,
        Shape::Grab => CursorStyle::OpenHand,
        Shape::Grabbing => CursorStyle::ClosedHand,
        Shape::NotAllowed | Shape::NoDrop => CursorStyle::OperationNotAllowed,
        Shape::Alias => CursorStyle::DragLink,
        Shape::Copy => CursorStyle::DragCopy,
        Shape::ContextMenu => CursorStyle::ContextualMenu,
        Shape::EwResize => CursorStyle::ResizeLeftRight,
        Shape::NsResize => CursorStyle::ResizeUpDown,
        Shape::EResize => CursorStyle::ResizeRight,
        Shape::WResize => CursorStyle::ResizeLeft,
        Shape::NResize => CursorStyle::ResizeUp,
        Shape::SResize => CursorStyle::ResizeDown,
        Shape::NwseResize => CursorStyle::ResizeUpLeftDownRight,
        Shape::NeswResize => CursorStyle::ResizeUpRightDownLeft,
        Shape::ColResize => CursorStyle::ResizeColumn,
        Shape::RowResize => CursorStyle::ResizeRow,
    }
}

/// The pointer over the grid: the hand while a link is ctrl-hovered
/// (`link_hovered`), the text beam while a selection is dragged
/// (`selecting`), else the program's shape (`program`, the top of the
/// active screen's stack), else Brindle's own: the arrow while the program
/// reports the mouse (`mouse_mode`), the text beam otherwise.
pub fn choose(link_hovered: bool, selecting: bool, program: Option<Shape>, mouse_mode: bool) -> CursorStyle {
    if link_hovered {
        CursorStyle::PointingHand
    } else if selecting {
        CursorStyle::IBeam
    } else if let Some(shape) = program {
        cursor_style(shape)
    } else if mouse_mode {
        CursorStyle::Arrow
    } else {
        CursorStyle::IBeam
    }
}

/// The pointer the grid's last painted frame chose ([`choose`]), kept to
/// say when the platform must be asked to show a new one at once.
///
/// GPUI sets the pointer as each frame ends, and the mouse's own moves show
/// it. A pointer that changes with no move (a press or release starting or
/// ending a selection drag, a ctrl press over a link, a program's `OSC 22`)
/// shows at once on Wayland and X11, but on Windows GPUI only records it
/// until the window's next `WM_SETCURSOR`, which comes with the next move,
/// and never while a pressed button holds the mouse captured. So such a
/// frame asks for it to be shown (`crate::cursor::show_now`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shown(Option<CursorStyle>);

impl Shown {
    /// Records a frame's `chosen` pointer, and says whether the platform
    /// must show it now: it differs from the last frame's while the mouse
    /// is over the grid (`over_grid`). Elsewhere the grid's pointer isn't
    /// the one showing.
    pub fn frame(&mut self, chosen: CursorStyle, over_grid: bool) -> bool {
        let changed = self.0 != Some(chosen);
        self.0 = Some(chosen);
        changed && over_grid
    }

    /// The pointer the last painted frame chose, if one was painted.
    pub fn last(self) -> Option<CursorStyle> {
        self.0
    }
}

/// The CSS name GPUI documents for each pointer, for the debug dump (`none`
/// for a hidden one). Matched without a wildcard, so a pointer GPUI adds is
/// looked at here.
pub fn css_name(style: CursorStyle) -> &'static str {
    match style {
        CursorStyle::Arrow => "default",
        CursorStyle::IBeam => "text",
        CursorStyle::Crosshair => "crosshair",
        CursorStyle::ClosedHand => "grabbing",
        CursorStyle::OpenHand => "grab",
        CursorStyle::PointingHand => "pointer",
        CursorStyle::ResizeLeft => "w-resize",
        CursorStyle::ResizeRight => "e-resize",
        CursorStyle::ResizeLeftRight => "ew-resize",
        CursorStyle::ResizeUp => "n-resize",
        CursorStyle::ResizeDown => "s-resize",
        CursorStyle::ResizeUpDown => "ns-resize",
        // GPUI's comments swap these two, but its Wayland and X11 code
        // shows up-left/down-right as `nwse-resize`.
        CursorStyle::ResizeUpLeftDownRight => "nwse-resize",
        CursorStyle::ResizeUpRightDownLeft => "nesw-resize",
        CursorStyle::ResizeColumn => "col-resize",
        CursorStyle::ResizeRow => "row-resize",
        CursorStyle::IBeamCursorForVerticalLayout => "vertical-text",
        CursorStyle::OperationNotAllowed => "not-allowed",
        CursorStyle::DragLink => "alias",
        CursorStyle::DragCopy => "copy",
        CursorStyle::ContextualMenu => "context-menu",
        CursorStyle::None => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brindle_term_core::pointer::parse;

    #[test]
    fn every_shape_shows_as_the_pointer_of_its_name() {
        for shape in Shape::ALL {
            let style = cursor_style(shape);
            let want = if shape == Shape::NoDrop { "not-allowed" } else { shape.name() };
            assert_eq!(css_name(style), want, "{shape:?} shows as {style:?}");
        }
    }

    #[test]
    fn a_selection_drag_over_a_programs_shape_shows_the_beam_from_press_to_release() {
        // The pointer each frame chooses over a program's crosshair, and
        // whether it is shown at once: the press and the release change it
        // with no mouse move, the frames of the drag between don't.
        let program = Some(Shape::Crosshair);
        let mut shown = Shown::default();
        let mut frame = |selecting: bool, over_grid: bool| {
            let chosen = choose(false, selecting, program, false);
            (chosen, shown.frame(chosen, over_grid))
        };
        assert_eq!(frame(false, true), (CursorStyle::Crosshair, true), "the first frame");
        assert_eq!(frame(false, true), (CursorStyle::Crosshair, false), "output, no change");
        assert_eq!(frame(true, true), (CursorStyle::IBeam, true), "the press");
        assert_eq!(frame(true, true), (CursorStyle::IBeam, false), "the drag");
        assert_eq!(frame(false, true), (CursorStyle::Crosshair, true), "the release");
        assert_eq!(shown.last(), Some(CursorStyle::Crosshair));
    }

    #[test]
    fn a_change_away_from_the_grid_is_recorded_but_not_shown() {
        let mut shown = Shown::default();
        assert_eq!(shown.last(), None, "nothing painted yet");
        assert!(!shown.frame(CursorStyle::IBeam, false), "the mouse is elsewhere");
        assert_eq!(shown.last(), Some(CursorStyle::IBeam));
        // Released past the grid's edge: the next frame over it, with no
        // change, needs nothing; a change there does.
        assert!(!shown.frame(CursorStyle::IBeam, true));
        assert!(!shown.frame(CursorStyle::Crosshair, false));
        assert!(shown.frame(CursorStyle::PointingHand, true));
    }

    /// Every pointer has its own CSS name.
    #[test]
    fn css_names_are_distinct() {
        let all = [
            CursorStyle::Arrow,
            CursorStyle::IBeam,
            CursorStyle::Crosshair,
            CursorStyle::ClosedHand,
            CursorStyle::OpenHand,
            CursorStyle::PointingHand,
            CursorStyle::ResizeLeft,
            CursorStyle::ResizeRight,
            CursorStyle::ResizeLeftRight,
            CursorStyle::ResizeUp,
            CursorStyle::ResizeDown,
            CursorStyle::ResizeUpDown,
            CursorStyle::ResizeUpLeftDownRight,
            CursorStyle::ResizeUpRightDownLeft,
            CursorStyle::ResizeColumn,
            CursorStyle::ResizeRow,
            CursorStyle::IBeamCursorForVerticalLayout,
            CursorStyle::OperationNotAllowed,
            CursorStyle::DragLink,
            CursorStyle::DragCopy,
            CursorStyle::ContextualMenu,
            CursorStyle::None,
        ];
        let names: std::collections::BTreeSet<_> = all.iter().map(|&style| css_name(style)).collect();
        assert_eq!(names.len(), all.len());
    }

    #[test]
    fn the_link_then_the_selection_then_the_program_then_brindles_own() {
        let program = Some(Shape::Crosshair);
        // A ctrl-hovered link wins over everything, and the program's
        // shape returns when ctrl is released.
        assert_eq!(choose(true, false, program, true), CursorStyle::PointingHand);
        assert_eq!(choose(true, true, program, false), CursorStyle::PointingHand);
        assert_eq!(choose(false, false, program, false), CursorStyle::Crosshair);
        // A selection drag shows the text beam.
        assert_eq!(choose(false, true, program, true), CursorStyle::IBeam);
        // The program's shape applies whether it reports the mouse or not.
        assert_eq!(choose(false, false, program, true), CursorStyle::Crosshair);
        assert_eq!(choose(false, false, Some(Shape::EwResize), false), CursorStyle::ResizeLeftRight);
        // None: Brindle's own.
        assert_eq!(choose(false, false, None, true), CursorStyle::Arrow);
        assert_eq!(choose(false, false, None, false), CursorStyle::IBeam);
        assert_eq!(choose(true, false, None, false), CursorStyle::PointingHand);
    }

    #[test]
    fn the_specs_fallback_scenarios_show_the_hand_and_keep_the_crosshair() {
        let top = |bodies: &[&[u8]]| {
            let mut stacks = Stacks::default();
            for op in bodies.iter().filter_map(|body| parse(body)) {
                stacks.apply(Screen::Primary, op);
            }
            stacks.top(Screen::Primary)
        };
        assert_eq!(choose(false, false, top(&[b"zoom-in,pointer"]), false), CursorStyle::PointingHand);
        assert_eq!(choose(false, false, top(&[b"crosshair", b"zoom-in"]), false), CursorStyle::Crosshair);
        assert_eq!(choose(false, false, top(&[b">crosshair", b""]), true), CursorStyle::Arrow);
    }
}
