//! A few stroked icons, drawn with `PathBuilder` instead of SVG assets.
//!
//! The shapes are lucide's (shadcn's icon set): strokes on a 24-unit grid,
//! 2 units wide, kept here in unit coordinates (the grid divided by 24). An
//! icon paints in the current text colour unless given one, and is sized in
//! rems (1rem by default, shadcn's `size-4`).
//!
//! `svg()` would need the app's single `AssetSource`, and text glyphs depend
//! on the font, so the crate draws these itself.

use gpui::{
    App, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, Rems, RenderOnce, Styled, Window,
    canvas, point, px, rems,
};

/// The icons the components use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoElement)]
pub enum Icon {
    Check,
    ChevronDown,
    ChevronUp,
    X,
    Search,
}

/// One stroke of an icon, in unit coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Stroke {
    /// Straight segments through the points.
    Polyline(&'static [(f32, f32)]),
    Circle { center: (f32, f32), radius: f32 },
}

/// The size of lucide's grid.
const GRID: f32 = 24.;

/// Lucide's stroke width, in unit coordinates.
const STROKE_WIDTH: f32 = 2. / GRID;

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

impl Icon {
    pub const ALL: [Icon; 5] = [
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::X,
        Icon::Search,
    ];

    pub(crate) fn strokes(self) -> &'static [Stroke] {
        match self {
            Icon::Check => CHECK,
            Icon::ChevronDown => CHEVRON_DOWN,
            Icon::ChevronUp => CHEVRON_UP,
            Icon::X => X,
            Icon::Search => SEARCH,
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

    /// Strokes the icon into the largest square centred in `bounds`.
    fn paint(self, bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        let side = bounds.size.width.min(bounds.size.height);
        let origin = bounds.origin
            + point(
                (bounds.size.width - side) / 2.,
                (bounds.size.height - side) / 2.,
            );
        let at = |(x, y): (f32, f32)| -> Point<Pixels> { origin + point(side * x, side * y) };

        // Butt caps and miter joins: gpui doesn't re-export lyon's `LineCap`,
        // so lucide's round caps aren't available.
        let mut path = PathBuilder::stroke(side * STROKE_WIDTH);
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
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
    }
}

impl RenderOnce for Icon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        IconElement::new(self)
    }
}

/// An [`Icon`] with a size and, optionally, its own colour.
#[derive(IntoElement)]
pub struct IconElement {
    icon: Icon,
    size: Rems,
    color: Option<Hsla>,
}

impl IconElement {
    pub fn new(icon: Icon) -> Self {
        Self {
            icon,
            size: rems(1.),
            color: None,
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
}

impl RenderOnce for IconElement {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self { icon, size, color } = self;
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                // Read at paint time, inside the parents' text styles.
                let color = color.unwrap_or_else(|| window.text_style().color);
                icon.paint(bounds, color, window);
            },
        )
        .size(size)
        .flex_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The extent of a stroke, widened by half the stroke width:
    /// (min x, min y, max x, max y).
    fn extent(stroke: &Stroke) -> (f32, f32, f32, f32) {
        let half = STROKE_WIDTH / 2.;
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
        }
    }

    #[test]
    fn every_icon_stays_inside_its_unit_box() {
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
                let (x0, y0, x1, y1) = extent(stroke);
                assert!(
                    x0 >= 0. && y0 >= 0. && x1 <= 1. && y1 <= 1.,
                    "{icon:?}: {stroke:?} reaches ({x0}, {y0})..({x1}, {y1})"
                );
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
    }
}
