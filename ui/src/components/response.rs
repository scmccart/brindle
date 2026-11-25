//! Rich text as plain data: the blocks a response draws, after AI
//! Elements' Response.
//!
//! These are the shared contract between an owner that reads some markup
//! (the app's Markdown renderer) and the components that draw it. The crate
//! interprets no markup of its own: an owner builds these values and hands
//! them over. They hold no entity and no callback, so they compare, clone
//! and print like any value.
//!
//! [`Inline`] is the one type with an invariant: its runs cover its text
//! exactly and its links lie inside it, all on char boundaries and the links
//! apart, so a component slices it without a check that could panic. Its
//! fields are private and [`Inline::new`] checks them.
//!
//! [`Response`] draws a list of these blocks:
//!
//! - Paragraphs and headings are one `StyledText` each, its runs built from
//!   the [`Inline`]'s ([`text_runs`]): strong in a bold weight, emphasis in
//!   italics, strikethrough struck, code in the owner's code family
//!   (`.code_font`) in the foreground on the muted surface, muted runs in
//!   the muted foreground, and links in the `link` colour, underlined.
//!   Headings are bold, levels 1–3 at 1.25, 1.125 and 1 rem and the rest at
//!   the size around them.
//! - Lists have a marker column ([`list_marker`]: a bullet, the number from
//!   the list's start, or a drawn task mark) and their items' blocks beside
//!   it, so a nested list is indented by its parent's marker column.
//! - Quotes have a `border` bar at the start and their text in the muted
//!   foreground; rules are a `border` line.
//! - Code blocks are [`CodeBlock`]s, their copy button reporting through
//!   `.on_copy_code`.
//! - Tables are a grid of equal-width columns in a `border` frame: the
//!   header row bold with a line under it, a line between body rows, each
//!   column aligned from its [`ColumnAlign`], cells wrapping.
//!
//! **Links.** A text with links is an `InteractiveText`. A press released
//! over the same link calls `.on_link` with the link and the modifiers held
//! at the release; the owner decides what that means (its own gesture). The
//! pointer is a pointing hand over a link only while the modifiers the
//! owner named (`.link_modifiers`) are held ([`holds_modifiers`]); a
//! pointer move that changes that redraws the window, and an owner whose
//! focused ancestor sees a modifiers change redraws too
//! (`on_modifiers_changed`), so a still pointer follows the keys. A link
//! with a tooltip shows it on hover ([`tooltip_at`]); one without shows
//! none.

use std::fmt;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, CursorStyle, DispatchPhase, ElementId, Font, FontStyle, FontWeight, HitboxBehavior, Hsla,
    InteractiveElement, InteractiveText, IntoElement, Modifiers, MouseMoveEvent, ParentElement, Pixels, RenderOnce, SharedString,
    StrikethroughStyle, StyledText, Styled, TextAlign, TextRun, UnderlineStyle, Window, canvas, div,
    prelude::FluentBuilder, px, rems,
};

use crate::components::code_block::{CodeBlock, OnCopy};
use crate::components::tooltip::Tooltip;
use crate::icon::Icon;
use crate::style::{CHECKBOX_RADIUS, CHECKBOX_TICK_SIZE, checkbox_style};
use crate::theme::{ActiveTheme, Tokens};

/// One block of rich text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichBlock {
    Paragraph(Inline),
    Heading { level: HeadingLevel, text: Inline },
    Code(Code),
    List { kind: ListKind, items: Vec<ListItem> },
    Quote(Vec<RichBlock>),
    Table(Table),
    Rule,
}

/// A heading's level, 1 to 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
}

/// Whether a list is bulleted or numbered, and from where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Numbered { start: u64 },
}

/// One item of a list: its task mark, if it is a task, and its blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub task: Option<Task>,
    pub blocks: Vec<RichBlock>,
}

/// A task item's mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Open,
    Done,
}

/// A code block: its label (a language name, say) when it has one, and its
/// text exactly as it is to be shown and copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub label: Option<SharedString>,
    pub text: SharedString,
}

/// A table column's alignment. Not the placement primitive's `Align`, which
/// lines a box up with its trigger and centres by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlign {
    #[default]
    Start,
    Center,
    End,
}

/// A table: a header row and body rows, every row one cell per column.
/// [`Table::new`] makes the rows that wide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    columns: Vec<ColumnAlign>,
    head: Vec<Inline>,
    rows: Vec<Vec<Inline>>,
}

impl Table {
    /// A table as wide as its header. Short rows are padded with empty cells
    /// and cells past the header's width are dropped, as GFM does; so are
    /// alignments past it, and missing ones are [`ColumnAlign::Start`].
    pub fn new(mut columns: Vec<ColumnAlign>, head: Vec<Inline>, mut rows: Vec<Vec<Inline>>) -> Self {
        let width = head.len();
        columns.resize(width, ColumnAlign::Start);
        for row in &mut rows {
            row.resize_with(width, Inline::default);
        }
        Self { columns, head, rows }
    }

    /// The number of columns.
    pub fn width(&self) -> usize {
        self.head.len()
    }

    /// Each column's alignment, one per column.
    pub fn columns(&self) -> &[ColumnAlign] {
        &self.columns
    }

    /// The header row, one cell per column.
    pub fn head(&self) -> &[Inline] {
        &self.head
    }

    /// The body rows, each one cell per column.
    pub fn rows(&self) -> &[Vec<Inline>] {
        &self.rows
    }
}

/// How a run of text is drawn. The flags are independent: text can be
/// strong and struck at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RunStyle {
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    /// Inline code: the owner's code family on the muted surface.
    pub code: bool,
    /// Secondary text, in the muted foreground.
    pub muted: bool,
}

/// A link over a range of an [`Inline`]'s text, in bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineLink {
    pub range: Range<usize>,
    pub target: SharedString,
    /// What a hover shows, when the owner wants one shown (the target, when
    /// it differs from the shown text).
    pub tooltip: Option<SharedString>,
}

/// A line of styled text with links: the text, its runs as (length in
/// bytes, style) in order, and its links in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    text: SharedString,
    runs: Vec<(usize, RunStyle)>,
    links: Vec<InlineLink>,
}

/// Why [`Inline::new`] refused its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineError {
    /// The runs' lengths don't add up to the text's length.
    RunsDontCoverText { runs: usize, text: usize },
    /// A run or link boundary falls inside a character.
    NotCharBoundary { at: usize },
    /// A link's range doesn't lie inside the text.
    LinkOutOfRange { range: Range<usize>, text: usize },
    /// Two links share some of the text.
    LinksOverlap { first: Range<usize>, second: Range<usize> },
}

impl fmt::Display for InlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunsDontCoverText { runs, text } => {
                write!(f, "runs cover {runs} bytes of a {text}-byte text")
            }
            Self::NotCharBoundary { at } => write!(f, "byte {at} is inside a character"),
            Self::LinkOutOfRange { range, text } => {
                write!(f, "link {range:?} lies outside a {text}-byte text")
            }
            Self::LinksOverlap { first, second } => write!(f, "links {first:?} and {second:?} overlap"),
        }
    }
}

impl std::error::Error for InlineError {}

impl Inline {
    /// Text with its runs and links, checked: the runs' lengths add up to the
    /// text's, every run and link boundary is a char boundary, and the links
    /// lie inside the text without sharing any of it. The links are kept in
    /// the order of their starts.
    pub fn new(
        text: impl Into<SharedString>,
        runs: Vec<(usize, RunStyle)>,
        mut links: Vec<InlineLink>,
    ) -> Result<Self, InlineError> {
        let text = text.into();
        let len = text.len();
        let covered = runs
            .iter()
            .try_fold(0usize, |sum, (run, _)| sum.checked_add(*run))
            .unwrap_or(usize::MAX);
        if covered != len {
            return Err(InlineError::RunsDontCoverText { runs: covered, text: len });
        }
        let mut at = 0;
        for (run, _) in &runs {
            at += run;
            if !text.is_char_boundary(at) {
                return Err(InlineError::NotCharBoundary { at });
            }
        }
        for link in &links {
            let Range { start, end } = link.range;
            if start > end || end > len {
                return Err(InlineError::LinkOutOfRange { range: link.range.clone(), text: len });
            }
            if let Some(at) = [start, end].into_iter().find(|&at| !text.is_char_boundary(at)) {
                return Err(InlineError::NotCharBoundary { at });
            }
        }
        links.sort_by_key(|link| (link.range.start, link.range.end));
        if let Some(pair) = links.windows(2).find(|pair| pair[0].range.end > pair[1].range.start) {
            return Err(InlineError::LinksOverlap {
                first: pair[0].range.clone(),
                second: pair[1].range.clone(),
            });
        }
        Ok(Self { text, runs, links })
    }

    /// Text in one unstyled run, with no links.
    pub fn plain(text: impl Into<SharedString>) -> Self {
        let text = text.into();
        let runs = match text.len() {
            0 => Vec::new(),
            len => vec![(len, RunStyle::default())],
        };
        Self { text, runs, links: Vec::new() }
    }

    pub fn text(&self) -> &SharedString {
        &self.text
    }

    /// The runs as (length in bytes, style), in order, covering the text.
    pub fn runs(&self) -> &[(usize, RunStyle)] {
        &self.runs
    }

    /// The links, in the order of their starts, none sharing any text.
    pub fn links(&self) -> &[InlineLink] {
        &self.links
    }
}

// ---- drawing ------------------------------------------------------------------

/// One stretch of an [`Inline`] drawn alike: its length in bytes, its run's
/// style and whether a link covers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub len: usize,
    pub style: RunStyle,
    pub link: bool,
}

/// The inline's runs cut where links start and end, so each stretch is
/// wholly inside a link or wholly outside every one. Empty runs are
/// dropped; the lengths still add up to the text's.
pub fn spans(inline: &Inline) -> Vec<Span> {
    let mut out = Vec::with_capacity(inline.runs.len() + 2 * inline.links.len());
    let mut links = inline.links.iter().peekable();
    let mut at = 0;
    for &(len, style) in &inline.runs {
        let end = at + len;
        let mut pos = at;
        while pos < end {
            while links.peek().is_some_and(|link| link.range.end <= pos) {
                links.next();
            }
            // A link still ahead ends after `pos`, so each step moves on.
            let (link, next) = match links.peek() {
                Some(link) if link.range.start <= pos => (true, link.range.end.min(end)),
                Some(link) => (false, link.range.start.min(end)),
                None => (false, end),
            };
            out.push(Span { len: next - pos, style, link });
            pos = next;
        }
        at = end;
    }
    out
}

/// What a text's runs start from: the family, weight and style around it,
/// its colour, whether it is bold throughout (a heading, a table's header)
/// and the owner's code family, if it gave one.
#[derive(Clone, Debug)]
pub struct RunBase {
    pub font: Font,
    pub color: Hsla,
    pub bold: bool,
    pub code_family: Option<SharedString>,
}

/// The text runs that draw `inline` over `base` in the tokens' colours (see
/// the module doc for each style).
pub fn text_runs(inline: &Inline, base: &RunBase, t: &Tokens) -> Vec<TextRun> {
    spans(inline)
        .into_iter()
        .map(|span| {
            let style = span.style;
            let mut font = base.font.clone();
            if base.bold || style.strong {
                font.weight = FontWeight::BOLD;
            }
            if style.emphasis {
                font.style = FontStyle::Italic;
            }
            if style.code
                && let Some(family) = &base.code_family
            {
                font.family = family.clone();
            }
            let color = if span.link {
                t.link
            } else if style.code {
                t.foreground
            } else if style.muted {
                t.muted_foreground
            } else {
                base.color
            };
            TextRun {
                len: span.len,
                font,
                color,
                background_color: style.code.then_some(t.muted),
                underline: span.link.then_some(UnderlineStyle { thickness: px(1.), color: Some(t.link), wavy: false }),
                strikethrough: style.strike.then_some(StrikethroughStyle { thickness: px(1.), color: None }),
            }
        })
        .collect()
}

/// A list item's marker: a bullet, or the item's number counted from the
/// list's start. A bulleted task item has none (its task mark stands in
/// its place); a numbered one keeps its number before the mark.
pub fn list_marker(kind: ListKind, index: usize, task: Option<Task>) -> Option<SharedString> {
    match (kind, task) {
        (ListKind::Bullet, None) => Some("•".into()),
        (ListKind::Bullet, Some(_)) => None,
        (ListKind::Numbered { start }, _) => {
            let number = start.saturating_add(u64::try_from(index).unwrap_or(u64::MAX));
            Some(format!("{number}.").into())
        }
    }
}

/// Whether `held` holds every modifier in `wanted` (others may be held too).
/// No modifiers wanted is always held.
pub fn holds_modifiers(held: &Modifiers, wanted: &Modifiers) -> bool {
    (held.control || !wanted.control)
        && (held.alt || !wanted.alt)
        && (held.shift || !wanted.shift)
        && (held.platform || !wanted.platform)
        && (held.function || !wanted.function)
}

/// The tooltip of the link over the character at byte `index`, if a link
/// covers it and has one.
pub fn tooltip_at(links: &[InlineLink], index: usize) -> Option<&SharedString> {
    links.iter().find(|link| link.range.contains(&index)).and_then(|link| link.tooltip.as_ref())
}

/// A heading level's text size, when it isn't the size around it.
fn heading_size(level: HeadingLevel) -> Option<gpui::Rems> {
    match level {
        HeadingLevel::H1 => Some(rems(1.25)),
        HeadingLevel::H2 => Some(rems(1.125)),
        HeadingLevel::H3 => Some(rems(1.)),
        HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => None,
    }
}

fn text_align(align: ColumnAlign) -> TextAlign {
    match align {
        ColumnAlign::Start => TextAlign::Left,
        ColumnAlign::Center => TextAlign::Center,
        ColumnAlign::End => TextAlign::Right,
    }
}

/// What the owner is told of a press released on the same link.
type OnLink = Rc<dyn Fn(&InlineLink, Modifiers, &mut Window, &mut App) + 'static>;

/// A response: rich-text blocks the owner gives as plain data. See the
/// module doc.
#[derive(IntoElement)]
pub struct Response {
    id: ElementId,
    blocks: Vec<Arc<RichBlock>>,
    code_font: Option<SharedString>,
    link_modifiers: Modifiers,
    on_link: Option<OnLink>,
    on_copy_code: Option<OnCopy>,
}

impl Response {
    /// `id` is unique among its siblings (a list item's index, say).
    pub fn new(id: impl Into<ElementId>, blocks: impl IntoIterator<Item = Arc<RichBlock>>) -> Self {
        Self {
            id: id.into(),
            blocks: blocks.into_iter().collect(),
            code_font: None,
            link_modifiers: Modifiers::none(),
            on_link: None,
            on_copy_code: None,
        }
    }

    /// The family code is drawn in, inline and in blocks: the owner's
    /// monospace one. Without it, the family around the response.
    pub fn code_font(mut self, family: impl Into<SharedString>) -> Self {
        self.code_font = Some(family.into());
        self
    }

    /// The modifiers that make a press on a link act, for the pointer: a
    /// pointing hand shows over a link only while they are held. None by
    /// default, so the hand always shows.
    pub fn link_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.link_modifiers = modifiers;
        self
    }

    /// Called on a press and release on the same link, with the link and
    /// the modifiers held at the release.
    pub fn on_link(mut self, on_link: impl Fn(&InlineLink, Modifiers, &mut Window, &mut App) + 'static) -> Self {
        self.on_link = Some(Rc::new(on_link));
        self
    }

    /// Called with a code block's text on a press on its copy button.
    pub fn on_copy_code(mut self, on_copy: impl Fn(SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_copy_code = Some(Rc::new(on_copy));
        self
    }
}

/// The walk over a response's blocks, with what every block needs.
struct Draw<'a> {
    t: &'a Tokens,
    base: Font,
    /// The text colour of the block being drawn (muted inside a quote).
    color: Hsla,
    code_family: Option<SharedString>,
    line_height: Pixels,
    /// The link modifiers are held: links show the pointing hand.
    armed: bool,
    link_modifiers: Modifiers,
    on_link: Option<OnLink>,
    on_copy: Option<OnCopy>,
    /// Numbers the texts and code blocks, for their element ids.
    next: usize,
}

impl Draw<'_> {
    fn number(&mut self) -> usize {
        let n = self.next;
        self.next += 1;
        n
    }

    fn block(&mut self, block: &RichBlock) -> AnyElement {
        match block {
            RichBlock::Paragraph(text) => self.inline(text, false),
            RichBlock::Heading { level, text } => {
                div().when_some(heading_size(*level), |d, size| d.text_size(size)).child(self.inline(text, true)).into_any_element()
            }
            RichBlock::Code(code) => {
                let n = self.number();
                CodeBlock::new(("response-code", n), code.clone())
                    .when_some(self.code_family.clone(), |block, family| block.font(family))
                    .on_copy_shared(self.on_copy.clone())
                    .into_any_element()
            }
            RichBlock::List { kind, items } => self.list(*kind, items),
            RichBlock::Quote(blocks) => {
                let color = std::mem::replace(&mut self.color, self.t.muted_foreground);
                let children: Vec<AnyElement> = blocks.iter().map(|block| self.block(block)).collect();
                self.color = color;
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_l_2()
                    .border_color(self.t.border)
                    .pl_3()
                    .text_color(self.t.muted_foreground)
                    .children(children)
                    .into_any_element()
            }
            RichBlock::Table(table) => self.table(table),
            RichBlock::Rule => div().my_1().h(px(1.)).w_full().bg(self.t.border).into_any_element(),
        }
    }

    fn list(&mut self, kind: ListKind, items: &[ListItem]) -> AnyElement {
        let mut rows = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let marker = list_marker(kind, index, item.task).map(|marker| {
                let selector = marker.clone();
                div().debug_selector(move || format!("response-marker {selector}")).child(marker)
            });
            let mark = item.task.map(|task| self.task_mark(task));
            let blocks: Vec<AnyElement> = item.blocks.iter().map(|block| self.block(block)).collect();
            rows.push(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w(rems(0.75))
                            .h(self.line_height)
                            .children(marker)
                            .children(mark),
                    )
                    .child(div().flex_1().min_w_0().flex().flex_col().gap_2().children(blocks)),
            );
        }
        div().flex().flex_col().gap_1().children(rows).into_any_element()
    }

    /// A task's mark, drawn as a checkbox is but taking no focus or press.
    fn task_mark(&self, task: Task) -> AnyElement {
        let done = match task {
            Task::Open => false,
            Task::Done => true,
        };
        let style = checkbox_style(done, self.t);
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(CHECKBOX_TICK_SIZE)
            .rounded(CHECKBOX_RADIUS)
            .border_1()
            .border_color(style.border)
            .when_some(style.background, |d, background| d.bg(background))
            .when(done, |d| d.child(Icon::Check.size(rems(0.625)).color(style.tick)))
            .into_any_element()
    }

    fn table(&mut self, table: &Table) -> AnyElement {
        let width = table.width();
        let columns = u16::try_from(width).unwrap_or(u16::MAX);
        let last_row = table.rows().len();
        let mut cells = Vec::with_capacity(width * (last_row + 1));
        let rows = std::iter::once(table.head()).chain(table.rows().iter().map(Vec::as_slice));
        for (row, cells_of_row) in rows.enumerate() {
            for (column, (cell, align)) in cells_of_row.iter().zip(table.columns()).enumerate() {
                let text = self.inline(cell, row == 0);
                // A line under the header and between body rows.
                let ruled = row < last_row;
                cells.push(
                    div()
                        .debug_selector(move || format!("response-cell {row},{column}"))
                        .min_w_0()
                        .px_2()
                        .py_1()
                        .text_align(text_align(*align))
                        .when(ruled, |d| d.border_b_1().border_color(self.t.border))
                        .child(text),
                );
            }
        }
        div()
            .w_full()
            .min_w_0()
            .rounded(self.t.radius_md())
            .border_1()
            .border_color(self.t.border)
            .overflow_hidden()
            .child(div().grid().grid_cols(columns).w_full().children(cells))
            .into_any_element()
    }

    /// One text: styled runs, and links when it has any.
    fn inline(&mut self, inline: &Inline, bold: bool) -> AnyElement {
        let n = self.number();
        let base = RunBase { font: self.base.clone(), color: self.color, bold, code_family: self.code_family.clone() };
        let styled = StyledText::new(inline.text().clone()).with_runs(text_runs(inline, &base, self.t));
        let wrapper = div().debug_selector(move || format!("response-text {n}")).w_full().min_w_0();
        let has_tooltip = inline.links().iter().any(|link| link.tooltip.is_some());
        if inline.links().is_empty() || (self.on_link.is_none() && !has_tooltip) {
            return wrapper.child(styled).into_any_element();
        }
        let links: Rc<[InlineLink]> = inline.links().into();
        let mut text = InteractiveText::new(("response-link", n), styled);
        let clickable = self.on_link.clone().map(|on_link| {
            let ranges = links.iter().map(|link| link.range.clone()).collect();
            let links = links.clone();
            (ranges, move |ix: usize, window: &mut Window, cx: &mut App| {
                if let Some(link) = links.get(ix) {
                    let modifiers = window.modifiers();
                    on_link(link, modifiers, window, cx);
                }
            })
        });
        let guarded = clickable.is_some();
        if let Some((ranges, listener)) = clickable {
            text = text.on_click(ranges, listener);
        }
        if has_tooltip {
            text = text.tooltip(move |ix, window, cx| {
                tooltip_at(&links, ix).map(|tooltip| Tooltip::new(tooltip.clone()).build(window, cx))
            });
        }
        wrapper
            .relative()
            .child(text)
            .when(guarded, |d| d.child(pointer_guard(self.armed, self.link_modifiers)))
            .into_any_element()
    }
}

/// Over a text with links, after it: while the link modifiers aren't held
/// it sets the plain pointer, which wins over the text's pointing hand (the
/// later request on a hovered hitbox does), and a pointer move whose
/// modifiers change that redraws the window. Its hitbox blocks nothing.
fn pointer_guard(armed: bool, wanted: Modifiers) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |_, hitbox, window, _| {
            if !armed {
                window.set_cursor_style(CursorStyle::Arrow, &hitbox);
            }
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble
                    && hitbox.is_hovered(window)
                    && holds_modifiers(&event.modifiers, &wanted) != armed
                {
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl RenderOnce for Response {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = *cx.ui_theme();
        let style = window.text_style();
        let mut draw = Draw {
            t: &t,
            base: style.font(),
            color: style.color,
            code_family: self.code_font,
            line_height: window.line_height(),
            armed: holds_modifiers(&window.modifiers(), &self.link_modifiers),
            link_modifiers: self.link_modifiers,
            on_link: self.on_link,
            on_copy: self.on_copy_code,
            next: 0,
        };
        let blocks: Vec<AnyElement> = self.blocks.iter().map(|block| draw.block(block)).collect();
        div().id(self.id).flex().flex_col().gap_3().w_full().min_w_0().children(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> RunStyle {
        RunStyle::default()
    }

    fn strong() -> RunStyle {
        RunStyle { strong: true, ..RunStyle::default() }
    }

    fn link(range: Range<usize>) -> InlineLink {
        InlineLink { range, target: "https://example.com".into(), tooltip: None }
    }

    #[test]
    fn a_valid_inline_keeps_its_parts() {
        let inline = Inline::new(
            "see the docs",
            vec![(4, plain()), (3, strong()), (5, plain())],
            vec![link(8..12)],
        )
        .expect("valid");
        assert_eq!(inline.text().as_ref(), "see the docs");
        assert_eq!(inline.runs(), &[(4, plain()), (3, strong()), (5, plain())]);
        assert_eq!(inline.links(), &[link(8..12)]);
    }

    #[test]
    fn links_are_kept_in_order_of_their_starts() {
        let inline = Inline::new("ab cd", vec![(5, plain())], vec![link(3..5), link(0..2)]).expect("valid");
        assert_eq!(inline.links(), &[link(0..2), link(3..5)]);
    }

    #[test]
    fn empty_and_plain_inlines_are_valid() {
        assert_eq!(Inline::new("", Vec::new(), Vec::new()), Ok(Inline::default()));
        assert_eq!(Inline::plain(""), Inline::default());
        let plain_text = Inline::plain("héllo");
        assert_eq!(Inline::new("héllo", vec![(6, plain())], Vec::new()), Ok(plain_text));
    }

    #[test]
    fn runs_must_cover_the_text() {
        assert_eq!(
            Inline::new("hello", vec![(3, plain())], Vec::new()),
            Err(InlineError::RunsDontCoverText { runs: 3, text: 5 })
        );
        assert_eq!(
            Inline::new("hello", vec![(3, plain()), (3, strong())], Vec::new()),
            Err(InlineError::RunsDontCoverText { runs: 6, text: 5 })
        );
        assert_eq!(
            Inline::new("hi", vec![(usize::MAX, plain()), (2, plain())], Vec::new()),
            Err(InlineError::RunsDontCoverText { runs: usize::MAX, text: 2 })
        );
    }

    #[test]
    fn a_run_must_end_on_a_char_boundary() {
        // "é" is two bytes: a run of 2 ends inside it.
        assert_eq!(
            Inline::new("hé", vec![(2, plain()), (1, strong())], Vec::new()),
            Err(InlineError::NotCharBoundary { at: 2 })
        );
    }

    #[test]
    fn a_link_must_lie_on_char_boundaries() {
        assert_eq!(
            Inline::new("hé!", vec![(4, plain())], vec![link(0..2)]),
            Err(InlineError::NotCharBoundary { at: 2 })
        );
        assert_eq!(
            Inline::new("hé!", vec![(4, plain())], vec![link(2..4)]),
            Err(InlineError::NotCharBoundary { at: 2 })
        );
    }

    #[test]
    fn a_link_must_lie_inside_the_text() {
        assert_eq!(
            Inline::new("hello", vec![(5, plain())], vec![link(3..6)]),
            Err(InlineError::LinkOutOfRange { range: 3..6, text: 5 })
        );
        let backwards = Range { start: 4, end: 2 };
        assert_eq!(
            Inline::new("hello", vec![(5, plain())], vec![link(backwards.clone())]),
            Err(InlineError::LinkOutOfRange { range: backwards, text: 5 })
        );
    }

    #[test]
    fn links_must_not_overlap() {
        assert_eq!(
            Inline::new("hello world", vec![(11, plain())], vec![link(6..11), link(0..7)]),
            Err(InlineError::LinksOverlap { first: 0..7, second: 6..11 })
        );
        // Touching is not overlapping.
        assert!(Inline::new("helloworld", vec![(10, plain())], vec![link(0..5), link(5..10)]).is_ok());
    }

    #[test]
    fn errors_read_as_sentences() {
        let error = InlineError::LinksOverlap { first: 0..7, second: 6..11 };
        assert_eq!(error.to_string(), "links 0..7 and 6..11 overlap");
    }

    fn cell(text: &'static str) -> Inline {
        Inline::plain(text)
    }

    #[test]
    fn a_short_row_is_padded_with_empty_cells() {
        let table = Table::new(
            vec![ColumnAlign::Start, ColumnAlign::Center, ColumnAlign::End],
            vec![cell("a"), cell("b"), cell("c")],
            vec![vec![cell("1"), cell("2")]],
        );
        assert_eq!(table.width(), 3);
        assert_eq!(table.rows(), &[vec![cell("1"), cell("2"), Inline::default()]]);
        assert_eq!(table.columns(), &[ColumnAlign::Start, ColumnAlign::Center, ColumnAlign::End]);
    }

    #[test]
    fn cells_past_the_header_are_dropped() {
        let table = Table::new(
            vec![ColumnAlign::End, ColumnAlign::End, ColumnAlign::Center],
            vec![cell("a"), cell("b")],
            vec![vec![cell("1"), cell("2"), cell("3")], vec![cell("4")]],
        );
        assert_eq!(table.head(), &[cell("a"), cell("b")]);
        assert_eq!(table.rows(), &[vec![cell("1"), cell("2")], vec![cell("4"), Inline::default()]]);
        assert_eq!(table.columns(), &[ColumnAlign::End, ColumnAlign::End]);
    }

    #[test]
    fn missing_alignments_are_start() {
        let table = Table::new(Vec::new(), vec![cell("a"), cell("b")], Vec::new());
        assert_eq!(table.columns(), &[ColumnAlign::Start, ColumnAlign::Start]);
    }

    // ---- drawing, pure ---------------------------------------------------------

    fn code() -> RunStyle {
        RunStyle { code: true, ..RunStyle::default() }
    }

    fn span(len: usize, style: RunStyle, link: bool) -> Span {
        Span { len, style, link }
    }

    #[test]
    fn spans_cut_runs_where_links_start_and_end() {
        // "see the docs now": a strong run over "the docs", a link over
        // "docs now" crossing the run's end.
        let inline = Inline::new(
            "see the docs now",
            vec![(4, plain()), (8, strong()), (0, plain()), (4, plain())],
            vec![link(8..16)],
        )
        .expect("valid");
        assert_eq!(
            spans(&inline),
            [span(4, plain(), false), span(4, strong(), false), span(4, strong(), true), span(4, plain(), true)]
        );
        // Two links in one run, apart and touching.
        let inline = Inline::new("abcdef", vec![(6, plain())], vec![link(0..2), link(2..3), link(4..6)]).expect("valid");
        assert_eq!(
            spans(&inline),
            [span(2, plain(), true), span(1, plain(), true), span(1, plain(), false), span(2, plain(), true)]
        );
        // An empty link takes nothing; no runs, no spans.
        let inline = Inline::new("ab", vec![(2, plain())], vec![link(1..1)]).expect("valid");
        assert_eq!(spans(&inline), [span(1, plain(), false), span(1, plain(), false)]);
        assert_eq!(spans(&Inline::default()), []);
    }

    fn tokens() -> Tokens {
        Tokens::neutral_light()
    }

    fn base(bold: bool) -> RunBase {
        RunBase { font: gpui::font("Sans"), color: gpui::red(), bold, code_family: Some("Mono".into()) }
    }

    #[test]
    fn runs_take_each_style() {
        let t = tokens();
        let emphasis = RunStyle { emphasis: true, ..RunStyle::default() };
        let strike = RunStyle { strike: true, ..RunStyle::default() };
        let muted = RunStyle { muted: true, ..RunStyle::default() };
        let inline = Inline::new(
            "abcdef",
            vec![(1, plain()), (1, strong()), (1, emphasis), (1, strike), (1, code()), (1, muted)],
            Vec::new(),
        )
        .expect("valid");
        let runs = text_runs(&inline, &base(false), &t);
        assert_eq!(runs.iter().map(|run| run.len).collect::<Vec<_>>(), [1; 6]);
        let [plain_run, strong_run, emphasis_run, strike_run, code_run, muted_run] = runs.as_slice() else {
            panic!("six runs");
        };
        assert_eq!((plain_run.font.weight, plain_run.color, plain_run.background_color), (FontWeight::NORMAL, gpui::red(), None));
        assert_eq!(strong_run.font.weight, FontWeight::BOLD);
        assert_eq!(emphasis_run.font.style, FontStyle::Italic);
        assert!(strike_run.strikethrough.is_some() && plain_run.strikethrough.is_none());
        assert_eq!(
            (code_run.font.family.as_ref(), code_run.color, code_run.background_color),
            ("Mono", t.foreground, Some(t.muted))
        );
        assert_eq!(plain_run.font.family.as_ref(), "Sans");
        assert_eq!(muted_run.color, t.muted_foreground);
        assert!(runs.iter().all(|run| run.underline.is_none()));
    }

    #[test]
    fn links_are_the_link_colour_underlined_and_bold_text_bold_throughout() {
        let t = tokens();
        let inline = Inline::new("a link", vec![(6, plain())], vec![link(2..6)]).expect("valid");
        let runs = text_runs(&inline, &base(true), &t);
        assert_eq!(runs.len(), 2);
        assert_eq!((runs[0].color, runs[0].underline), (gpui::red(), None));
        assert_eq!(runs[1].color, t.link);
        assert_eq!(runs[1].underline.and_then(|underline| underline.color), Some(t.link));
        assert!(runs.iter().all(|run| run.font.weight == FontWeight::BOLD));
        // Without a code family, code keeps the family around it.
        let inline = Inline::new("x", vec![(1, code())], Vec::new()).expect("valid");
        let runs = text_runs(&inline, &RunBase { code_family: None, ..base(false) }, &t);
        assert_eq!(runs[0].font.family.as_ref(), "Sans");
    }

    #[test]
    fn markers_are_a_bullet_or_the_number_from_the_start() {
        let numbered = ListKind::Numbered { start: 3 };
        let marker = |kind, index, task| list_marker(kind, index, task).map(|m| m.to_string());
        assert_eq!(marker(ListKind::Bullet, 0, None).as_deref(), Some("•"));
        assert_eq!(marker(ListKind::Bullet, 0, Some(Task::Open)), None);
        assert_eq!(marker(numbered, 0, None).as_deref(), Some("3."));
        assert_eq!(marker(numbered, 1, Some(Task::Done)).as_deref(), Some("4."));
        assert_eq!(marker(ListKind::Numbered { start: u64::MAX }, 2, None), Some(format!("{}.", u64::MAX)));
    }

    #[test]
    fn link_modifiers_are_held_when_each_is() {
        let ctrl = Modifiers::control();
        assert!(holds_modifiers(&ctrl, &ctrl));
        assert!(holds_modifiers(&Modifiers { shift: true, ..ctrl }, &ctrl));
        assert!(!holds_modifiers(&Modifiers::none(), &ctrl));
        assert!(!holds_modifiers(&Modifiers::alt(), &ctrl));
        // None wanted: always held.
        assert!(holds_modifiers(&Modifiers::none(), &Modifiers::none()));
        assert!(holds_modifiers(&ctrl, &Modifiers::none()));
    }

    #[test]
    fn a_tooltip_is_the_covering_links() {
        let links = [
            InlineLink { range: 0..4, target: "https://a".into(), tooltip: Some("https://a".into()) },
            InlineLink { range: 5..9, target: "https://b".into(), tooltip: None },
        ];
        assert_eq!(tooltip_at(&links, 0).map(|t| t.as_ref()), Some("https://a"));
        assert_eq!(tooltip_at(&links, 3).map(|t| t.as_ref()), Some("https://a"));
        assert_eq!(tooltip_at(&links, 4), None);
        assert_eq!(tooltip_at(&links, 6), None);
    }

    // ---- drawing, in a window ----------------------------------------------------

    use std::cell::RefCell;
    use std::time::Duration;

    use gpui::{Bounds, Context, MouseButton, Render, TestAppContext, VisualTestContext, point};

    use crate::test_support;

    fn paragraph(inline: Inline) -> Arc<RichBlock> {
        Arc::new(RichBlock::Paragraph(inline))
    }

    /// What a host's response told it, as `target modifiers`.
    type Log = Rc<RefCell<Vec<(SharedString, Modifiers)>>>;

    struct Host {
        blocks: Vec<Arc<RichBlock>>,
        log: Log,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let log = self.log.clone();
            div().w(px(400.)).child(
                Response::new("response", self.blocks.iter().cloned())
                    .code_font("Mono")
                    .link_modifiers(Modifiers::control())
                    .on_link(move |link, modifiers, _, _| log.borrow_mut().push((link.target.clone(), modifiers))),
            )
        }
    }

    fn open(blocks: Vec<Arc<RichBlock>>) -> (Log, &'static mut VisualTestContext) {
        let app: &'static mut TestAppContext = Box::leak(Box::new(test_support::app()));
        let log: Log = Rc::default();
        let held = log.clone();
        let (_, cx) = app.add_window_view(move |_, _| Host { blocks, log: held });
        cx.run_until_parked();
        (log, cx)
    }

    /// The middle of the character at byte `at` of the one-line text drawn
    /// as `selector`. gpui's test text system draws every character 0.6 of
    /// the font size wide.
    fn over(cx: &mut VisualTestContext, selector: &'static str, text: &str, at: usize) -> gpui::Point<Pixels> {
        let bounds = cx.debug_bounds(selector).expect("drawn");
        let advance = cx.update(|window, _| window.text_style().font_size.to_pixels(window.rem_size())) * 0.6;
        let chars = text[..at].chars().count() as f32;
        point(bounds.left() + advance * (chars + 0.5), bounds.top() + px(4.))
    }

    /// The spec's "Styled runs": plain, strong and code runs stay on one
    /// line, as tall as a plain line.
    #[test]
    fn styled_runs_stay_on_one_line() {
        let styled = Inline::new("plain bold code", vec![(6, plain()), (5, strong()), (4, code())], Vec::new())
            .expect("valid");
        let (_, cx) = open(vec![paragraph(styled), paragraph(Inline::plain("x"))]);
        let styled = cx.debug_bounds("response-text 0").expect("drawn");
        let plain_line = cx.debug_bounds("response-text 1").expect("drawn");
        assert_eq!(styled.size.height, plain_line.size.height);
        assert!(styled.top() < plain_line.top());
    }

    /// The spec's "Nested list": a numbered list from 3 numbers its items 3
    /// and 4, and the bulleted list in item 4 sits under it, indented.
    #[test]
    fn a_numbered_list_counts_from_its_start_and_indents_what_it_holds() {
        let nested = RichBlock::List {
            kind: ListKind::Bullet,
            items: vec![ListItem { task: None, blocks: vec![RichBlock::Paragraph(Inline::plain("inner"))] }],
        };
        let list = RichBlock::List {
            kind: ListKind::Numbered { start: 3 },
            items: vec![
                ListItem { task: None, blocks: vec![RichBlock::Paragraph(Inline::plain("three"))] },
                ListItem {
                    task: None,
                    blocks: vec![RichBlock::Paragraph(Inline::plain("four")), nested],
                },
            ],
        };
        let (_, cx) = open(vec![Arc::new(list)]);
        let three = cx.debug_bounds("response-marker 3.").expect("item 3");
        let four = cx.debug_bounds("response-marker 4.").expect("item 4");
        let bullet = cx.debug_bounds("response-marker •").expect("the nested bullet");
        assert!(three.top() < four.top(), "{three:?} {four:?}");
        assert!(bullet.top() > four.top(), "the nested list isn't under item 4: {bullet:?} {four:?}");
        assert!(bullet.left() > four.left(), "the nested list isn't indented: {bullet:?} {four:?}");
        assert!(cx.debug_bounds("response-marker 1.").is_none());
    }

    fn linked() -> Arc<RichBlock> {
        // "go to docs or home": "docs" links with a tooltip, "home" without.
        let inline = Inline::new(
            "go to docs or home",
            vec![(18, plain())],
            vec![
                InlineLink { range: 6..10, target: "https://docs".into(), tooltip: Some("https://docs".into()) },
                InlineLink { range: 14..18, target: "https://home".into(), tooltip: None },
            ],
        )
        .expect("valid");
        paragraph(inline)
    }

    const LINKED: &str = "go to docs or home";

    /// The spec's "Press on a link": a press and release on it with ctrl
    /// held tells the owner which link, with ctrl; a plain one tells it too,
    /// without.
    #[test]
    fn a_press_and_release_on_a_link_reports_it_with_the_modifiers() {
        let (log, cx) = open(vec![linked()]);
        let docs = over(cx, "response-text 0", LINKED, 7);
        cx.simulate_click(docs, Modifiers::control());
        cx.run_until_parked();
        assert_eq!(log.borrow().as_slice(), &[(SharedString::from("https://docs"), Modifiers::control())]);
        let home = over(cx, "response-text 0", LINKED, 15);
        cx.simulate_click(home, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(log.borrow().len(), 2);
        assert_eq!(log.borrow()[1], (SharedString::from("https://home"), Modifiers::none()));
    }

    /// The spec's "Release elsewhere": a press on a link released off it
    /// (on plain text, or on the other link) reports nothing.
    #[test]
    fn a_release_off_the_link_reports_nothing() {
        let (log, cx) = open(vec![linked()]);
        let docs = over(cx, "response-text 0", LINKED, 7);
        for elsewhere in [over(cx, "response-text 0", LINKED, 1), over(cx, "response-text 0", LINKED, 15)] {
            cx.simulate_mouse_down(docs, MouseButton::Left, Modifiers::control());
            cx.run_until_parked();
            cx.simulate_mouse_move(elsewhere, MouseButton::Left, Modifiers::control());
            cx.simulate_mouse_up(elsewhere, MouseButton::Left, Modifiers::control());
            cx.run_until_parked();
        }
        assert_eq!(log.borrow().as_slice(), &[]);
    }

    /// Whether a tooltip shows once the pointer has rested at `at`.
    fn tooltip_shown_at(cx: &mut VisualTestContext, at: gpui::Point<Pixels>) -> bool {
        cx.simulate_mouse_move(point(px(390.), px(300.)), None, Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(at, None, Modifiers::none());
        cx.executor().advance_clock(Duration::from_millis(800));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.debug_bounds("tooltip").is_some()
    }

    /// A link with a tooltip shows it on hover; one without builds none.
    #[test]
    fn only_a_link_with_a_tooltip_shows_one() {
        let (_, cx) = open(vec![linked()]);
        let home = over(cx, "response-text 0", LINKED, 15);
        let docs = over(cx, "response-text 0", LINKED, 7);
        assert!(!tooltip_shown_at(cx, home), "a link without a tooltip showed one");
        assert!(tooltip_shown_at(cx, docs), "the link's tooltip didn't show");
    }

    /// The spec's "Short row": a three-column table whose row has two cells
    /// keeps three equal columns, the third cell empty.
    #[test]
    fn a_short_row_keeps_the_columns_and_an_empty_cell() {
        let table = Table::new(
            vec![ColumnAlign::Start, ColumnAlign::Center, ColumnAlign::End],
            vec![cell("name"), cell("a much longer header"), cell("n")],
            vec![vec![cell("x"), cell("y")]],
        );
        let (_, cx) = open(vec![Arc::new(RichBlock::Table(table))]);
        let cells: Vec<Vec<Bounds<Pixels>>> = (0..2)
            .map(|row| {
                (0..3)
                    .map(|column| {
                        let selector: &'static str = Box::leak(format!("response-cell {row},{column}").into_boxed_str());
                        cx.debug_bounds(selector).unwrap_or_else(|| panic!("no cell {row},{column}"))
                    })
                    .collect()
            })
            .collect();
        let widths: Vec<Pixels> = cells[0].iter().map(|bounds| bounds.size.width).collect();
        assert!(widths.iter().all(|width| (*width - widths[0]).abs() <= px(0.5)), "{widths:?}");
        for (head, body) in cells[0].iter().zip(&cells[1]) {
            assert_eq!(body.left(), head.left());
            assert!(body.top() >= head.bottom());
        }
        // The padded cell is there, as tall as its row.
        assert_eq!(cells[1][2].size.height, cells[1][0].size.height);
    }
}
