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
    Send,
    Brain,
    FileText,
    Pencil,
    Trash,
    Move,
    Terminal,
    Globe,
    Wrench,
    CircleAlert,
    LoaderCircle,
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

// The icons below are lucide's paths with their arcs and curves drawn as
// chords (an arc every 15 degrees or so, a curve in eight), flattened once
// from the path data quoted above each.

// lucide `send`: M14.536 21.686a.5.5 0 0 0 .937-.024l6.5-19a.496.496 0 0
// 0-.635-.635l-19 6.5a.5.5 0 0 0-.024.937l7.93 3.18a2 2 0 0 1 1.112 1.11z,
// m21.854 2.147-10.94 10.939
const SEND: &[Stroke] = &[
    Stroke::Polyline(&[p(14.536, 21.686), p(14.594, 21.792), p(14.677, 21.881), p(14.778, 21.948), p(14.892, 21.988), p(15.013, 22.), p(15.133, 21.982), p(15.245, 21.936), p(15.343, 21.864), p(15.42, 21.771), p(15.473, 21.662), p(21.973, 2.662), p(22., 2.541), p(21.996, 2.416), p(21.962, 2.297), p(21.898, 2.19), p(21.81, 2.102), p(21.703, 2.038), p(21.584, 2.004), p(21.459, 2.), p(21.338, 2.027), p(2.338, 8.527), p(2.229, 8.58), p(2.136, 8.657), p(2.064, 8.755), p(2.018, 8.867), p(2., 8.987), p(2.012, 9.108), p(2.052, 9.222), p(2.119, 9.323), p(2.208, 9.406), p(2.314, 9.464), p(10.244, 12.644), p(10.601, 12.831), p(10.914, 13.085), p(11.169, 13.397), p(11.356, 13.754), p(14.536, 21.686)]),
    Stroke::Polyline(&[p(21.854, 2.147), p(10.914, 13.086)]),
];
// lucide `brain`: M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0
// .556 6.588A4 4 0 1 0 12 18Z, its mirror image, M15 13a4.5 4.5 0 0 1-3-4
// 4.5 4.5 0 0 1-3 4, and six short arcs (M17.599 6.5a3 3 0 0 0 .399-1.375,
// M6.003 5.125A3 3 0 0 0 6.401 6.5, M3.477 10.896a4 4 0 0 1 .585-.396,
// M19.938 10.5a4 4 0 0 1 .585.396, M6 18a4 4 0 0 1-1.967-.516,
// M19.967 17.484A4 4 0 0 1 18 18)
const BRAIN: &[Stroke] = &[
    Stroke::Polyline(&[p(12., 5.), p(11.912, 4.271), p(11.65, 3.585), p(11.229, 2.984), p(10.675, 2.502), p(10.021, 2.17), p(9.305, 2.007), p(8.571, 2.022), p(7.863, 2.215), p(7.223, 2.574), p(6.689, 3.078), p(6.293, 3.697), p(6.06, 4.393), p(6.003, 5.125), p(5.044, 5.509), p(4.216, 6.126), p(3.573, 6.935), p(3.158, 7.881), p(3., 8.902), p(3.109, 9.93), p(3.477, 10.895), p(2.804, 11.594), p(2.318, 12.434), p(2.05, 13.366), p(2.013, 14.336), p(2.212, 15.286), p(2.633, 16.16), p(3.252, 16.907), p(4.033, 17.483), p(4.03, 18.486), p(4.277, 19.459), p(4.758, 20.339), p(5.443, 21.072), p(6.289, 21.611), p(7.242, 21.923), p(8.244, 21.988), p(9.229, 21.802), p(10.138, 21.377), p(10.912, 20.738), p(11.503, 19.927), p(11.873, 18.995), p(12., 18.), p(12., 5.)]),
    Stroke::Polyline(&[p(12., 5.), p(12.088, 4.271), p(12.35, 3.585), p(12.771, 2.984), p(13.325, 2.502), p(13.979, 2.17), p(14.695, 2.007), p(15.429, 2.022), p(16.137, 2.215), p(16.777, 2.574), p(17.311, 3.078), p(17.707, 3.697), p(17.94, 4.393), p(17.997, 5.125), p(18.956, 5.509), p(19.784, 6.126), p(20.427, 6.935), p(20.842, 7.881), p(21., 8.902), p(20.891, 9.93), p(20.523, 10.895), p(21.196, 11.594), p(21.682, 12.434), p(21.95, 13.366), p(21.987, 14.336), p(21.788, 15.286), p(21.367, 16.16), p(20.748, 16.907), p(19.967, 17.483), p(19.97, 18.486), p(19.723, 19.459), p(19.242, 20.339), p(18.557, 21.072), p(17.711, 21.611), p(16.758, 21.923), p(15.756, 21.988), p(14.771, 21.802), p(13.862, 21.377), p(13.088, 20.738), p(12.497, 19.927), p(12.127, 18.995), p(12., 18.), p(12., 5.)]),
    Stroke::Polyline(&[p(15., 13.), p(14.05, 12.534), p(13.236, 11.859), p(12.601, 11.013), p(12.181, 10.042), p(12., 9.), p(11.819, 10.042), p(11.399, 11.013), p(10.764, 11.859), p(9.95, 12.534), p(9., 13.)]),
    Stroke::Polyline(&[p(17.599, 6.5), p(17.882, 5.837), p(17.998, 5.125)]),
    Stroke::Polyline(&[p(6.003, 5.125), p(6.119, 5.837), p(6.401, 6.5)]),
    Stroke::Polyline(&[p(3.477, 10.896), p(3.761, 10.685), p(4.062, 10.5)]),
    Stroke::Polyline(&[p(19.938, 10.5), p(20.239, 10.685), p(20.523, 10.896)]),
    Stroke::Polyline(&[p(6., 18.), p(4.983, 17.869), p(4.033, 17.484)]),
    Stroke::Polyline(&[p(19.967, 17.484), p(19.017, 17.869), p(18., 18.)]),
];
// lucide `file-text`: M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0
// 2-2V7Z, M14 2v4a2 2 0 0 0 2 2h4, M10 9H8, M16 13H8, M16 17H8
const FILE_TEXT: &[Stroke] = &[
    Stroke::Polyline(&[p(15., 2.), p(6., 2.), p(5.482, 2.068), p(5., 2.268), p(4.586, 2.586), p(4.268, 3.), p(4.068, 3.482), p(4., 4.), p(4., 20.), p(4.068, 20.518), p(4.268, 21.), p(4.586, 21.414), p(5., 21.732), p(5.482, 21.932), p(6., 22.), p(18., 22.), p(18.518, 21.932), p(19., 21.732), p(19.414, 21.414), p(19.732, 21.), p(19.932, 20.518), p(20., 20.), p(20., 7.), p(15., 2.)]),
    Stroke::Polyline(&[p(14., 2.), p(14., 6.), p(14.068, 6.518), p(14.268, 7.), p(14.586, 7.414), p(15., 7.732), p(15.482, 7.932), p(16., 8.), p(20., 8.)]),
    Stroke::Polyline(&[p(10., 9.), p(8., 9.)]),
    Stroke::Polyline(&[p(16., 13.), p(8., 13.)]),
    Stroke::Polyline(&[p(16., 17.), p(8., 17.)]),
];
// lucide `pencil`: M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0
// 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z,
// m15 5 4 4
const PENCIL: &[Stroke] = &[
    Stroke::Polyline(&[p(21.174, 6.812), p(21.622, 6.228), p(21.904, 5.548), p(22., 4.819), p(21.904, 4.089), p(21.622, 3.409), p(21.174, 2.826), p(20.591, 2.377), p(19.911, 2.096), p(19.181, 2.), p(18.452, 2.096), p(17.772, 2.377), p(17.188, 2.825), p(3.842, 16.174), p(3.541, 16.558), p(3.342, 17.004), p(2.021, 21.356), p(2.001, 21.474), p(2.009, 21.593), p(2.045, 21.706), p(2.107, 21.808), p(2.192, 21.893), p(2.294, 21.955), p(2.407, 21.99), p(2.526, 21.998), p(2.644, 21.978), p(6.997, 20.658), p(7.443, 20.46), p(7.827, 20.161), p(21.174, 6.812)]),
    Stroke::Polyline(&[p(15., 5.), p(19., 9.)]),
];
// lucide `trash`: M3 6h18, M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6,
// M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2
const TRASH: &[Stroke] = &[
    Stroke::Polyline(&[p(3., 6.), p(21., 6.)]),
    Stroke::Polyline(&[p(19., 6.), p(19., 20.), p(18.955, 20.373), p(18.828, 20.734), p(18.631, 21.072), p(18.375, 21.375), p(18.072, 21.631), p(17.734, 21.828), p(17.373, 21.955), p(17., 22.), p(7., 22.), p(6.627, 21.955), p(6.266, 21.828), p(5.928, 21.631), p(5.625, 21.375), p(5.369, 21.072), p(5.172, 20.734), p(5.045, 20.373), p(5., 20.), p(5., 6.)]),
    Stroke::Polyline(&[p(8., 6.), p(8., 4.), p(8.045, 3.627), p(8.172, 3.266), p(8.369, 2.928), p(8.625, 2.625), p(8.928, 2.369), p(9.266, 2.172), p(9.627, 2.045), p(10., 2.), p(14., 2.), p(14.373, 2.045), p(14.734, 2.172), p(15.072, 2.369), p(15.375, 2.625), p(15.631, 2.928), p(15.828, 3.266), p(15.955, 3.627), p(16., 4.), p(16., 6.)]),
];
// lucide `move`: M12 2v20, m15 19-3 3-3-3, m19 9 3 3-3 3, M2 12h20,
// m5 9-3 3 3 3, m9 5 3-3 3 3
const MOVE: &[Stroke] = &[
    Stroke::Polyline(&[p(12., 2.), p(12., 22.)]),
    Stroke::Polyline(&[p(15., 19.), p(12., 22.), p(9., 19.)]),
    Stroke::Polyline(&[p(19., 9.), p(22., 12.), p(19., 15.)]),
    Stroke::Polyline(&[p(2., 12.), p(22., 12.)]),
    Stroke::Polyline(&[p(5., 9.), p(2., 12.), p(5., 15.)]),
    Stroke::Polyline(&[p(9., 5.), p(12., 2.), p(15., 5.)]),
];
// lucide `terminal`: m4 17 6-6-6-6, M12 19h8
const TERMINAL: &[Stroke] = &[
    Stroke::Polyline(&[p(4., 17.), p(10., 11.), p(4., 5.)]),
    Stroke::Polyline(&[p(12., 19.), p(20., 19.)]),
];
// lucide `globe`: circle cx=12 cy=12 r=10, M12 2a14.5 14.5 0 0 0 0 20
// 14.5 14.5 0 0 0 0-20, M2 12h20
const GLOBE: &[Stroke] = &[
    Stroke::Circle { center: p(12., 12.), radius: 10. / GRID },
    Stroke::Polyline(&[p(12., 2.), p(9.826, 4.955), p(8.464, 8.361), p(8., 12.), p(8.464, 15.639), p(9.826, 19.045), p(12., 22.), p(14.174, 19.045), p(15.536, 15.639), p(16., 12.), p(15.536, 8.361), p(14.174, 4.955), p(12., 2.)]),
    Stroke::Polyline(&[p(2., 12.), p(22., 12.)]),
];
// lucide `wrench`: M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0
// l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91
// a6 6 0 0 1 7.94-7.94l-3.76 3.76z
const WRENCH: &[Stroke] = &[
    Stroke::Polyline(&[p(14.7, 6.3), p(14.545, 6.506), p(14.447, 6.744), p(14.414, 7.), p(14.447, 7.256), p(14.545, 7.494), p(14.7, 7.7), p(16.3, 9.3), p(16.506, 9.455), p(16.744, 9.553), p(17., 9.586), p(17.256, 9.553), p(17.494, 9.455), p(17.7, 9.3), p(21.47, 5.53), p(21.904, 6.912), p(21.993, 8.359), p(21.733, 9.784), p(21.14, 11.106), p(20.246, 12.246), p(19.106, 13.14), p(17.784, 13.733), p(16.359, 13.993), p(14.912, 13.904), p(13.53, 13.47), p(6.62, 20.38), p(6.181, 20.717), p(5.669, 20.929), p(5.12, 21.001), p(4.571, 20.929), p(4.059, 20.717), p(3.62, 20.38), p(3.283, 19.941), p(3.071, 19.429), p(2.999, 18.88), p(3.071, 18.331), p(3.283, 17.819), p(3.62, 17.38), p(10.53, 10.47), p(10.096, 9.088), p(10.007, 7.641), p(10.267, 6.216), p(10.86, 4.894), p(11.754, 3.754), p(12.894, 2.86), p(14.216, 2.267), p(15.641, 2.007), p(17.088, 2.096), p(18.47, 2.53), p(14.7, 6.3)]),
];
// lucide `circle-alert`: circle cx=12 cy=12 r=10, M12 8v4, M12 16h.01 (the
// dot a 2-unit rule centred on (12, 16), as in `triangle-alert`)
const CIRCLE_ALERT: &[Stroke] = &[
    Stroke::Circle { center: p(12., 12.), radius: 10. / GRID },
    Stroke::Polyline(&[p(12., 8.), p(12., 12.)]),
    Stroke::Polyline(&[p(12., 15.), p(12., 17.)]),
];
// lucide `loader-circle`: M21 12a9 9 0 1 1-6.219-8.56, the spinner
// (`IconElement::turn` rotates it)
const LOADER_CIRCLE: &[Stroke] = &[Stroke::Polyline(&[
    p(21., 12.),
    p(20.717, 14.238),
    p(19.887, 16.336),
    p(18.56, 18.161),
    p(16.822, 19.599),
    p(14.781, 20.559),
    p(12.565, 20.982),
    p(10.313, 20.84),
    p(8.168, 20.143),
    p(6.263, 18.934),
    p(4.719, 17.29),
    p(3.632, 15.313),
    p(3.071, 13.127),
    p(3.071, 10.872),
    p(3.632, 8.686),
    p(4.719, 6.709),
    p(6.263, 5.065),
    p(8.168, 3.856),
    p(10.313, 3.159),
    p(12.565, 3.017),
    p(14.781, 3.44),
])];

impl Icon {
    pub const ALL: [Icon; 29] = [
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
        Icon::Send,
        Icon::Brain,
        Icon::FileText,
        Icon::Pencil,
        Icon::Trash,
        Icon::Move,
        Icon::Terminal,
        Icon::Globe,
        Icon::Wrench,
        Icon::CircleAlert,
        Icon::LoaderCircle,
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
            Icon::Send => SEND,
            Icon::Brain => BRAIN,
            Icon::FileText => FILE_TEXT,
            Icon::Pencil => PENCIL,
            Icon::Trash => TRASH,
            Icon::Move => MOVE,
            Icon::Terminal => TERMINAL,
            Icon::Globe => GLOBE,
            Icon::Wrench => WRENCH,
            Icon::CircleAlert => CIRCLE_ALERT,
            Icon::LoaderCircle => LOADER_CIRCLE,
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
        self.turned_path(origin, side, min_stroke, 0.)
    }

    /// [`Icon::path`] turned clockwise about the square's centre by
    /// `turns` (1 is a whole turn): a spinner's frame.
    pub fn turned_path(
        self,
        origin: Point<Pixels>,
        side: Pixels,
        min_stroke: Option<Pixels>,
        turns: f32,
    ) -> Result<Path<Pixels>, PathError> {
        let at = |unit: (f32, f32)| -> Point<Pixels> {
            let (x, y) = turn_point(unit, turns);
            origin + point(side * x, side * y)
        };

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
    fn paint(self, bounds: Bounds<Pixels>, color: Hsla, min_stroke: Option<Pixels>, turns: f32, window: &mut Window) {
        let side = bounds.size.width.min(bounds.size.height);
        let origin = bounds.origin
            + point(
                (bounds.size.width - side) / 2.,
                (bounds.size.height - side) / 2.,
            );
        match self.turned_path(origin, side, min_stroke, turns) {
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

/// A point in unit coordinates turned clockwise (y down) about the unit
/// square's centre by `turns`.
fn turn_point((x, y): (f32, f32), turns: f32) -> (f32, f32) {
    if turns == 0. {
        return (x, y);
    }
    let (sin, cos) = (turns * std::f32::consts::TAU).sin_cos();
    let (dx, dy) = (x - 0.5, y - 0.5);
    (0.5 + dx * cos - dy * sin, 0.5 + dx * sin + dy * cos)
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
    turns: f32,
}

impl IconElement {
    pub fn new(icon: Icon) -> Self {
        Self {
            icon,
            size: rems(1.),
            color: None,
            min_stroke: None,
            turns: 0.,
        }
    }

    /// Drawn turned clockwise by `turns` (1 is a whole turn), as a spinner
    /// animates its icon.
    pub fn turn(mut self, turns: f32) -> Self {
        self.turns = turns;
        self
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
        let Self { icon, size, color, min_stroke, turns } = self;
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                // Read at paint time, inside the parents' text styles.
                let color = color.unwrap_or_else(|| window.text_style().color);
                icon.paint(bounds, color, min_stroke, turns, window);
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

    /// A spinner's frames: a quarter turn takes the right edge's middle to
    /// the bottom's, a half turn to the left's, and every icon builds
    /// turned.
    #[test]
    fn icons_turn_about_their_centre() {
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
        assert_eq!(turn_point((0.25, 0.1), 0.), (0.25, 0.1));
        assert!(near(turn_point((1., 0.5), 0.25), (0.5, 1.)));
        assert!(near(turn_point((1., 0.5), 0.5), (0., 0.5)));
        assert!(near(turn_point((0.5, 0.5), 0.3), (0.5, 0.5)));
        for icon in Icon::ALL {
            for turns in [0.125, 0.5, 0.9] {
                let built = icon.turned_path(point(px(3.), px(4.)), px(16.), None, turns);
                assert!(built.is_ok(), "{icon:?} turned {turns}: {built:?}");
            }
        }
        assert_eq!(IconElement::new(Icon::LoaderCircle).turn(0.5).turns, 0.5);
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
