//! Paints a terminal grid with GPUI's GPU renderer.
//!
//! Each row is split into batches of adjacent cells sharing a style; every
//! batch is shaped once (GPUI caches shaped lines across frames) and placed
//! at its exact column, so a glyph with an odd advance can never push the
//! rest of the row out of alignment. Backgrounds are merged into runs, and
//! box-drawing / block characters are drawn as rectangles so tmux pane
//! borders and TUI frames join seamlessly regardless of line height.

use alacritty_terminal::index::Point as GridPoint;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{self, CursorShape, NamedColor, Rgb};
use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent,
    ShapedLine, SharedString, StrikethroughStyle, Style, TextRun, UnderlineStyle, Window, fill,
    outline, point, px, relative, size,
};

use crate::settings::Settings;
use crate::terminal::{GridSize, resolve_color};
use crate::terminal_view::{GridLayout, TerminalView};
use crate::theme::Color;

pub struct TerminalElement {
    view: Entity<TerminalView>,
}

impl TerminalElement {
    pub fn new(view: Entity<TerminalView>) -> Self {
        Self { view }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub struct Frame {
    hitbox: Hitbox,
    background: Hsla,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(gpui::Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, Hsla, CursorShape)>,
    preedit: Option<(gpui::Point<Pixels>, ShapedLine, Bounds<Pixels>)>,
    line_height: Pixels,
    arrow_cursor: bool,
}

fn rgb_hsla(c: Rgb) -> Hsla {
    Color { r: c.r, g: c.g, b: c.b }.hsla()
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let c = Color { r: a.r, g: a.g, b: a.b }.mix(Color { r: b.r, g: b.g, b: b.b }, t);
    Rgb { r: c.r, g: c.g, b: c.b }
}

#[derive(Clone, PartialEq)]
struct RunStyle {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: Option<UnderlineStyle>,
    strikethrough: bool,
}

struct Batch {
    line: usize,
    start_col: usize,
    next_col: usize,
    text: String,
    style: RunStyle,
    /// Single-cell batch for a non-ASCII glyph; never extended.
    isolated: bool,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Frame;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let settings = Settings::get(cx);
        let view = self.view.read(cx);
        let base_font = settings.font();
        let font_size = settings.font_size(view.font_size_override);
        let padding = px(settings.config.padding);
        let line_height_mult = settings.config.font.line_height;

        let text_system = window.text_system().clone();
        let font_id = text_system.resolve_font(&base_font);
        let cell_width = text_system
            .advance(font_id, font_size, 'm')
            .map(|s| s.width)
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * line_height_mult).round();

        let origin = bounds.origin + point(padding, padding);
        let avail = bounds.size - size(padding * 2.0, padding * 2.0);
        let fit_cols = ((avail.width / cell_width).floor() as usize).max(2);
        let fit_rows = ((avail.height / line_height).floor() as usize).max(1);

        let terminal_entity = view.terminal.clone();
        let fixed = view.fixed_size;
        let focused = view.focus_handle.is_focused(window) && window.is_window_active();
        let blink_visible = view.blink_visible;
        let marked_text = view.marked_text.clone();
        let layout_cell = view.layout.clone();

        if !fixed {
            terminal_entity.update(cx, |t, _| {
                t.resize(GridSize {
                    cols: fit_cols as u16,
                    rows: fit_rows as u16,
                    cell_width: f32::from(cell_width) as u16,
                    cell_height: f32::from(line_height) as u16,
                })
            });
        }

        let terminal = terminal_entity.read(cx);
        let theme = &terminal.theme;
        let grid_size = terminal.size();
        let (cols, rows) = (grid_size.cols as usize, grid_size.rows as usize);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        let term = terminal.term().lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let display_offset = content.display_offset as i32;
        let default_bg = resolve_color(ansi::Color::Named(NamedColor::Background), colors, terminal);
        let default_fg = resolve_color(ansi::Color::Named(NamedColor::Foreground), colors, terminal);
        let cursor_color = colors[NamedColor::Cursor]
            .unwrap_or(Rgb { r: theme.cursor.r, g: theme.cursor.g, b: theme.cursor.b });
        let cursor_text = theme
            .cursor_text
            .map(|c| Rgb { r: c.r, g: c.g, b: c.b })
            .unwrap_or(default_bg);
        let selection_bg = Rgb {
            r: theme.selection_background.r,
            g: theme.selection_background.g,
            b: theme.selection_background.b,
        };
        let selection_fg = theme.selection_foreground.map(|c| Rgb { r: c.r, g: c.g, b: c.b });

        // Cursor placement in viewport coordinates.
        let cursor_point = content.cursor.point;
        let cursor_line = cursor_point.line.0 + display_offset;
        let mut cursor_shape = content.cursor.shape;
        if !focused && cursor_shape != CursorShape::Hidden {
            cursor_shape = CursorShape::HollowBlock;
        } else if !blink_visible {
            cursor_shape = CursorShape::Hidden;
        }
        if marked_text.is_some() {
            cursor_shape = CursorShape::Hidden;
        }
        let cursor_visible = cursor_shape != CursorShape::Hidden
            && cursor_line >= 0
            && (cursor_line as usize) < rows;
        let cursor_cell = (cursor_line.max(0) as usize, cursor_point.column.0);
        let mut cursor_is_wide = false;

        let cell_x = |col: usize| origin.x + cell_width * col as f32;
        let cell_y = |line: usize| origin.y + line_height * line as f32;
        let cell_bounds = |line: usize, col: usize, width: usize| {
            // Snap to whole pixels so adjacent runs never leave hairline gaps.
            let x0 = cell_x(col).round();
            let x1 = cell_x(col + width).round();
            let y0 = cell_y(line).round();
            let y1 = cell_y(line + 1).round();
            Bounds::new(point(x0, y0), size(x1 - x0, y1 - y0))
        };

        let mut rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
        let mut bg_run: Option<(usize, usize, usize, Rgb)> = None; // line, start, end, color
        let mut batches: Vec<Batch> = Vec::new();
        let mut box_rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();

        let flush_bg = |run: &mut Option<(usize, usize, usize, Rgb)>, rects: &mut Vec<_>| {
            if let Some((line, start, end, color)) = run.take() {
                rects.push((cell_bounds(line, start, end - start), rgb_hsla(color)));
            }
        };

        for indexed in content.display_iter {
            let cell = indexed.cell;
            let line = indexed.point.line.0 + display_offset;
            if line < 0 || line as usize >= rows {
                continue;
            }
            let line = line as usize;
            let col = indexed.point.column.0;
            if col >= cols
                || cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

            let (mut fg, mut bg) = (cell.fg, cell.bg);
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let mut fg_rgb = resolve_color(fg, colors, terminal);
            let mut bg_rgb = resolve_color(bg, colors, terminal);
            if cell.flags.contains(Flags::DIM) {
                fg_rgb = mix(fg_rgb, bg_rgb, 0.4);
            }
            let selected = content
                .selection
                .is_some_and(|s| s.contains(GridPoint::new(indexed.point.line, indexed.point.column)));
            if selected {
                bg_rgb = selection_bg;
                if let Some(sfg) = selection_fg {
                    fg_rgb = sfg;
                }
            }
            if cell.flags.contains(Flags::HIDDEN) {
                fg_rgb = bg_rgb;
            }
            let is_cursor = cursor_visible && (line, col) == cursor_cell;
            if is_cursor {
                cursor_is_wide = width == 2;
                if cursor_shape == CursorShape::Block {
                    fg_rgb = cursor_text;
                }
            }

            // Background runs.
            let needs_bg = bg_rgb != default_bg;
            match &mut bg_run {
                Some((l, _, end, color)) if *l == line && *end == col && *color == bg_rgb && needs_bg => {
                    *end = col + width;
                }
                _ => {
                    flush_bg(&mut bg_run, &mut rects);
                    if needs_bg {
                        bg_run = Some((line, col, col + width, bg_rgb));
                    }
                }
            }

            let c = cell.c;
            let fg_hsla = rgb_hsla(fg_rgb);

            // Box drawing and block elements are drawn as geometry.
            if let Some(shapes) = box_drawing::rects(c) {
                let cb = cell_bounds(line, col, 1);
                for r in shapes {
                    box_rects.push((r.place(cb), fg_hsla));
                }
                continue;
            }

            let underline_color = cell
                .underline_color()
                .map(|c| rgb_hsla(resolve_color(c, colors, terminal)))
                .unwrap_or(fg_hsla);
            let underline = if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                Some(UnderlineStyle {
                    thickness: if cell.flags.contains(Flags::DOUBLE_UNDERLINE) { px(2.0) } else { px(1.0) },
                    color: Some(underline_color),
                    wavy: cell.flags.contains(Flags::UNDERCURL),
                })
            } else {
                None
            };
            let style = RunStyle {
                fg: fg_hsla,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline,
                strikethrough: cell.flags.contains(Flags::STRIKEOUT),
            };
            let decorated = style.underline.is_some() || style.strikethrough;
            if c == ' ' && !decorated {
                continue;
            }
            let isolated = !c.is_ascii() || is_cursor || width != 1 || cell.zerowidth().is_some();

            if let Some(last) = batches.last_mut()
                && !isolated
                && !last.isolated
                && last.line == line
                && last.style == style
            {
                // Bridge undecorated blank gaps so a row needs fewer shapes.
                if last.next_col <= col && col - last.next_col <= 4 {
                    for _ in last.next_col..col {
                        last.text.push(' ');
                    }
                    last.text.push(c);
                    last.next_col = col + 1;
                    continue;
                }
            }
            let mut text = String::new();
            text.push(c);
            if let Some(extra) = cell.zerowidth() {
                text.extend(extra.iter());
            }
            batches.push(Batch { line, start_col: col, next_col: col + width, text, style, isolated });
        }
        flush_bg(&mut bg_run, &mut rects);

        // Cursor geometry.
        let cursor = cursor_visible.then(|| {
            let (line, col) = cursor_cell;
            let cb = cell_bounds(line, col.min(cols.saturating_sub(1)), if cursor_is_wide { 2 } else { 1 });
            let thickness = px((f32::from(cell_width) / 8.0).max(1.0).round());
            let b = match cursor_shape {
                CursorShape::Beam => Bounds::new(cb.origin, size(thickness * 2.0, cb.size.height)),
                CursorShape::Underline => Bounds::new(
                    point(cb.origin.x, cb.origin.y + cb.size.height - thickness * 2.0),
                    size(cb.size.width, thickness * 2.0),
                ),
                _ => cb,
            };
            (b, rgb_hsla(cursor_color), cursor_shape)
        });
        let cursor_cell_bounds = {
            let (line, col) = cursor_cell;
            cell_bounds(line.min(rows.saturating_sub(1)), col.min(cols.saturating_sub(1)), 1)
        };
        drop(term);

        // Shape text.
        let mut lines = Vec::with_capacity(batches.len());
        for batch in batches {
            let mut font = base_font.clone();
            if batch.style.bold {
                font.weight = FontWeight::BOLD;
            }
            if batch.style.italic {
                font.style = FontStyle::Italic;
            }
            let run = TextRun {
                len: batch.text.len(),
                font,
                color: batch.style.fg,
                background_color: None,
                underline: batch.style.underline,
                strikethrough: batch.style.strikethrough.then_some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(batch.style.fg),
                }),
            };
            let shaped = text_system.shape_line(SharedString::from(batch.text), font_size, &[run], None);
            lines.push((point(cell_x(batch.start_col), cell_y(batch.line)), shaped));
        }

        // IME pre-edit text is drawn at the cursor with an underline.
        let preedit = marked_text.map(|text| {
            let run = TextRun {
                len: text.len(),
                font: base_font.clone(),
                color: rgb_hsla(default_fg),
                background_color: None,
                underline: Some(UnderlineStyle { thickness: px(1.0), color: None, wavy: false }),
                strikethrough: None,
            };
            let shaped = text_system.shape_line(SharedString::from(text), font_size, &[run], None);
            let width = shaped.width;
            let origin = cursor_cell_bounds.origin;
            (origin, shaped, Bounds::new(origin, size(width, line_height)))
        });

        rects.extend(box_rects);

        layout_cell.set(Some(GridLayout {
            origin,
            cell_width,
            line_height,
            cols,
            rows,
            cursor: Some(cursor_cell_bounds),
        }));

        Frame {
            hitbox,
            background: rgb_hsla(default_bg),
            rects,
            lines,
            cursor,
            preedit,
            line_height,
            arrow_cursor: self.view.read(cx).mouse_cursor_is_arrow(cx),
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, frame.background));
            for (b, color) in &frame.rects {
                window.paint_quad(fill(*b, *color));
            }
            if let Some((b, color, shape)) = frame.cursor
                && shape == CursorShape::Block
            {
                window.paint_quad(fill(b, color));
            }
            for (origin, line) in &frame.lines {
                line.paint(*origin, frame.line_height, window, cx).ok();
            }
            match frame.cursor {
                Some((b, color, CursorShape::HollowBlock)) => {
                    window.paint_quad(outline(b, color, gpui::BorderStyle::Solid));
                }
                Some((b, color, CursorShape::Beam | CursorShape::Underline)) => {
                    window.paint_quad(fill(b, color));
                }
                _ => {}
            }
            if let Some((origin, line, b)) = &frame.preedit {
                window.paint_quad(fill(*b, frame.background));
                line.paint(*origin, frame.line_height, window, cx).ok();
            }
        });

        let focus = self.view.read(cx).focus_handle.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.view.clone()), cx);
        window.set_cursor_style(
            if frame.arrow_cursor { CursorStyle::Arrow } else { CursorStyle::IBeam },
            &frame.hitbox,
        );

        let view = self.view.clone();
        let hitbox = frame.hitbox.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                view.update(cx, |view, cx| view.mouse_down(e, window, cx));
                cx.stop_propagation();
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |view, cx| view.mouse_move(e, window, cx));
            }
        });
        let view = self.view.clone();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble {
                view.update(cx, |view, cx| view.mouse_up(e, window, cx));
            }
        });
        let view = self.view.clone();
        let hitbox = frame.hitbox.clone();
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                view.update(cx, |view, cx| view.scroll_wheel(e, window, cx));
                cx.stop_propagation();
            }
        });
    }
}

/// Procedural box-drawing (U+2500–U+257F subset) and block elements
/// (U+2580–U+259F).
pub mod box_drawing {
    use gpui::{Bounds, Pixels, point, px, size};

    /// A rectangle in cell-relative units (0..1), with `stroke` units for
    /// line thickness measured in multiples of the base stroke.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Rect {
        pub x0: f32,
        pub y0: f32,
        pub x1: f32,
        pub y1: f32,
        /// Additional half-thickness in base strokes added around a
        /// zero-width/height center line (0 = use the coordinates as-is).
        pub stroke_x: f32,
        pub stroke_y: f32,
        /// Offset from the center line in base strokes (for double lines).
        pub offset_x: f32,
        pub offset_y: f32,
    }

    impl Rect {
        const fn area(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
            Rect { x0, y0, x1, y1, stroke_x: 0.0, stroke_y: 0.0, offset_x: 0.0, offset_y: 0.0 }
        }

        pub fn place(&self, cell: Bounds<Pixels>) -> Bounds<Pixels> {
            let w = f32::from(cell.size.width);
            let h = f32::from(cell.size.height);
            let base = (w / 8.0).max(1.0).round();
            let ox = f32::from(cell.origin.x);
            let oy = f32::from(cell.origin.y);
            let mut x0 = ox + self.x0 * w;
            let mut x1 = ox + self.x1 * w;
            let mut y0 = oy + self.y0 * h;
            let mut y1 = oy + self.y1 * h;
            if self.stroke_x > 0.0 {
                let t = (base * self.stroke_x).max(1.0);
                let c = (x0 + self.offset_x * base * 2.0 - t / 2.0).round();
                x0 = c;
                x1 = c + t;
            }
            if self.stroke_y > 0.0 {
                let t = (base * self.stroke_y).max(1.0);
                let c = (y0 + self.offset_y * base * 2.0 - t / 2.0).round();
                y0 = c;
                y1 = c + t;
            }
            Bounds::new(point(px(x0.round()), px(y0.round())), size(px((x1 - x0).round().max(1.0)), px((y1 - y0).round().max(1.0))))
        }
    }

    /// Arm weights: 0 none, 1 light, 2 heavy, 3 double. Order: up, right, down, left.
    pub fn arms(c: char) -> Option<[u8; 4]> {
        Some(match c {
            '─' => [0, 1, 0, 1],
            '━' => [0, 2, 0, 2],
            '│' => [1, 0, 1, 0],
            '┃' => [2, 0, 2, 0],
            '┄' | '┈' | '╌' => [0, 1, 0, 1],
            '┅' | '┉' | '╍' => [0, 2, 0, 2],
            '┆' | '┊' | '╎' => [1, 0, 1, 0],
            '┇' | '┋' | '╏' => [2, 0, 2, 0],
            '┌' | '╭' => [0, 1, 1, 0],
            '┍' => [0, 2, 1, 0],
            '┎' => [0, 1, 2, 0],
            '┏' => [0, 2, 2, 0],
            '┐' | '╮' => [0, 0, 1, 1],
            '┑' => [0, 0, 1, 2],
            '┒' => [0, 0, 2, 1],
            '┓' => [0, 0, 2, 2],
            '└' | '╰' => [1, 1, 0, 0],
            '┕' => [1, 2, 0, 0],
            '┖' => [2, 1, 0, 0],
            '┗' => [2, 2, 0, 0],
            '┘' | '╯' => [1, 0, 0, 1],
            '┙' => [1, 0, 0, 2],
            '┚' => [2, 0, 0, 1],
            '┛' => [2, 0, 0, 2],
            '├' => [1, 1, 1, 0],
            '┝' => [1, 2, 1, 0],
            '┠' => [2, 1, 2, 0],
            '┣' => [2, 2, 2, 0],
            '┤' => [1, 0, 1, 1],
            '┥' => [1, 0, 1, 2],
            '┨' => [2, 0, 2, 1],
            '┫' => [2, 0, 2, 2],
            '┬' => [0, 1, 1, 1],
            '┯' => [0, 2, 1, 2],
            '┰' => [0, 1, 2, 1],
            '┳' => [0, 2, 2, 2],
            '┴' => [1, 1, 0, 1],
            '┷' => [1, 2, 0, 2],
            '┸' => [2, 1, 0, 1],
            '┻' => [2, 2, 0, 2],
            '┼' => [1, 1, 1, 1],
            '┿' => [1, 2, 1, 2],
            '╂' => [2, 1, 2, 1],
            '╋' => [2, 2, 2, 2],
            '═' => [0, 3, 0, 3],
            '║' => [3, 0, 3, 0],
            '╔' => [0, 3, 3, 0],
            '╗' => [0, 0, 3, 3],
            '╚' => [3, 3, 0, 0],
            '╝' => [3, 0, 0, 3],
            '╠' => [3, 3, 3, 0],
            '╣' => [3, 0, 3, 3],
            '╦' => [0, 3, 3, 3],
            '╩' => [3, 3, 0, 3],
            '╬' => [3, 3, 3, 3],
            '╴' => [0, 0, 0, 1],
            '╵' => [1, 0, 0, 0],
            '╶' => [0, 1, 0, 0],
            '╷' => [0, 0, 1, 0],
            '╸' => [0, 0, 0, 2],
            '╹' => [2, 0, 0, 0],
            '╺' => [0, 2, 0, 0],
            '╻' => [0, 0, 2, 0],
            '╼' => [0, 2, 0, 1],
            '╽' => [1, 0, 2, 0],
            '╾' => [0, 1, 0, 2],
            '╿' => [2, 0, 1, 0],
            _ => return None,
        })
    }

    fn line_rects(arms: [u8; 4]) -> Vec<Rect> {
        let [up, right, down, left] = arms;
        let mut out = Vec::new();
        // Vertical arms. Each arm extends past the center by half the
        // thickest crossing stroke so joints are filled.
        let vertical = |from: f32, to: f32, weight: u8, out: &mut Vec<Rect>| match weight {
            1 | 2 => out.push(Rect {
                stroke_x: weight as f32,
                ..Rect::area(0.5, from, 0.5, to)
            }),
            3 => {
                for off in [-0.5, 0.5] {
                    out.push(Rect { stroke_x: 1.0, offset_x: off, ..Rect::area(0.5, from, 0.5, to) });
                }
            }
            _ => {}
        };
        let horizontal = |from: f32, to: f32, weight: u8, out: &mut Vec<Rect>| match weight {
            1 | 2 => out.push(Rect {
                stroke_y: weight as f32,
                ..Rect::area(from, 0.5, to, 0.5)
            }),
            3 => {
                for off in [-0.5, 0.5] {
                    out.push(Rect { stroke_y: 1.0, offset_y: off, ..Rect::area(from, 0.5, to, 0.5) });
                }
            }
            _ => {}
        };
        // Overlap generously past the center; the exact amount hidden under
        // the crossing stroke doesn't matter visually.
        let has_h = left > 0 || right > 0;
        let has_v = up > 0 || down > 0;
        let v_end = if has_h { 0.56 } else { 0.5 };
        let h_end = if has_v { 0.56 } else { 0.5 };
        vertical(0.0, v_end, up, &mut out);
        vertical(1.0 - v_end, 1.0, down, &mut out);
        horizontal(0.0, h_end, left, &mut out);
        horizontal(1.0 - h_end, 1.0, right, &mut out);
        out
    }

    fn block_rects(c: char) -> Option<Vec<Rect>> {
        let eighth = |n: f32| n / 8.0;
        Some(match c {
            '█' => vec![Rect::area(0.0, 0.0, 1.0, 1.0)],
            '▀' => vec![Rect::area(0.0, 0.0, 1.0, 0.5)],
            '▄' => vec![Rect::area(0.0, 0.5, 1.0, 1.0)],
            '▌' => vec![Rect::area(0.0, 0.0, 0.5, 1.0)],
            '▐' => vec![Rect::area(0.5, 0.0, 1.0, 1.0)],
            '▁'..='▇' => {
                let n = (c as u32 - '▁' as u32 + 1) as f32;
                vec![Rect::area(0.0, 1.0 - eighth(n), 1.0, 1.0)]
            }
            '▉'..='▏' => {
                let n = (8 - (c as u32 - '▉' as u32 + 1)) as f32;
                vec![Rect::area(0.0, 0.0, eighth(n), 1.0)]
            }
            '▔' => vec![Rect::area(0.0, 0.0, 1.0, eighth(1.0))],
            '▕' => vec![Rect::area(1.0 - eighth(1.0), 0.0, 1.0, 1.0)],
            '▖' => vec![Rect::area(0.0, 0.5, 0.5, 1.0)],
            '▗' => vec![Rect::area(0.5, 0.5, 1.0, 1.0)],
            '▘' => vec![Rect::area(0.0, 0.0, 0.5, 0.5)],
            '▝' => vec![Rect::area(0.5, 0.0, 1.0, 0.5)],
            '▙' => vec![Rect::area(0.0, 0.0, 0.5, 1.0), Rect::area(0.5, 0.5, 1.0, 1.0)],
            '▚' => vec![Rect::area(0.0, 0.0, 0.5, 0.5), Rect::area(0.5, 0.5, 1.0, 1.0)],
            '▛' => vec![Rect::area(0.0, 0.0, 1.0, 0.5), Rect::area(0.0, 0.5, 0.5, 1.0)],
            '▜' => vec![Rect::area(0.0, 0.0, 1.0, 0.5), Rect::area(0.5, 0.5, 1.0, 1.0)],
            '▞' => vec![Rect::area(0.5, 0.0, 1.0, 0.5), Rect::area(0.0, 0.5, 0.5, 1.0)],
            '▟' => vec![Rect::area(0.5, 0.0, 1.0, 0.5), Rect::area(0.0, 0.5, 1.0, 1.0)],
            _ => return None,
        })
    }

    pub fn rects(c: char) -> Option<Vec<Rect>> {
        if !('\u{2500}'..='\u{259f}').contains(&c) {
            return None;
        }
        if let Some(arms) = arms(c) {
            return Some(line_rects(arms));
        }
        block_rects(c)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn cell() -> Bounds<Pixels> {
            Bounds::new(point(px(100.0), px(200.0)), size(px(8.0), px(20.0)))
        }

        #[test]
        fn vertical_line_spans_full_cell_height() {
            let rects = rects('│').unwrap();
            let top = rects.iter().map(|r| r.place(cell()).origin.y).fold(f32::MAX, |a, b| a.min(f32::from(b)));
            let bottom = rects
                .iter()
                .map(|r| r.place(cell()).bottom())
                .fold(0.0f32, |a, b| a.max(f32::from(b)));
            assert_eq!(top, 200.0);
            assert_eq!(bottom, 220.0);
        }

        #[test]
        fn cross_has_four_arms() {
            assert_eq!(rects('┼').unwrap().len(), 4);
            assert_eq!(rects('╬').unwrap().len(), 8);
            assert_eq!(rects('─').unwrap().len(), 2);
        }

        #[test]
        fn blocks() {
            let full = rects('█').unwrap()[0].place(cell());
            assert_eq!(full, cell());
            let lower = rects('▄').unwrap()[0].place(cell());
            assert_eq!(f32::from(lower.origin.y), 210.0);
            assert_eq!(f32::from(rects('▏').unwrap()[0].place(cell()).size.width), 1.0);
            assert_eq!(f32::from(rects('▉').unwrap()[0].place(cell()).size.width), 7.0);
        }

        #[test]
        fn other_chars_are_text() {
            assert!(rects('a').is_none());
            assert!(rects('░').is_none());
        }
    }
}
