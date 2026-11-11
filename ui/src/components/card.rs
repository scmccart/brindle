//! `Card` and `CardHeader`: shadcn's Card, compact, for a titled pane.
//!
//! A `RenderOnce` builder, like the other components: the owner passes the
//! header, the content and the colours on each render. A card is a
//! bordered, rounded surface (`card_style`) with an optional one-line
//! header (a leading adornment, the title, an action slot) over its
//! content, which fills the rest of the card with no padding of its own.
//! It knows nothing of what it holds: an app drawing a terminal in it
//! passes the terminal's colours ([`Card::colors`]).
//!
//! **Rounded corners over content.** gpui 0.2.2 clips to rectangles only,
//! so a content that paints a square background to its box (a terminal)
//! would show past the rounded border. The card paints, after its
//! children and before its border (gpui draws a box's border after its
//! children), the four pieces outside its rounded edge in the surround
//! colour, the one the card sits on ([`corner_cutouts`]). The content is
//! untouched and needs no clipping. The pieces are paint only: a press on
//! one reaches what is beneath, the content.
//!
//! **Ring.** With `ring(true)` the card draws the focus ring, a 3px band
//! just outside its border that takes no room. The owner decides when (a
//! focused pane, from the keyboard: [`shows_ring`](crate::shows_ring)).
//! Nothing is clipped round the card, so the ring needs room outside it
//! (a gap between cards).

use gpui::{
    AnyElement, App, Bounds, Div, ElementId, Hsla, InteractiveElement, Interactivity, IntoElement, ParentElement,
    PathBuilder, Pixels, Point, Refineable, RenderOnce, SharedString, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, canvas, div, point, prelude::FluentBuilder, px,
};

use crate::components::FocusRing;
use crate::components::tabs::trigger_label;
use crate::style::{CardColors, CardStyle, card_style};
use crate::theme::ActiveTheme;

/// A card's header row: an optional leading adornment (a status dot), the
/// title, cut with an ellipsis when the row is narrow, and an optional
/// action slot at its end, over a border line.
///
/// It is a stateful element, so the owner can give it handlers of its own
/// (a press that focuses the pane, a context menu through
/// `ContextMenuTarget`); the card draws nothing interactive itself.
pub struct CardHeader {
    base: Stateful<Div>,
    leading: Vec<AnyElement>,
    title: SharedString,
    title_muted: bool,
    actions: Vec<AnyElement>,
}

impl CardHeader {
    /// A header with no title. `id` keys its handlers.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            leading: Vec::new(),
            title: SharedString::default(),
            title_muted: false,
            actions: Vec::new(),
        }
    }

    /// Something before the title: a status dot, an icon. It keeps its size
    /// when the title is cut.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading.push(element.into_any_element());
        self
    }

    /// The title, in the card's text colour, on one line cut with an
    /// ellipsis at the room left (`trigger_label`).
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }

    /// Draws the title in the card's muted colour instead (an inactive
    /// pane's).
    pub fn title_muted(mut self, muted: bool) -> Self {
        self.title_muted = muted;
        self
    }

    /// Something at the header's end, in the card's muted colour: icon
    /// buttons. Each call adds one, in order.
    pub fn action(mut self, element: impl IntoElement) -> Self {
        self.actions.push(element.into_any_element());
        self
    }

    /// The header row in style `s`.
    fn build(self, s: &CardStyle) -> Stateful<Div> {
        let Self { mut base, leading, title, title_muted, actions } = self;
        // The owner's refinements (a cursor), applied over the header's own.
        let owner = std::mem::take(base.style());
        let mut header = base
            .flex()
            .flex_none()
            .items_center()
            .h(s.header_height)
            .pl(s.header_padding_left)
            .pr(s.header_padding_right)
            .gap(s.header_gap)
            .border_b(s.border_width)
            .border_color(s.border)
            .text_size(s.text_size)
            .text_color(if title_muted { s.muted } else { s.text })
            .children(leading)
            .child(trigger_label(title))
            .when(!actions.is_empty(), |d| {
                d.child(div().flex().flex_none().items_center().gap_0p5().text_color(s.muted).children(actions))
            });
        header.style().refine(&owner);
        header
    }
}

impl InteractiveElement for CardHeader {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

/// The owner's refinements (a cursor, say) win over the header's own.
impl Styled for CardHeader {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl StatefulInteractiveElement for CardHeader {}

type BoundsHandler = Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static>;

/// A card. Build it with [`Card::new`], give it a header, content and
/// colours, refine its layout through `Styled` (`size_full`, `flex_1`),
/// and render it.
#[derive(IntoElement)]
pub struct Card {
    id: ElementId,
    header: Option<CardHeader>,
    content: Vec<AnyElement>,
    colors: CardColors,
    ring: bool,
    on_header_bounds: Option<BoundsHandler>,
    style: StyleRefinement,
}

impl Card {
    /// A card with no header and no content. `id` names it among its
    /// siblings.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            header: None,
            content: Vec::new(),
            colors: CardColors::default(),
            ring: false,
            on_header_bounds: None,
            style: StyleRefinement::default(),
        }
    }

    /// The header row. Without one the content fills the whole card inside
    /// its border.
    pub fn header(mut self, header: CardHeader) -> Self {
        self.header = Some(header);
        self
    }

    /// Content, below the header. It fills the rest of the card, with no
    /// padding of its own.
    pub fn child(mut self, element: impl IntoElement) -> Self {
        self.content.push(element.into_any_element());
        self
    }

    pub fn children(mut self, elements: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.content.extend(elements.into_iter().map(IntoElement::into_any_element));
        self
    }

    /// The card's own colours; those left `None` come from the tokens.
    pub fn colors(mut self, colors: CardColors) -> Self {
        self.colors = colors;
        self
    }

    /// Draws the focus ring just outside the border.
    pub fn ring(mut self, shown: bool) -> Self {
        self.ring = shown;
        self
    }

    /// Told where the header was laid out, in window coordinates, on every
    /// frame it is drawn (during prepaint, so it must not notify): for
    /// placing a menu below it, say.
    pub fn on_header_bounds(mut self, handler: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_header_bounds = Some(Box::new(handler));
        self
    }
}

impl Styled for Card {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Card {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let s = card_style(cx.ui_theme()).with_colors(self.colors);
        let radius = s.radius.to_pixels(window.rem_size());
        let border = s.border_width;
        let surround = s.surround;
        let has_header = self.header.is_some();
        let on_header_bounds = self.on_header_bounds.filter(|_| has_header);
        let content = div().relative().flex_1().w_full().min_w_0().min_h_0().overflow_hidden().children(self.content);
        // Last, so it paints above the content; the border is painted after
        // every child, above it.
        let cutouts = canvas(|_, _, _| {}, move |bounds, (), window, _| paint_cutouts(bounds, radius, border, surround, window))
            .absolute()
            .top(-border)
            .right(-border)
            .bottom(-border)
            .left(-border);
        let mut card = div()
            .when_some(on_header_bounds, |d, on_bounds| {
                d.on_children_prepainted(move |bounds, window, cx| {
                    if let Some(header) = bounds.first() {
                        on_bounds(*header, window, cx);
                    }
                })
            })
            .id(self.id)
            .relative()
            .flex()
            .flex_col()
            .bg(s.surface)
            .text_color(s.text)
            .border(border)
            .border_color(s.border)
            .rounded(radius)
            .when_some(self.header, |d, header| d.child(header.build(&s)))
            .child(content)
            .child(cutouts);
        // The owner's layout refinements win over the component's.
        card.style().refine(&self.style);
        card.focus_ring(self.ring, s.ring, window)
    }
}

/// One corner's piece outside a rounded border: the corner's square less a
/// quarter disc, as a closed path from the box's corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CornerCutout {
    /// The box's corner, then along the horizontal edge to where the
    /// rounding starts, then towards the disc's centre to the arc's start.
    pub corner: Point<Pixels>,
    pub along_edge: Point<Pixels>,
    pub arc_from: Point<Pixels>,
    /// The arc's end, on the line from the vertical edge's rounding start
    /// towards the disc's centre; the path then goes to that start and back
    /// to the corner.
    pub arc_to: Point<Pixels>,
    pub down_edge: Point<Pixels>,
    pub arc_radius: Pixels,
    /// The SVG sweep flag for the arc from `arc_from` to `arc_to`.
    pub sweep: bool,
}

/// The four pieces outside a box of `bounds` with corners of `radius`
/// and a border `border` wide, clockwise from the top left.
///
/// The radius is clamped to half the shorter side, as gpui clamps a
/// quad's. Each piece's arc runs through the middle of the border band
/// (radius less half the border), so the border drawn over it hides the
/// arc's anti-aliased edge and no seam shows between the two. Without a
/// radius there is nothing to cut.
pub fn corner_cutouts(bounds: Bounds<Pixels>, radius: Pixels, border: Pixels) -> Vec<CornerCutout> {
    let radius = radius.min(bounds.size.width / 2.).min(bounds.size.height / 2.);
    if radius <= px(0.) {
        return Vec::new();
    }
    let arc_radius = (radius - border / 2.).max(px(0.));
    let corners = [
        (bounds.origin, 1., 1.),
        (bounds.top_right(), -1., 1.),
        (bounds.bottom_right(), -1., -1.),
        (bounds.bottom_left(), 1., -1.),
    ];
    corners
        .into_iter()
        .map(|(corner, dx, dy): (Point<Pixels>, f32, f32)| {
            let at = |x: Pixels, y: Pixels| point(corner.x + x * dx, corner.y + y * dy);
            CornerCutout {
                corner,
                along_edge: at(radius, px(0.)),
                arc_from: at(radius, radius - arc_radius),
                arc_to: at(radius - arc_radius, radius),
                down_edge: at(px(0.), radius),
                arc_radius,
                // Towards the corner, which turns the other way on screen
                // in a mirrored corner.
                sweep: dx * dy < 0.,
            }
        })
        .collect()
}

/// Fills the [`corner_cutouts`] of `bounds` in `color`.
fn paint_cutouts(bounds: Bounds<Pixels>, radius: Pixels, border: Pixels, color: Hsla, window: &mut Window) {
    let cutouts = corner_cutouts(bounds, radius, border);
    if cutouts.is_empty() || color.is_transparent() {
        return;
    }
    let mut path = PathBuilder::fill();
    for cut in cutouts {
        path.move_to(cut.corner);
        path.line_to(cut.along_edge);
        path.line_to(cut.arc_from);
        path.arc_to(point(cut.arc_radius, cut.arc_radius), px(0.), false, cut.sweep, cut.arc_to);
        path.line_to(cut.down_edge);
        path.close();
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) }
    }

    /// Each corner's piece starts at the box's corner, runs `radius` along
    /// the horizontal edge and down the vertical one, and its arc lies in
    /// the middle of the border, curving towards the corner.
    #[test]
    fn cutouts_cover_each_corner_outside_the_rounded_edge() {
        let b = bounds(10., 20., 200., 100.);
        let cuts = corner_cutouts(b, px(10.), px(1.));
        assert_eq!(cuts.len(), 4);
        let top_left = cuts[0];
        assert_eq!(top_left.corner, point(px(10.), px(20.)));
        assert_eq!(top_left.along_edge, point(px(20.), px(20.)));
        assert_eq!(top_left.arc_from, point(px(20.), px(20.5)));
        assert_eq!(top_left.arc_to, point(px(10.5), px(30.)));
        assert_eq!(top_left.down_edge, point(px(10.), px(30.)));
        assert_eq!(top_left.arc_radius, px(9.5));
        assert!(!top_left.sweep);
        let bottom_right = cuts[2];
        assert_eq!(bottom_right.corner, point(px(210.), px(120.)));
        assert_eq!(bottom_right.along_edge, point(px(200.), px(120.)));
        assert_eq!(bottom_right.arc_from, point(px(200.), px(119.5)));
        assert_eq!(bottom_right.arc_to, point(px(209.5), px(110.)));
        assert!(!bottom_right.sweep);
        // The mirrored corners turn the other way.
        assert_eq!(cuts[1].corner, b.top_right());
        assert_eq!(cuts[3].corner, b.bottom_left());
        assert!(cuts[1].sweep && cuts[3].sweep);
        // Every point stays inside the box and inside its corner's square.
        for cut in &cuts {
            for p in [cut.corner, cut.along_edge, cut.arc_from, cut.arc_to, cut.down_edge] {
                assert!(b.contains(&p) || p.x == b.right() || p.y == b.bottom(), "{p:?} outside {b:?}");
                assert!((p.x - cut.corner.x).abs() <= px(10.) && (p.y - cut.corner.y).abs() <= px(10.), "{p:?}");
            }
            // The arc's ends lie straight above and beside the quarter
            // disc's centre, its radius away.
            let centre = point(cut.along_edge.x, cut.down_edge.y);
            for end in [cut.arc_from, cut.arc_to] {
                let (dx, dy) = ((end.x - centre.x).abs(), (end.y - centre.y).abs());
                assert_eq!(dx.max(dy), px(9.5), "{end:?} isn't on the arc round {centre:?}");
                assert_eq!(dx.min(dy), px(0.), "{end:?} isn't on the arc round {centre:?}");
            }
        }
    }

    /// A radius larger than half the shorter side is clamped, as gpui
    /// clamps a quad's; no radius cuts nothing; no border puts the arc on
    /// the edge.
    #[test]
    fn cutouts_clamp_the_radius_and_need_one() {
        let b = bounds(0., 0., 12., 40.);
        let cuts = corner_cutouts(b, px(20.), px(1.));
        assert_eq!(cuts[0].along_edge, point(px(6.), px(0.)));
        assert_eq!(cuts[0].arc_radius, px(5.5));
        assert!(corner_cutouts(b, px(0.), px(1.)).is_empty());
        assert_eq!(corner_cutouts(b, px(4.), px(0.))[0].arc_radius, px(4.));
        // A border wider than twice the radius leaves a square piece.
        assert_eq!(corner_cutouts(b, px(1.), px(4.))[0].arc_radius, px(0.));
    }
}
