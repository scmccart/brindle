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

use std::fmt;
use std::ops::Range;

use gpui::SharedString;

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
}
