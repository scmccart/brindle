//! A few stroked icons, drawn with `PathBuilder` instead of SVG assets.
//!
//! The shapes are lucide's (shadcn's icon set): strokes on a 24-unit grid,
//! 2 units wide, kept here in unit coordinates (the grid divided by 24). An
//! icon paints in the current text colour unless given one, and is sized in
//! rems (1rem by default, shadcn's `size-4`).
//!
//! `svg()` would need the app's single `AssetSource`, and text glyphs depend
//! on the font, so the crate draws these itself.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use gpui::{
    App, Bounds, Hsla, IntoElement, Path, PathBuilder, Pixels, Point, Rems, RenderOnce, Styled,
    Window, canvas, point, px, rems,
};

/// An icon's stroke couldn't be turned into a path (gpui's tessellator
/// refused it), with the tessellator's account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathError(String);

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathError {}

/// The icons whose failure has been logged, so a paint that fails every
/// frame says so once.
static REPORTED: Mutex<Option<HashSet<Icon>>> = Mutex::new(None);

/// Whether `icon` is new to `reported`, recording it: the decision to log
/// a failed icon, once per icon.
fn first_report(reported: &mut HashSet<Icon>, icon: Icon) -> bool {
    reported.insert(icon)
}

/// The icons the components use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoElement)]
pub enum Icon {
    Check,
    ChevronDown,
    ChevronUp,
    X,
    Search,
    Plus,
    Minus,
    Square,
    Copy,
    AppWindow,
    ArrowUp,
    ArrowDown,
    Columns2,
    Rows2,
    Lock,
    TriangleAlert,
    ClipboardPaste,
    Ban,
}

/// One stroke of an icon, in unit coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Stroke {
    /// Straight segments through the points.
    Polyline(&'static [(f32, f32)]),
    Circle { center: (f32, f32), radius: f32 },
    /// A rectangle with its top-left corner at `(x, y)` and rounded corners
    /// of `radius` (zero for square ones).
    Rect { x: f32, y: f32, width: f32, height: f32, radius: f32 },
}

/// The size of lucide's grid.
const GRID: f32 = 24.;

/// Lucide's stroke width, in unit coordinates.
const STROKE_WIDTH: f32 = 2. / GRID;

/// The thinnest stroke a window-chrome icon (a tab strip's buttons, a tab's
/// close button, the caption buttons) reads at: two pixels. Lucide's stroke
/// scales with the icon, so at 16px it is 1.33px, too thin to see there;
/// [`IconElement::min_stroke`] sets this instead.
pub const LEGIBLE_ICON_STROKE: Pixels = px(2.);

/// The stroke width of an icon drawn `side` pixels square: lucide's 2 on
/// its 24 grid, scaled with the icon, and at least `min` pixels when given.
pub fn stroke_width(side: Pixels, min: Option<Pixels>) -> Pixels {
    let lucide = side * STROKE_WIDTH;
    min.map_or(lucide, |min| lucide.max(min))
}

/// A point on lucide's grid, in unit coordinates.
const fn p(x: f32, y: f32) -> (f32, f32) {
    (x / GRID, y / GRID)
}

// lucide `check`: M20 6 9 17l-5-5
const CHECK: &[Stroke] = &[Stroke::Polyline(&[p(20., 6.), p(9., 17.), p(4., 12.)])];
// lucide `chevron-down`: m6 9 6 6 6-6
const CHEVRON_DOWN: &[Stroke] = &[Stroke::Polyline(&[p(6., 9.), p(12., 15.), p(18., 9.)])];
// lucide `chevron-up`: m18 15-6-6-6 6
const CHEVRON_UP: &[Stroke] = &[Stroke::Polyline(&[p(18., 15.), p(12., 9.), p(6., 15.)])];
// lucide `x`: M18 6 6 18, m6 6 12 12
const X: &[Stroke] = &[
    Stroke::Polyline(&[p(18., 6.), p(6., 18.)]),
    Stroke::Polyline(&[p(6., 6.), p(18., 18.)]),
];
// lucide `search`: circle cx=11 cy=11 r=8, m21 21-4.34-4.34
const SEARCH: &[Stroke] = &[
    Stroke::Circle {
        center: p(11., 11.),
        radius: 8. / GRID,
    },
    Stroke::Polyline(&[p(21., 21.), p(16.66, 16.66)]),
];

/// A rectangle on lucide's grid, in unit coordinates.
const fn rect(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Stroke {
    Stroke::Rect { x: x / GRID, y: y / GRID, width: width / GRID, height: height / GRID, radius: radius / GRID }
}

// lucide `plus`: M5 12h14, M12 5v14
const PLUS: &[Stroke] = &[
    Stroke::Polyline(&[p(5., 12.), p(19., 12.)]),
    Stroke::Polyline(&[p(12., 5.), p(12., 19.)]),
];
// lucide `minus`: M5 12h14
const MINUS: &[Stroke] = &[Stroke::Polyline(&[p(5., 12.), p(19., 12.)])];
// lucide `square`: rect width=18 height=18 x=3 y=3 rx=2
const SQUARE: &[Stroke] = &[rect(3., 3., 18., 18., 2.)];
// lucide `copy`: rect width=14 height=14 x=8 y=8 rx=2 ry=2, and
// M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2, whose corners
// are drawn as their quarter-circle midpoints.
const COPY: &[Stroke] = &[
    rect(8., 8., 14., 14., 2.),
    Stroke::Polyline(&[
        p(4., 16.),
        p(2.586, 15.414),
        p(2., 14.),
        p(2., 4.),
        p(2.586, 2.586),
        p(4., 2.),
        p(14., 2.),
        p(15.414, 2.586),
        p(16., 4.),
    ]),
];
// lucide `app-window`: rect x=2 y=4 width=20 height=16 rx=2, M10 4v4, M2 8h20, M6 4v4
const APP_WINDOW: &[Stroke] = &[
    rect(2., 4., 20., 16., 2.),
    Stroke::Polyline(&[p(10., 4.), p(10., 8.)]),
    Stroke::Polyline(&[p(2., 8.), p(22., 8.)]),
    Stroke::Polyline(&[p(6., 4.), p(6., 8.)]),
];
// lucide `arrow-up`: m5 12 7-7 7 7, M12 19V5
const ARROW_UP: &[Stroke] = &[
    Stroke::Polyline(&[p(5., 12.), p(12., 5.), p(19., 12.)]),
    Stroke::Polyline(&[p(12., 19.), p(12., 5.)]),
];
// lucide `arrow-down`: M12 5v14, m19 12-7 7-7-7
const ARROW_DOWN: &[Stroke] = &[
    Stroke::Polyline(&[p(12., 5.), p(12., 19.)]),
    Stroke::Polyline(&[p(19., 12.), p(12., 19.), p(5., 12.)]),
];
// lucide `columns-2`: rect width=18 height=18 x=3 y=3 rx=2, M12 3v18
const COLUMNS_2: &[Stroke] = &[rect(3., 3., 18., 18., 2.), Stroke::Polyline(&[p(12., 3.), p(12., 21.)])];
// lucide `rows-2`: rect width=18 height=18 x=3 y=3 rx=2, M3 12h18
const ROWS_2: &[Stroke] = &[rect(3., 3., 18., 18., 2.), Stroke::Polyline(&[p(3., 12.), p(21., 12.)])];
// lucide `lock`: rect width=18 height=11 x=3 y=11 rx=2, M7 11V7a5 5 0 0 1 10 0v4,
// the shackle's arc (radius 5 about (12, 7)) drawn as chords every 22.5 degrees.
const LOCK: &[Stroke] = &[
    rect(3., 11., 18., 11., 2.),
    Stroke::Polyline(&[
        p(7., 11.),
        p(7., 7.),
        p(7.38, 5.09),
        p(8.46, 3.46),
        p(10.09, 2.38),
        p(12., 2.),
        p(13.91, 2.38),
        p(15.54, 3.46),
        p(16.62, 5.09),
        p(17., 7.),
        p(17., 11.),
    ]),
];
// lucide `triangle-alert`: m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16
// a2 2 0 0 0 1.73-3, M12 9v4, M12 17h.01. The triangle starts and ends
// halfway along its bottom edge, so its butt-capped ends meet on a straight
// line; its corners (radius 2) are drawn as chords every 30 degrees or so.
// Without round caps the `h.01` dot would be invisible, so it is a 2-unit
// rule centred on (12, 17).
const TRIANGLE_ALERT: &[Stroke] = &[
    Stroke::Polyline(&[
        p(12., 21.),
        p(20., 21.),
        p(21.414, 20.414),
        p(22., 19.),
        p(21.73, 18.),
        p(13.73, 4.),
        p(12.99, 3.254),
        p(11.99, 2.986),
        p(10.99, 3.254),
        p(10.25, 4.),
        p(2.25, 18.),
        p(2., 19.),
        p(2.586, 20.414),
        p(4., 21.),
        p(12., 21.),
    ]),
    Stroke::Polyline(&[p(12., 9.), p(12., 13.)]),
    Stroke::Polyline(&[p(12., 16.), p(12., 18.)]),
];

// lucide `clipboard-paste`: M11 14h10, M16 4h2a2 2 0 0 1 2 2v1.344,
// m17 18 4-4-4-4, M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0
// 1.793-1.113, rect x=8 y=2 width=8 height=4 rx=1. The board's corners
// (radius 2) are drawn through their quarter-circle midpoints, and its last
// arc, which stops 26.3 degrees short of the corner's end, through its own.
const CLIPBOARD_PASTE: &[Stroke] = &[
    Stroke::Polyline(&[p(11., 14.), p(21., 14.)]),
    Stroke::Polyline(&[p(16., 4.), p(18., 4.), p(19.414, 4.586), p(20., 6.), p(20., 7.344)]),
    Stroke::Polyline(&[p(17., 18.), p(21., 14.), p(17., 10.)]),
    Stroke::Polyline(&[
        p(8., 4.),
        p(6., 4.),
        p(4.586, 4.586),
        p(4., 6.),
        p(4., 20.),
        p(4.586, 21.414),
        p(6., 22.),
        p(18., 22.),
        p(19.054, 21.7),
        p(19.793, 20.887),
    ]),
    rect(8., 2., 8., 4., 1.),
];
// lucide `ban`: circle cx=12 cy=12 r=10, M4.929 4.929 19.07 19.071
const BAN: &[Stroke] = &[
    Stroke::Circle {
        center: p(12., 12.),
        radius: 10. / GRID,
    },
    Stroke::Polyline(&[p(4.929, 4.929), p(19.07, 19.071)]),
];

impl Icon {
    pub const ALL: [Icon; 18] = [
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::X,
        Icon::Search,
        Icon::Plus,
        Icon::Minus,
        Icon::Square,
        Icon::Copy,
        Icon::AppWindow,
        Icon::ArrowUp,
        Icon::ArrowDown,
        Icon::Columns2,
        Icon::Rows2,
        Icon::Lock,
        Icon::TriangleAlert,
        Icon::ClipboardPaste,
        Icon::Ban,
    ];

    pub(crate) fn strokes(self) -> &'static [Stroke] {
        match self {
            Icon::Check => CHECK,
            Icon::ChevronDown => CHEVRON_DOWN,
            Icon::ChevronUp => CHEVRON_UP,
            Icon::X => X,
            Icon::Search => SEARCH,
            Icon::Plus => PLUS,
            Icon::Minus => MINUS,
            Icon::Square => SQUARE,
            Icon::Copy => COPY,
            Icon::AppWindow => APP_WINDOW,
            Icon::ArrowUp => ARROW_UP,
            Icon::ArrowDown => ARROW_DOWN,
            Icon::Columns2 => COLUMNS_2,
            Icon::Rows2 => ROWS_2,
            Icon::Lock => LOCK,
            Icon::TriangleAlert => TRIANGLE_ALERT,
            Icon::ClipboardPaste => CLIPBOARD_PASTE,
            Icon::Ban => BAN,
        }
    }

    /// This icon at `size`.
    pub fn size(self, size: Rems) -> IconElement {
        IconElement::new(self).size(size)
    }

    /// This icon in `color` instead of the text colour.
    pub fn color(self, color: Hsla) -> IconElement {
        IconElement::new(self).color(color)
    }

    /// The icon's strokes as one path, `side` pixels square with its
    /// top-left corner at `origin`, at least `min_stroke` wide when given
    /// ([`stroke_width`]). Pure: painting it is [`Icon::paint`]'s.
    pub fn path(
        self,
        origin: Point<Pixels>,
        side: Pixels,
        min_stroke: Option<Pixels>,
    ) -> Result<Path<Pixels>, PathError> {
        let at = |(x, y): (f32, f32)| -> Point<Pixels> { origin + point(side * x, side * y) };

        // Butt caps and miter joins: gpui doesn't re-export lyon's `LineCap`,
        // so lucide's round caps aren't available.
        let mut path = PathBuilder::stroke(stroke_width(side, min_stroke));
        for stroke in self.strokes() {
            match *stroke {
                Stroke::Polyline(points) => {
                    let Some((&first, rest)) = points.split_first() else {
                        continue;
                    };
                    path.move_to(at(first));
                    for &pt in rest {
                        path.line_to(at(pt));
                    }
                }
                Stroke::Circle { center, radius } => {
                    let (cx, cy) = center;
                    let r = side * radius;
                    let radii = point(r, r);
                    path.move_to(at((cx + radius, cy)));
                    path.arc_to(radii, px(0.), false, true, at((cx - radius, cy)));
                    path.arc_to(radii, px(0.), false, true, at((cx + radius, cy)));
                    path.close();
                }
                Stroke::Rect { x, y, width, height, radius } => {
                    let r = side * radius;
                    let radii = point(r, r);
                    let (right, bottom) = (x + width, y + height);
                    // Clockwise from the top edge, a quarter arc per corner.
                    path.move_to(at((x + radius, y)));
                    path.line_to(at((right - radius, y)));
                    path.arc_to(radii, px(0.), false, true, at((right, y + radius)));
                    path.line_to(at((right, bottom - radius)));
                    path.arc_to(radii, px(0.), false, true, at((right - radius, bottom)));
                    path.line_to(at((x + radius, bottom)));
                    path.arc_to(radii, px(0.), false, true, at((x, bottom - radius)));
                    path.line_to(at((x, y + radius)));
                    path.arc_to(radii, px(0.), false, true, at((x + radius, y)));
                    path.close();
                }
            }
        }
        path.build().map_err(|error| PathError(format!("{error:#}")))
    }

    /// Strokes the icon into the largest square centred in `bounds`
    /// ([`Icon::path`]). A path that can't be built paints nothing, and is
    /// logged once for this icon.
    fn paint(self, bounds: Bounds<Pixels>, color: Hsla, min_stroke: Option<Pixels>, window: &mut Window) {
        let side = bounds.size.width.min(bounds.size.height);
        let origin = bounds.origin
            + point(
                (bounds.size.width - side) / 2.,
                (bounds.size.height - side) / 2.,
            );
        match self.path(origin, side, min_stroke) {
            Ok(path) => window.paint_path(path, color),
            Err(error) => {
                let first = {
                    let mut reported = REPORTED.lock().unwrap_or_else(PoisonError::into_inner);
                    first_report(reported.get_or_insert_with(HashSet::new), self)
                };
                if first {
                    log::warn!("the {self:?} icon can't be drawn: {error}");
                }
            }
        }
    }
}

impl RenderOnce for Icon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        IconElement::new(self)
    }
}

/// An [`Icon`] with a size and, optionally, its own colour and a minimum
/// stroke width.
#[derive(IntoElement)]
pub struct IconElement {
    icon: Icon,
    size: Rems,
    color: Option<Hsla>,
    min_stroke: Option<Pixels>,
}

impl IconElement {
    pub fn new(icon: Icon) -> Self {
        Self {
            icon,
            size: rems(1.),
            color: None,
            min_stroke: None,
        }
    }

    pub fn size(mut self, size: Rems) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Strokes at least `width` pixels wide, whatever the icon's size
    /// (lucide's own stroke scales down with it): [`LEGIBLE_ICON_STROKE`]
    /// for window chrome.
    pub fn min_stroke(mut self, width: Pixels) -> Self {
        self.min_stroke = Some(width);
        self
    }
}

impl RenderOnce for IconElement {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self { icon, size, color, min_stroke } = self;
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                // Read at paint time, inside the parents' text styles.
                let color = color.unwrap_or_else(|| window.text_style().color);
                icon.paint(bounds, color, min_stroke, window);
            },
        )
        .size(size)
        .flex_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The extent of a stroke `width` wide (in unit coordinates), widened
    /// by half of it: (min x, min y, max x, max y).
    fn extent(stroke: &Stroke, width: f32) -> (f32, f32, f32, f32) {
        let half = width / 2.;
        match *stroke {
            Stroke::Polyline(points) => points.iter().fold(
                (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
                |(x0, y0, x1, y1), &(x, y)| {
                    (x0.min(x - half), y0.min(y - half), x1.max(x + half), y1.max(y + half))
                },
            ),
            Stroke::Circle {
                center: (x, y),
                radius,
            } => {
                let r = radius + half;
                (x - r, y - r, x + r, y + r)
            }
            Stroke::Rect { x, y, width, height, .. } => (x - half, y - half, x + width + half, y + height + half),
        }
    }

    /// The spec's "Strip icons are legible": at 16px lucide's stroke is
    /// 1.33px, and a chrome icon's minimum makes it two; a bigger icon whose
    /// own stroke is wider keeps it, and other icons keep lucide's.
    #[test]
    fn chrome_icons_stroke_two_pixels_at_16px() {
        let near = |width: Pixels, want: f32| (f32::from(width) - want).abs() < 1e-4;
        assert!(near(stroke_width(px(16.), None), 16. / 12.));
        assert!(near(stroke_width(px(16.), Some(LEGIBLE_ICON_STROKE)), 2.));
        assert!(f32::from(LEGIBLE_ICON_STROKE) >= 2.);
        assert!(near(stroke_width(px(48.), Some(LEGIBLE_ICON_STROKE)), 4.));
        assert!(near(stroke_width(px(12.), None), 1.));
        let icon = Icon::Plus.size(rems(1.)).min_stroke(LEGIBLE_ICON_STROKE);
        assert_eq!((icon.size, icon.min_stroke), (rems(1.), Some(px(2.))));
        assert_eq!(IconElement::new(Icon::Plus).min_stroke, None);
    }

    /// Every icon builds at the sizes the components use (12 to 32px
    /// squares, lucide's stroke and a chrome icon's two pixels), anywhere
    /// in the window.
    #[test]
    fn every_icon_builds_at_component_sizes() {
        for icon in Icon::ALL {
            for side in [12., 14., 16., 20., 24., 32.] {
                for min_stroke in [None, Some(LEGIBLE_ICON_STROKE)] {
                    for origin in [point(px(0.), px(0.)), point(px(100.5), px(37.25))] {
                        let built = icon.path(origin, px(side), min_stroke);
                        assert!(built.is_ok(), "{icon:?} at {side}px, {min_stroke:?}: {built:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_failed_icon_is_reported_once() {
        let mut reported = HashSet::new();
        assert!(first_report(&mut reported, Icon::Check));
        assert!(!first_report(&mut reported, Icon::Check));
        assert!(first_report(&mut reported, Icon::X));
    }

    #[test]
    fn every_icon_stays_inside_its_unit_box() {
        // Lucide's stroke, and a chrome icon's two pixels at 16px.
        let widths = [STROKE_WIDTH, f32::from(LEGIBLE_ICON_STROKE) / 16.];
        for icon in Icon::ALL {
            let strokes = icon.strokes();
            assert!(!strokes.is_empty(), "{icon:?} has no strokes");
            for stroke in strokes {
                if let Stroke::Polyline(points) = stroke {
                    assert!(points.len() >= 2, "{icon:?}: a polyline needs two points");
                }
                if let Stroke::Circle { radius, .. } = stroke {
                    assert!(*radius > 0., "{icon:?}: empty circle");
                }
                if let Stroke::Rect { width, height, radius, .. } = stroke {
                    assert!(*width > 0. && *height > 0., "{icon:?}: empty rectangle");
                    assert!(*radius >= 0. && *radius * 2. <= width.min(*height), "{icon:?}: corners overlap");
                }
                for width in widths {
                    let (x0, y0, x1, y1) = extent(stroke, width);
                    assert!(
                        x0 >= 0. && y0 >= 0. && x1 <= 1. && y1 <= 1.,
                        "{icon:?}: {stroke:?} {width} wide reaches ({x0}, {y0})..({x1}, {y1})"
                    );
                }
            }
        }
    }

    #[test]
    fn shapes_are_lucides() {
        // Spot checks against lucide's path data, back on its grid.
        let grid = |(x, y): (f32, f32)| ((x * GRID).round(), (y * GRID).round());
        let Stroke::Polyline(check) = Icon::Check.strokes()[0] else {
            panic!("check is a polyline");
        };
        let check: Vec<_> = check.iter().copied().map(grid).collect();
        assert_eq!(check, [(20., 6.), (9., 17.), (4., 12.)]);
        let Stroke::Circle { center, radius } = Icon::Search.strokes()[0] else {
            panic!("search starts with its circle");
        };
        assert_eq!((grid(center), (radius * GRID).round()), ((11., 11.), 8.));
        assert_eq!(Icon::X.strokes().len(), 2);
        // The split icons are lucide's 18-unit square with one rule through it.
        for (icon, rule) in [(Icon::Columns2, [(12., 3.), (12., 21.)]), (Icon::Rows2, [(3., 12.), (21., 12.)])] {
            let Stroke::Rect { x, y, width, height, radius } = icon.strokes()[0] else {
                panic!("{icon:?} starts with its rectangle");
            };
            assert_eq!([x, y, width, height, radius].map(|v| (v * GRID).round()), [3., 3., 18., 18., 2.]);
            let Stroke::Polyline(line) = icon.strokes()[1] else {
                panic!("{icon:?} ends with its rule");
            };
            assert_eq!(line.iter().copied().map(grid).collect::<Vec<_>>(), rule);
        }
        // The lock is an 18 by 11 body and a shackle over it, whose arc is
        // radius 5 about (12, 7).
        let Stroke::Rect { x, y, width, height, radius } = Icon::Lock.strokes()[0] else {
            panic!("lock starts with its body");
        };
        assert_eq!([x, y, width, height, radius].map(|v| (v * GRID).round()), [3., 11., 18., 11., 2.]);
        let Stroke::Polyline(shackle) = Icon::Lock.strokes()[1] else {
            panic!("lock ends with its shackle");
        };
        let shackle: Vec<_> = shackle.iter().copied().map(|(x, y)| (x * GRID, y * GRID)).collect();
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
        assert!(near(shackle[0], (7., 11.)) && near(shackle[1], (7., 7.)));
        assert!(shackle.iter().any(|&pt| near(pt, (12., 2.))));
        assert!(near(shackle[shackle.len() - 2], (17., 7.)) && near(shackle[shackle.len() - 1], (17., 11.)));
        for &(x, y) in &shackle[1..shackle.len() - 1] {
            let r = ((x - 12.).powi(2) + (y - 7.).powi(2)).sqrt();
            assert!((r - 5.).abs() < 0.01 || (x - 7.).abs() < 0.01, "({x}, {y}) is off the arc: {r}");
        }
        // The warning triangle is closed, its edges lucide's, with the
        // exclamation's bar and dot on its centre line.
        let strokes = Icon::TriangleAlert.strokes();
        assert_eq!(strokes.len(), 3);
        let Stroke::Polyline(triangle) = strokes[0] else {
            panic!("triangle-alert starts with its triangle");
        };
        let triangle: Vec<_> = triangle.iter().copied().map(|(x, y)| (x * GRID, y * GRID)).collect();
        assert_eq!(triangle.first(), triangle.last(), "the triangle is closed");
        for corner in [(21.73, 18.), (13.73, 4.), (10.25, 4.), (2.25, 18.), (4., 21.), (20., 21.)] {
            assert!(triangle.iter().any(|&pt| near(pt, corner)), "{corner:?} is missing");
        }
        for (stroke, (top, bottom)) in strokes[1..].iter().zip([(9., 13.), (16., 18.)]) {
            let Stroke::Polyline(line) = stroke else {
                panic!("the exclamation is two rules");
            };
            assert_eq!(line.iter().copied().map(grid).collect::<Vec<_>>(), [(12., top), (12., bottom)]);
        }
        // The paste icon: the clipboard's 8 by 4 clip at the top, its board
        // open on the right where an arrow comes out of it along y = 14.
        let strokes = Icon::ClipboardPaste.strokes();
        assert_eq!(strokes.len(), 5);
        let Stroke::Rect { x, y, width, height, radius } = strokes[4] else {
            panic!("clipboard-paste ends with its clip");
        };
        assert_eq!([x, y, width, height, radius].map(|v| (v * GRID).round()), [8., 2., 8., 4., 1.]);
        let Stroke::Polyline(shaft) = strokes[0] else {
            panic!("clipboard-paste starts with the arrow's shaft");
        };
        assert_eq!(shaft.iter().copied().map(grid).collect::<Vec<_>>(), [(11., 14.), (21., 14.)]);
        let Stroke::Polyline(head) = strokes[2] else {
            panic!("the arrow's head is a polyline");
        };
        assert_eq!(head.iter().copied().map(grid).collect::<Vec<_>>(), [(17., 18.), (21., 14.), (17., 10.)]);
        let Stroke::Polyline(board) = strokes[3] else {
            panic!("the board is a polyline");
        };
        let board: Vec<_> = board.iter().copied().map(|(x, y)| (x * GRID, y * GRID)).collect();
        assert!(near(board[0], (8., 4.)) && near(board[board.len() - 1], (19.793, 20.887)));
        // Its last arc stays on the corner's circle, radius 2 about (18, 20).
        for &(x, y) in &board[board.len() - 2..] {
            let r = ((x - 18.).powi(2) + (y - 20.).powi(2)).sqrt();
            assert!((r - 2.).abs() < 0.01, "({x}, {y}) is off the corner: {r}");
        }
        // The ban sign: a radius 10 circle about the centre, crossed from
        // top left to bottom right.
        let Stroke::Circle { center, radius } = Icon::Ban.strokes()[0] else {
            panic!("ban starts with its circle");
        };
        assert_eq!((grid(center), (radius * GRID).round()), ((12., 12.), 10.));
        let Stroke::Polyline(bar) = Icon::Ban.strokes()[1] else {
            panic!("ban ends with its bar");
        };
        assert_eq!(bar.iter().copied().map(grid).collect::<Vec<_>>(), [(5., 5.), (19., 19.)]);
    }

    #[test]
    fn every_icon_is_listed_once() {
        let mut seen = Icon::ALL.to_vec();
        seen.sort_by_key(|icon| format!("{icon:?}"));
        seen.dedup();
        assert_eq!(seen.len(), Icon::ALL.len());
    }
}
