//! The mouse pointer over a terminal's grid: the platform pointer each
//! `OSC 22` shape shows as, and which pointer wins ([`choose`]). The stacks,
//! names and replies are `brindle-term-core`'s, re-exported here.

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

#[cfg(test)]
mod tests {
    use super::*;
    use brindle_term_core::pointer::parse;

    /// The CSS name GPUI documents for each pointer, matched without a
    /// wildcard, so a pointer GPUI adds is looked at here.
    fn css_name(style: CursorStyle) -> Option<&'static str> {
        Some(match style {
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
            CursorStyle::None => return None,
        })
    }

    #[test]
    fn every_shape_shows_as_the_pointer_of_its_name() {
        for shape in Shape::ALL {
            let style = cursor_style(shape);
            let want = if shape == Shape::NoDrop { "not-allowed" } else { shape.name() };
            assert_eq!(css_name(style), Some(want), "{shape:?} shows as {style:?}");
        }
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
