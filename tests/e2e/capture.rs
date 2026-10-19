//! Window captures: the X11 window a process owns, read with `GetImage`
//! (Brindle runs under Xwayland for this), and color checks on cell
//! regions of it.

use std::collections::HashMap;
use std::path::Path;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, ImageFormat, MapState, Window};

use crate::brindle::Grid;
use crate::{Result, ensure};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// The form terminals use in OSC 10/11 replies.
    pub fn report(self) -> String {
        let c = |v: u8| format!("{v:02x}{v:02x}");
        format!("rgb:{}/{}/{}", c(self.0), c(self.1), c(self.2))
    }

    /// Within `tolerance` on every channel; text edges are blended.
    pub fn near(self, other: Rgb, tolerance: u8) -> bool {
        self.0.abs_diff(other.0) <= tolerance
            && self.1.abs_diff(other.1) <= tolerance
            && self.2.abs_diff(other.2) <= tolerance
    }
}

/// A rectangle in device pixels, end-exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGB, row by row.
    data: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> Rgb {
        let i = ((y * self.width + x) * 3) as usize;
        Rgb(self.data[i], self.data[i + 1], self.data[i + 2])
    }

    fn pixels(&self, r: PxRect) -> impl Iterator<Item = Rgb> + '_ {
        let (x1, y1) = (r.x1.min(self.width), r.y1.min(self.height));
        (r.y0..y1).flat_map(move |y| (r.x0..x1).map(move |x| self.pixel(x, y)))
    }

    /// How many pixels in `r` are close to `color`.
    pub fn count(&self, r: PxRect, color: Rgb) -> usize {
        self.pixels(r).filter(|p| p.near(color, 24)).count()
    }

    /// The most common color in `r`.
    pub fn dominant(&self, r: PxRect) -> Option<Rgb> {
        let mut counts: HashMap<Rgb, usize> = HashMap::new();
        for p in self.pixels(r) {
            *counts.entry(p).or_default() += 1;
        }
        counts.into_iter().max_by_key(|&(_, n)| n).map(|(c, _)| c)
    }

    /// The colors in `r`, most common first, for failure messages.
    pub fn summary(&self, r: PxRect) -> String {
        let mut counts: HashMap<Rgb, usize> = HashMap::new();
        for p in self.pixels(r) {
            *counts.entry(p).or_default() += 1;
        }
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        counts.iter().take(5).map(|(c, n)| format!("{} x{n}", c.hex())).collect::<Vec<_>>().join(", ")
    }

    /// Fails unless at least `min` pixels in `r` are close to `color`.
    pub fn expect(&self, what: &str, r: PxRect, color: Rgb, min: usize) -> Result {
        let n = self.count(r, color);
        ensure!(n >= min, "{what}: {n} pixels of {} in {r:?} (want {min}); found {}", color.hex(), self.summary(r));
        Ok(())
    }

    /// Fails if more than a few pixels in `r` are close to `color`.
    pub fn expect_none(&self, what: &str, r: PxRect, color: Rgb) -> Result {
        let n = self.count(r, color);
        ensure!(n <= 2, "{what}: {n} pixels of {} in {r:?}; found {}", color.hex(), self.summary(r));
        Ok(())
    }

    pub fn save_ppm(&self, path: &Path) {
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.extend_from_slice(&self.data);
        std::fs::write(path, out).ok();
    }
}

/// One arm of a divider junction: the line running from the crossing point
/// to the edge of the sampled rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// No line pixels at all.
    None,
    /// Line pixels that don't run unbroken from the crossing to the edge.
    Gap,
    /// An unbroken line that changes color along the way.
    Mixed,
    Color(Rgb),
}

/// The four arms of a junction, from the crossing point outwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Junction {
    pub up: Arm,
    pub down: Arm,
    pub left: Arm,
    pub right: Arm,
}

impl Image {
    /// Reads the divider junction in `r`. Line pixels are those near one of
    /// `colors`. The crossing is where the row and the column with the most
    /// line pixels meet; lines several pixels thick (scaled windows) count
    /// as one band each.
    pub fn junction(&self, r: PxRect, colors: &[Rgb]) -> Result<Junction> {
        let (x1, y1) = (r.x1.min(self.width), r.y1.min(self.height));
        ensure!(r.x0 < x1 && r.y0 < y1, "empty junction rectangle {r:?}");
        let line = |x: u32, y: u32| colors.iter().position(|&c| self.pixel(x, y).near(c, 24));
        // The band of rows (or columns) around the busiest one.
        let band = |counts: Vec<usize>, start: u32| -> Result<(u32, u32)> {
            let max = counts.iter().copied().max().unwrap_or(0);
            ensure!(max > 0, "no line pixels in {r:?}");
            let peak = counts.iter().position(|&n| n == max).unwrap();
            let thick = |i: usize| counts[i] * 2 >= max;
            let lo = (0..=peak).rev().take_while(|&i| thick(i)).last().unwrap_or(peak);
            let hi = (peak..counts.len()).take_while(|&i| thick(i)).last().unwrap_or(peak);
            Ok((start + lo as u32, start + hi as u32))
        };
        let rows = (r.y0..y1).map(|y| (r.x0..x1).filter(|&x| line(x, y).is_some()).count()).collect();
        let cols = (r.x0..x1).map(|x| (r.y0..y1).filter(|&y| line(x, y).is_some()).count()).collect();
        let (top, bottom) = band(rows, r.y0)?;
        let (left, right) = band(cols, r.x0)?;
        let (cx, cy) = ((left + right) / 2, (top + bottom) / 2);
        // An arm's pixels, from the crossing band outwards.
        let arm = |pixels: Vec<Option<usize>>| -> Arm {
            if pixels.iter().all(Option::is_none) {
                return Arm::None;
            }
            if pixels.iter().any(Option::is_none) {
                return Arm::Gap;
            }
            let first = pixels[0].unwrap();
            if pixels.iter().all(|&p| p == Some(first)) { Arm::Color(colors[first]) } else { Arm::Mixed }
        };
        Ok(Junction {
            up: arm((r.y0..top).rev().map(|y| line(cx, y)).collect()),
            down: arm((bottom + 1..y1).map(|y| line(cx, y)).collect()),
            left: arm((r.x0..left).rev().map(|x| line(x, cy)).collect()),
            right: arm((right + 1..x1).map(|x| line(x, cy)).collect()),
        })
    }
}

/// The cell grid of a whole tab, in device pixels.
#[derive(Debug, Clone, Copy)]
pub struct Cells {
    x: f32,
    y: f32,
    cell_width: f32,
    line_height: f32,
    scale: f32,
}

impl Cells {
    /// From the dumped grid of the active pane, which sits at `left`,`top`
    /// in tmux's window.
    pub fn from_pane(grid: Grid, left: u16, top: u16) -> Cells {
        Cells {
            x: grid.x - grid.cell_width * left as f32,
            y: grid.y - grid.line_height * top as f32,
            cell_width: grid.cell_width,
            line_height: grid.line_height,
            scale: grid.scale,
        }
    }

    /// `cols` x `rows` cells starting at `col`,`row`.
    pub fn rect(&self, col: u16, row: u16, cols: u16, rows: u16) -> PxRect {
        let px = |v: f32| (v * self.scale).round().max(0.0) as u32;
        PxRect {
            x0: px(self.x + self.cell_width * col as f32),
            y0: px(self.y + self.line_height * row as f32),
            x1: px(self.x + self.cell_width * (col + cols) as f32),
            y1: px(self.y + self.line_height * (row + rows) as f32),
        }
    }
}

/// Window captures are X11-only; cases that need them run on Linux.
#[cfg(not(target_os = "linux"))]
pub fn grab(_display: &str, _pid: u32) -> Result<Image> {
    Err("window captures need X11".into())
}

/// Captures the viewable X11 window whose `_NET_WM_PID` is `pid`.
#[cfg(target_os = "linux")]
pub fn grab(display: &str, pid: u32) -> Result<Image> {
    let (conn, screen) = x11rb::connect(Some(display)).map_err(|e| format!("X display {display}: {e}"))?;
    let root = conn.setup().roots[screen].root;
    let atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?
        .atom;
    let deadline = Instant::now() + Duration::from_secs(3);
    let window = loop {
        if let Some(w) = find_window(&conn, root, atom, pid) {
            break w;
        }
        ensure!(Instant::now() < deadline, "no viewable window for pid {pid} on {display}");
        std::thread::sleep(Duration::from_millis(100));
    };
    let geometry = conn.get_geometry(window).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
    // A window that hasn't presented a frame yet reads back as all zeros,
    // which no theme paints; a loaded machine (lavapipe on CI) can get there.
    let deadline = Instant::now() + Duration::from_secs(3);
    let image = loop {
        let image = conn
            .get_image(ImageFormat::Z_PIXMAP, window, 0, 0, geometry.width, geometry.height, !0)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| format!("GetImage: {e}"))?;
        if image.data.chunks_exact(4).any(|px| px[..3] != [0, 0, 0]) || Instant::now() >= deadline {
            break image;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    ensure!(image.depth >= 24, "unexpected window depth {}", image.depth);
    let mut data = Vec::with_capacity(geometry.width as usize * geometry.height as usize * 3);
    for px in image.data.chunks_exact(4) {
        data.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    Ok(Image { width: geometry.width.into(), height: geometry.height.into(), data })
}

#[cfg(target_os = "linux")]
fn find_window(conn: &impl Connection, window: Window, atom: Atom, pid: u32) -> Option<Window> {
    let owned = conn
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()
        .and_then(|r| r.value32().and_then(|mut v| v.next()))
        == Some(pid);
    if owned {
        let attrs = conn.get_window_attributes(window).ok()?.reply().ok()?;
        if attrs.map_state == MapState::VIEWABLE {
            return Some(window);
        }
    }
    let tree = conn.query_tree(window).ok()?.reply().ok()?;
    tree.children.iter().find_map(|&child| find_window(conn, child, atom, pid))
}

pub fn self_test() -> Result {
    let grid = Grid { x: 6.0 + 8.5 * 51.0, y: 51.0 + 18.0, cell_width: 8.5, line_height: 18.0, scale: 2.0 };
    let cells = Cells::from_pane(grid, 51, 1);
    ensure!(cells.rect(0, 0, 1, 1) == PxRect { x0: 12, y0: 102, x1: 29, y1: 138 }, "{:?}", cells.rect(0, 0, 1, 1));
    ensure!(cells.rect(51, 1, 2, 1) == PxRect { x0: 879, y0: 138, x1: 913, y1: 174 }, "{:?}", cells.rect(51, 1, 2, 1));
    ensure!(Rgb(0x10, 0x20, 0x30).report() == "rgb:1010/2020/3030", "report");
    ensure!(Rgb(100, 100, 100).near(Rgb(120, 90, 100), 24) && !Rgb(0, 0, 0).near(Rgb(30, 0, 0), 24), "near");
    Ok(())
}

pub fn self_test_junction() -> Result {
    const BG: Rgb = Rgb(0x10, 0x14, 0x18);
    const A: Rgb = Rgb(0xff, 0xff, 0x00);
    const B: Rgb = Rgb(0xff, 0x00, 0xff);
    let colors = [A, B];
    // A 20x20 image, lines 2 pixels thick at rows/columns 9-10.
    let image = |paint: &dyn Fn(u32, u32) -> Option<Rgb>| {
        let mut data = Vec::new();
        for y in 0..20 {
            for x in 0..20 {
                let c = paint(x, y).unwrap_or(BG);
                data.extend_from_slice(&[c.0, c.1, c.2]);
            }
        }
        Image { width: 20, height: 20, data }
    };
    let on = |v: u32| (9..=10).contains(&v);
    let all = PxRect { x0: 0, y0: 0, x1: 20, y1: 20 };

    // A cross whose highlight (A) turns from up to left on the lines.
    let cross = image(&|x, y| match (on(x), on(y)) {
        (true, _) if y <= 10 => Some(A),
        (true, _) => Some(B),
        (_, true) if x <= 10 => Some(A),
        (_, true) => Some(B),
        _ => None,
    });
    let got = cross.junction(all, &colors)?;
    let want = Junction { up: Arm::Color(A), down: Arm::Color(B), left: Arm::Color(A), right: Arm::Color(B) };
    ensure!(got == want, "cross: {got:?}");

    // A T: the vertical line, and a horizontal one going right only.
    let tee = image(&|x, y| (on(x) || (on(y) && x > 10)).then_some(A));
    let got = tee.junction(all, &colors)?;
    let want = Junction { up: Arm::Color(A), down: Arm::Color(A), left: Arm::None, right: Arm::Color(A) };
    ensure!(got == want, "tee: {got:?}");

    // The horizontal line stops a pixel short of the vertical one.
    let gap = image(&|x, y| (on(x) || (on(y) && x != 11)).then_some(A));
    let got = gap.junction(all, &colors)?;
    ensure!(got.right == Arm::Gap && got.left == Arm::Color(A), "gap: {got:?}");

    // The color changes partway along an arm.
    let mixed = image(&|x, y| (on(x) || on(y)).then_some(if y < 4 { B } else { A }));
    let got = mixed.junction(all, &colors)?;
    ensure!(got.up == Arm::Mixed && got.down == Arm::Color(A), "mixed: {got:?}");
    Ok(())
}
