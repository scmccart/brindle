//! Anchored placement: where a popover or a menu goes beside its trigger.
//!
//! [`place`] is pure. It puts a box against the trigger on the requested
//! [`Side`], aligned to the trigger's start, centre or end ([`Align`]) and
//! `offset` away, flips to the opposite side when the box doesn't fit there
//! within the viewport less `margin`, and clamps the result into that area.
//! The [`Anchored`] element lays out its children, measures them and moves
//! them where `place` says. It works wherever it is rendered, in a deferred
//! draw or inline, because layout bounds are window coordinates either way.
//!
//! gpui's own `anchored` knows only corners: it can't centre a box on its
//! trigger or shift it along one axis, which is why the crate has its own.
//!
//! [`available_height`] is how tall a box may be and still fit on one side
//! of its trigger: a menu longer than that is capped there and scrolls.

use gpui::{
    AnyElement, App, Bounds, Display, Div, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, ParentElement, Pixels, Point, Position, Size, Style, Window, div, point, px,
};

/// The gap [`Anchored`] leaves between a trigger and its box by default,
/// shadcn's `sideOffset={4}`.
pub const SIDE_OFFSET: Pixels = px(4.);

/// The least distance [`Anchored`] keeps from the window's edges by default.
pub const EDGE_MARGIN: Pixels = px(8.);

/// The side of the trigger a box is placed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

impl Side {
    /// The side a box flips to when it doesn't fit on this one.
    pub fn opposite(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// How a box lines up with its trigger along the trigger's edge: a box
/// below or above it aligns to its left edge, its centre or its right
/// edge; a box beside it to its top, its centre or its bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

/// Where [`place`] put the box: its top-left corner, and the side it ended
/// up on after flipping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub origin: Point<Pixels>,
    pub side: Side,
}

/// The top-left corner of a box of `size` placed against `trigger` on
/// `side`, aligned by `align`, `offset` away from it, inside a viewport
/// `viewport` large less `margin` at each edge.
///
/// When the box doesn't fit on `side` (it would cross the margin on that
/// side's axis), it flips to the opposite side if it fits there. When it
/// fits on neither, it stays on `side`. Either way the result is then
/// clamped into the viewport less the margin on both axes, which also
/// shifts a box that crosses an edge along the trigger; a box larger than
/// that area is pinned at the margin on its top or left.
pub fn place(
    trigger: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
    side: Side,
    align: Align,
    offset: Pixels,
    margin: Pixels,
) -> Placement {
    let at = |side| beside(trigger, size, side, align, offset);
    let fits = |side| fits(at(side), size, viewport, margin, side);
    let side = if fits(side) || !fits(side.opposite()) { side } else { side.opposite() };
    Placement { origin: clamp(at(side), size, viewport, margin), side }
}

/// The greatest height a box placed against `trigger` on `side`, `offset`
/// away, can have and still fit in a viewport `viewport` large less
/// `margin` at each edge, which is Radix's available height. Above or
/// below the trigger it is the larger room of the two, so a box capped at
/// it fits on one of them ([`place`] flips it there) and never covers its
/// trigger. Beside the trigger it is the viewport's height less the
/// margins, since [`place`] only shifts a box along that axis. Never
/// negative.
pub fn available_height(trigger: Bounds<Pixels>, viewport: Size<Pixels>, side: Side, offset: Pixels, margin: Pixels) -> Pixels {
    let room = match side {
        Side::Top | Side::Bottom => {
            let below = viewport.height - margin - (trigger.bottom() + offset);
            let above = trigger.top() - offset - margin;
            below.max(above)
        }
        Side::Left | Side::Right => viewport.height - margin * 2.,
    };
    room.max(px(0.))
}

/// The box's corner against the trigger on `side`, before fitting.
fn beside(trigger: Bounds<Pixels>, size: Size<Pixels>, side: Side, align: Align, offset: Pixels) -> Point<Pixels> {
    let x = match side {
        Side::Left => trigger.left() - offset - size.width,
        Side::Right => trigger.right() + offset,
        Side::Top | Side::Bottom => aligned(trigger.left(), trigger.size.width, size.width, align),
    };
    let y = match side {
        Side::Top => trigger.top() - offset - size.height,
        Side::Bottom => trigger.bottom() + offset,
        Side::Left | Side::Right => aligned(trigger.top(), trigger.size.height, size.height, align),
    };
    point(x, y)
}

/// Where a length `len` starts when aligned by `align` with a trigger edge
/// that runs from `start` for `extent`.
fn aligned(start: Pixels, extent: Pixels, len: Pixels, align: Align) -> Pixels {
    match align {
        Align::Start => start,
        Align::Center => start + (extent - len) * 0.5,
        Align::End => start + extent - len,
    }
}

/// Whether a box at `origin` stays within the margin on the axis `side`
/// pushes it out along. The other axis is clamped, not flipped.
fn fits(origin: Point<Pixels>, size: Size<Pixels>, viewport: Size<Pixels>, margin: Pixels, side: Side) -> bool {
    match side {
        Side::Top => origin.y >= margin,
        Side::Bottom => origin.y + size.height <= viewport.height - margin,
        Side::Left => origin.x >= margin,
        Side::Right => origin.x + size.width <= viewport.width - margin,
    }
}

/// `origin` moved into the viewport less the margin on both axes.
fn clamp(origin: Point<Pixels>, size: Size<Pixels>, viewport: Size<Pixels>, margin: Pixels) -> Point<Pixels> {
    point(
        clamp_axis(origin.x, size.width, viewport.width, margin),
        clamp_axis(origin.y, size.height, viewport.height, margin),
    )
}

/// `start` moved so that `len` from it stays within `margin..extent -
/// margin`; when it can't, it is pinned at the near margin.
fn clamp_axis(start: Pixels, len: Pixels, extent: Pixels, margin: Pixels) -> Pixels {
    let max = (extent - margin - len).max(margin);
    start.clamp(margin, max)
}

/// The smallest bounds holding all of `bounds`, or `None` with none.
fn union(bounds: impl IntoIterator<Item = Bounds<Pixels>>) -> Option<Bounds<Pixels>> {
    let mut bounds = bounds.into_iter();
    let first = bounds.next()?;
    let (origin, far) = bounds.fold((first.origin, first.bottom_right()), |(origin, far), b| {
        (origin.min(&b.origin), far.max(&b.bottom_right()))
    });
    Some(Bounds::from_corners(origin, far))
}

/// An element that places its children against a trigger with [`place`].
/// It takes no room in its parent's flow (it is positioned absolutely) and
/// moves its children's bounds to the placement, so hitboxes and paint land
/// there. Its children should carry no margin, which would count towards
/// the measured size without moving.
///
/// By default the box goes below the trigger, centred, [`SIDE_OFFSET`] (4px,
/// shadcn's `sideOffset`) away, and stays [`EDGE_MARGIN`] (8px) from the
/// window's edges.
pub struct Anchored {
    trigger: Bounds<Pixels>,
    side: Side,
    align: Align,
    offset: Pixels,
    margin: Pixels,
    children: Vec<AnyElement>,
}

impl Anchored {
    /// A box placed against `trigger`, whose bounds are in window
    /// coordinates (as [`anchor_trigger`] records them).
    pub fn new(trigger: Bounds<Pixels>) -> Self {
        Self {
            trigger,
            side: Side::default(),
            align: Align::default(),
            offset: SIDE_OFFSET,
            margin: EDGE_MARGIN,
            children: Vec::new(),
        }
    }

    pub fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// The gap between the trigger and the box.
    pub fn offset(mut self, offset: Pixels) -> Self {
        self.offset = offset;
        self
    }

    /// The least distance kept from the window's edges.
    pub fn margin(mut self, margin: Pixels) -> Self {
        self.margin = margin;
        self
    }
}

/// A `div` around `trigger` that hands the trigger's bounds, in window
/// coordinates, to `record` each frame, for its owner to keep (in keyed
/// state, say) and give to [`Anchored::new`] when the overlay opens. An
/// overlay opened by a click uses the bounds of the frame that was clicked,
/// which are current.
pub fn anchor_trigger(
    trigger: impl IntoElement,
    record: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
) -> Div {
    div().child(trigger).on_children_prepainted(move |bounds, window, cx| {
        if let Some(bounds) = bounds.first() {
            record(*bounds, window, cx);
        }
    })
}

impl ParentElement for Anchored {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

/// The children's layout ids, kept from layout to prepaint.
pub struct AnchoredLayout {
    child_layout_ids: Vec<LayoutId>,
}

impl Element for Anchored {
    type RequestLayoutState = AnchoredLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnchoredLayout) {
        let child_layout_ids: Vec<LayoutId> =
            self.children.iter_mut().map(|child| child.request_layout(window, cx)).collect();
        // Absolute, so the box takes no room in the owner's flow: `place`
        // positions it, not its parent.
        let style = Style { position: Position::Absolute, display: Display::Flex, ..Style::default() };
        let layout_id = window.request_layout(style, child_layout_ids.iter().copied(), cx);
        (layout_id, AnchoredLayout { child_layout_ids })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        layout: &mut AnchoredLayout,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Layout bounds include the current element offset, so they are
        // window coordinates in a deferred draw and inline alike.
        let Some(extent) = union(layout.child_layout_ids.iter().map(|id| window.layout_bounds(*id))) else {
            return;
        };
        let placement = place(
            self.trigger,
            extent.size,
            window.viewport_size(),
            self.side,
            self.align,
            self.offset,
            self.margin,
        );
        let shift = placement.origin - extent.origin;
        let shift = point(shift.x.round(), shift.y.round());
        window.with_element_offset(shift, |window| {
            for child in &mut self.children {
                child.prepaint(window, cx);
            }
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut AnchoredLayout,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for Anchored {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{bounds, size};

    const OFFSET: Pixels = px(4.);
    const MARGIN: Pixels = px(8.);

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        bounds(point(px(x), px(y)), size(px(w), px(h)))
    }

    /// A 50x20 trigger at (100, 100), a 30x10 box and room all round.
    fn place_in_room(side: Side, align: Align) -> Placement {
        place(b(100., 100., 50., 20.), size(px(30.), px(10.)), size(px(500.), px(500.)), side, align, OFFSET, MARGIN)
    }

    fn at(x: f32, y: f32, side: Side) -> Placement {
        Placement { origin: point(px(x), px(y)), side }
    }

    #[test]
    fn each_side_and_align() {
        use Align::{Center, End, Start};
        // Below: 4px under the trigger's bottom edge (120), along its width.
        assert_eq!(place_in_room(Side::Bottom, Start), at(100., 124., Side::Bottom));
        assert_eq!(place_in_room(Side::Bottom, Center), at(110., 124., Side::Bottom));
        assert_eq!(place_in_room(Side::Bottom, End), at(120., 124., Side::Bottom));
        // Above: the box's bottom 4px over the trigger's top edge.
        assert_eq!(place_in_room(Side::Top, Start), at(100., 86., Side::Top));
        assert_eq!(place_in_room(Side::Top, Center), at(110., 86., Side::Top));
        assert_eq!(place_in_room(Side::Top, End), at(120., 86., Side::Top));
        // Beside: 4px past the trigger's right edge (150), along its height.
        assert_eq!(place_in_room(Side::Right, Start), at(154., 100., Side::Right));
        assert_eq!(place_in_room(Side::Right, Center), at(154., 105., Side::Right));
        assert_eq!(place_in_room(Side::Right, End), at(154., 110., Side::Right));
        assert_eq!(place_in_room(Side::Left, Start), at(66., 100., Side::Left));
        assert_eq!(place_in_room(Side::Left, Center), at(66., 105., Side::Left));
        assert_eq!(place_in_room(Side::Left, End), at(66., 110., Side::Left));
    }

    #[test]
    fn flips_at_each_edge() {
        let viewport = size(px(300.), px(300.));
        let box_size = size(px(40.), px(40.));
        let go = |trigger, side| place(trigger, box_size, viewport, side, Align::Start, OFFSET, MARGIN);
        // A trigger near the bottom: below would end at 294 + 40, past the
        // margin, so the box goes above (250 - 4 - 40).
        assert_eq!(go(b(100., 250., 50., 20.), Side::Bottom), at(100., 206., Side::Top));
        // Near the top: above would start at 10 - 44 < 0.
        assert_eq!(go(b(100., 10., 50., 20.), Side::Top), at(100., 34., Side::Bottom));
        // Near the right: beside would end at 304 + 40.
        assert_eq!(go(b(250., 100., 50., 20.), Side::Right), at(206., 100., Side::Left));
        // Near the left: left would start at 10 - 44.
        assert_eq!(go(b(10., 100., 50., 20.), Side::Left), at(64., 100., Side::Right));
        // The margin counts: a box ending exactly on the margin fits, one
        // pixel further doesn't.
        assert_eq!(go(b(100., 248., 50., 0.), Side::Bottom).side, Side::Bottom);
        assert_eq!(go(b(100., 249., 50., 0.), Side::Bottom).side, Side::Top);
    }

    #[test]
    fn clamps_when_neither_side_fits() {
        let viewport = size(px(100.), px(100.));
        // A 60-tall box under a trigger in the middle of a 100-tall
        // viewport: below ends at 114, above starts at -24. It stays below
        // and is clamped up to end on the margin (100 - 8 - 60 = 32).
        let placement =
            place(b(20., 40., 50., 10.), size(px(30.), px(60.)), viewport, Side::Bottom, Align::Start, OFFSET, MARGIN);
        assert_eq!(placement, at(20., 32., Side::Bottom));
        // The same beside the trigger.
        let placement =
            place(b(40., 20., 10., 50.), size(px(60.), px(30.)), viewport, Side::Right, Align::Start, OFFSET, MARGIN);
        assert_eq!(placement, at(32., 20., Side::Right));
        // A box taller than the viewport less the margins is pinned at the
        // top margin, below the trigger still.
        let placement =
            place(b(20., 40., 50., 10.), size(px(30.), px(200.)), viewport, Side::Bottom, Align::Start, OFFSET, MARGIN);
        assert_eq!(placement, at(20., 8., Side::Bottom));
    }

    #[test]
    fn clamps_along_the_trigger() {
        let viewport = size(px(300.), px(300.));
        // Centred under a trigger at the right edge, the box would cross
        // it; it is shifted left to the margin and stays below.
        let placement = place(
            b(270., 100., 30., 20.),
            size(px(80.), px(10.)),
            viewport,
            Side::Bottom,
            Align::Center,
            OFFSET,
            MARGIN,
        );
        assert_eq!(placement, at(212., 124., Side::Bottom));
        // Start-aligned at the left edge, a box beside the trigger is
        // shifted down to the margin.
        let placement =
            place(b(100., 0., 30., 20.), size(px(10.), px(80.)), viewport, Side::Right, Align::End, OFFSET, MARGIN);
        assert_eq!(placement, at(134., 8., Side::Right));
    }

    /// A context menu's anchor: a zero-size trigger at the pointer, the box
    /// beside it on the right with no offset, aligned to its start (Radix's
    /// `side="right" align="start"`).
    fn at_pointer(x: f32, y: f32, box_size: Size<Pixels>, viewport: Size<Pixels>) -> Placement {
        place(b(x, y, 0., 0.), box_size, viewport, Side::Right, Align::Start, px(0.), MARGIN)
    }

    #[test]
    fn a_zero_size_trigger_puts_the_corner_at_the_point() {
        let viewport = size(px(500.), px(500.));
        let menu = size(px(100.), px(80.));
        assert_eq!(at_pointer(200., 40., menu, viewport), at(200., 40., Side::Right));
        // Below it, as a dropdown from a point would go.
        let below = place(b(200., 40., 0., 0.), menu, viewport, Side::Bottom, Align::Start, px(0.), MARGIN);
        assert_eq!(below, at(200., 40., Side::Bottom));
    }

    /// Near the right edge the box flips to the left of the point; near
    /// the bottom it is shifted up to end on the margin and stays right of
    /// the point; in the corner both.
    #[test]
    fn a_zero_size_trigger_flips_and_clamps_at_the_edges() {
        let viewport = size(px(300.), px(300.));
        let menu = size(px(100.), px(80.));
        // Right: 270 + 100 would end past 292, so it ends at the point.
        assert_eq!(at_pointer(270., 100., menu, viewport), at(170., 100., Side::Left));
        // Bottom: 260 + 80 would end past 292; clamped to 292 - 80.
        assert_eq!(at_pointer(50., 260., menu, viewport), at(50., 212., Side::Right));
        // The corner: flipped left and clamped up.
        assert_eq!(at_pointer(270., 260., menu, viewport), at(170., 212., Side::Left));
        // On the bottom side instead, the bottom edge flips and the right
        // edge clamps.
        let go = |x, y| place(b(x, y, 0., 0.), menu, viewport, Side::Bottom, Align::Start, px(0.), MARGIN);
        assert_eq!(go(50., 260.), at(50., 180., Side::Top));
        assert_eq!(go(270., 100.), at(192., 100., Side::Bottom));
        // Every placement is inside the viewport less the margin.
        for (x, y) in [(270., 100.), (50., 260.), (270., 260.), (0., 0.), (300., 300.)] {
            let placed = at_pointer(x, y, menu, viewport).origin;
            assert!(placed.x >= MARGIN && placed.x + menu.width <= viewport.width - MARGIN, "{placed:?}");
            assert!(placed.y >= MARGIN && placed.y + menu.height <= viewport.height - MARGIN, "{placed:?}");
        }
    }

    /// A menu taller than the window is capped at the available height and
    /// then fits: beside a point, the window less its margins; above or
    /// below a trigger, the larger room of the two, on that side, so it
    /// never covers the trigger.
    #[test]
    fn a_menu_taller_than_the_window_is_limited_to_it() {
        let viewport = size(px(300.), px(300.));
        let tall = |height: Pixels| size(px(100.), height);
        // Beside a point near the bottom: the whole height less the margins.
        let height = available_height(b(50., 260., 0., 0.), viewport, Side::Right, px(0.), MARGIN);
        assert_eq!(height, px(284.));
        assert_eq!(at_pointer(50., 260., tall(height), viewport), at(50., 8., Side::Right));
        // Below a trigger near the top: the room below (300 - 8 - 64).
        let trigger = b(20., 40., 50., 20.);
        let height = available_height(trigger, viewport, Side::Bottom, OFFSET, MARGIN);
        assert_eq!(height, px(228.));
        let placed = place(trigger, tall(height), viewport, Side::Bottom, Align::Start, OFFSET, MARGIN);
        assert_eq!(placed, at(20., 64., Side::Bottom));
        assert!(placed.origin.y >= trigger.bottom());
        // Below a trigger near the bottom: the room above, where it flips.
        let trigger = b(20., 250., 50., 20.);
        let height = available_height(trigger, viewport, Side::Bottom, OFFSET, MARGIN);
        assert_eq!(height, px(238.));
        let placed = place(trigger, tall(height), viewport, Side::Bottom, Align::Start, OFFSET, MARGIN);
        assert_eq!(placed, at(20., 8., Side::Top));
        assert!(placed.origin.y + height <= trigger.top());
        // No room at all is no height, never a negative one.
        assert_eq!(available_height(b(0., 0., 10., 10.), size(px(10.), px(10.)), Side::Left, OFFSET, MARGIN), px(0.));
        assert_eq!(available_height(b(0., 0., 10., 10.), size(px(10.), px(10.)), Side::Bottom, OFFSET, MARGIN), px(0.));
    }

    #[test]
    fn union_of_bounds() {
        assert_eq!(union([]), None);
        assert_eq!(union([b(10., 10., 5., 5.)]), Some(b(10., 10., 5., 5.)));
        assert_eq!(union([b(10., 10., 5., 5.), b(0., 20., 5., 5.)]), Some(b(0., 10., 15., 15.)));
    }

    #[test]
    fn opposite_sides() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            assert_ne!(side.opposite(), side);
            assert_eq!(side.opposite().opposite(), side);
        }
    }
}
