//! Markdown in agent messages and thoughts (`agent-zones`, design.md
//! decisions 4, 7 and 11): the source rendered as `brindle-ui`'s rich-text
//! data, one [`Part`] per top-level block; what a press on a link does
//! ([`link_action`]); and the dump's words for a part.
//!
//! The app's one door to `pulldown-cmark`: no other file names it, so a
//! parser swap touches only this one. Pure: plain values in and out, no IO,
//! clock or environment (`fcis R1`–`R3`). The source is another process's
//! text, so nothing here panics on it: every slice is on a range the parser
//! gave or a char boundary checked first, nesting and table width are
//! bounded ([`MAX_DEPTH`], [`MAX_COLUMNS`]), and an [`Inline`] the walk
//! somehow builds wrong falls back to its plain text (`railway R5`).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use brindle_ui::{
    Code, ColumnAlign, HeadingLevel, Inline, InlineLink, ListItem, ListKind, RichBlock, RunStyle, Table, Task,
};
use gpui::SharedString;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::terminal::links::{self, Plan, Refusal};

/// How deep lists and quotes nest before deeper ones are kept as the
/// paragraphs of their text at this depth.
pub const MAX_DEPTH: usize = 16;

/// The widest table drawn as a grid; a wider one shows as a code block of
/// its source.
pub const MAX_COLUMNS: usize = 32;

/// How much of an image's target its text shows.
const IMAGE_TARGET_CHARS: usize = 120;

/// CommonMark plus GitHub's strikethrough, task lists and tables, nothing
/// else: footnotes, math, smart punctuation, heading attributes, metadata
/// blocks and `> [!NOTE]` tags stay text as written.
fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES
}

/// One top-level block of a message: its range in the source and what it
/// draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub range: Range<usize>,
    pub block: Arc<RichBlock>,
}

/// `source` as Markdown, one part per top-level block in order. Source
/// with no block (empty, blank lines, only a link reference definition) is
/// one empty paragraph covering it, so a message that has started shows.
pub fn render(source: &str) -> Vec<Part> {
    let mut walk = Walk::new(source);
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        walk.event(event, range);
    }
    walk.finish()
}

/// Inline text being built: its runs, its links, and the byte ranges of
/// plain text (not code, not in a link or an image) a bare URL may be
/// found in.
#[derive(Default)]
struct InlineBuf {
    text: String,
    runs: Vec<(usize, RunStyle)>,
    links: Vec<InlineLink>,
    plain: Vec<Range<usize>>,
}

impl InlineBuf {
    fn push(&mut self, text: &str, style: RunStyle, plain: bool) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        match self.runs.last_mut() {
            Some((len, last)) if *last == style => *len += text.len(),
            Some(_) | None => self.runs.push((text.len(), style)),
        }
        if plain {
            let end = self.text.len();
            match self.plain.last_mut() {
                Some(range) if range.end == start => range.end = end,
                Some(_) | None => self.plain.push(start..end),
            }
        }
    }

    /// The inline, with bare URLs in its plain text linked.
    fn finish(mut self) -> Inline {
        for range in std::mem::take(&mut self.plain) {
            let Some(slice) = self.text.get(range.clone()) else { continue };
            // Every scheme ends in a colon: most text has none to look at.
            if !slice.contains(':') {
                continue;
            }
            let starts: Vec<usize> = slice.char_indices().map(|(at, _)| at).collect();
            let chars: Vec<char> = slice.chars().collect();
            for url in links::find_urls(&chars) {
                let Some(&from) = starts.get(url.start) else { continue };
                let to = starts.get(url.end).copied().unwrap_or(slice.len());
                let Some(target) = slice.get(from..to) else { continue };
                let link = InlineLink {
                    range: range.start + from..range.start + to,
                    target: target.to_string().into(),
                    tooltip: None,
                };
                self.links.push(link);
            }
        }
        let text = SharedString::from(self.text);
        match Inline::new(text.clone(), self.runs, self.links) {
            Ok(inline) => inline,
            // A bug in the walk, never the text's fault: the tests' corpus
            // would find it, and a user sees the text unstyled.
            Err(error) => {
                if cfg!(test) {
                    panic!("the walk built an invalid inline: {error}");
                }
                Inline::plain(text)
            }
        }
    }
}

/// A leaf block being built.
enum Leaf {
    Paragraph(InlineBuf),
    Heading(HeadingLevel, InlineBuf),
    Code { label: Option<SharedString>, text: String },
    Html(String),
    Table(TableBuf),
}

#[derive(Default)]
struct TableBuf {
    columns: Vec<ColumnAlign>,
    head: Vec<Inline>,
    rows: Vec<Vec<Inline>>,
    row: Vec<Inline>,
    cell: Option<InlineBuf>,
}

/// A container being built.
enum Frame {
    Quote(Vec<RichBlock>),
    List { kind: ListKind, items: Vec<ListItem> },
    Item { task: Option<Task>, blocks: Vec<RichBlock> },
}

/// A link whose text is being built: where it starts in the inline, and
/// its target.
struct OpenLink {
    start: usize,
    target: String,
}

/// An image whose alt text is being collected.
struct OpenImage {
    target: String,
    alt: String,
    depth: usize,
}

/// The walk over the parser's events.
struct Walk<'a> {
    source: &'a str,
    parts: Vec<Part>,
    /// The range of the top-level block being built.
    top: Option<Range<usize>>,
    /// The last event's range, for content outside any block.
    last: Range<usize>,
    frames: Vec<Frame>,
    leaf: Option<Leaf>,
    /// Lists, quotes and items past [`MAX_DEPTH`] being skipped.
    skipped: usize,
    /// Inside a table past [`MAX_COLUMNS`]: its nesting.
    wide_table: usize,
    strong: usize,
    emphasis: usize,
    strike: usize,
    link: Option<OpenLink>,
    /// Links opened inside a link (or an image), kept as text.
    inner_links: usize,
    image: Option<OpenImage>,
    /// Where text goes that has no inline to go to (never, from the
    /// parser): dropped rather than a panic.
    spare: InlineBuf,
    /// Each link target made so far, so a reference used many times holds
    /// one (OpenSpec change `agents-absorb-cost`, decision 8).
    targets: HashMap<String, SharedString>,
}

fn heading_level(level: pulldown_cmark::HeadingLevel) -> HeadingLevel {
    match level {
        pulldown_cmark::HeadingLevel::H1 => HeadingLevel::H1,
        pulldown_cmark::HeadingLevel::H2 => HeadingLevel::H2,
        pulldown_cmark::HeadingLevel::H3 => HeadingLevel::H3,
        pulldown_cmark::HeadingLevel::H4 => HeadingLevel::H4,
        pulldown_cmark::HeadingLevel::H5 => HeadingLevel::H5,
        pulldown_cmark::HeadingLevel::H6 => HeadingLevel::H6,
    }
}

fn column_align(align: &Alignment) -> ColumnAlign {
    match align {
        Alignment::None | Alignment::Left => ColumnAlign::Start,
        Alignment::Center => ColumnAlign::Center,
        Alignment::Right => ColumnAlign::End,
    }
}

/// A link's target: an e-mail autolink's address gets `mailto:`, as
/// CommonMark's HTML does.
fn link_target(link_type: LinkType, dest: &str) -> String {
    match link_type {
        LinkType::Email => format!("mailto:{dest}"),
        LinkType::Inline
        | LinkType::Reference
        | LinkType::ReferenceUnknown
        | LinkType::Collapsed
        | LinkType::CollapsedUnknown
        | LinkType::Shortcut
        | LinkType::ShortcutUnknown
        | LinkType::Autolink
        | LinkType::WikiLink { .. } => dest.to_string(),
    }
}

/// An image's text: `[image: ALT]`, a space and the target, cut to its
/// first [`IMAGE_TARGET_CHARS`] characters with `…` after.
fn image_text(alt: &str, target: &str) -> String {
    let mut shown: String = target.chars().take(IMAGE_TARGET_CHARS).collect();
    if target.chars().nth(IMAGE_TARGET_CHARS).is_some() {
        shown.push('…');
    }
    format!("[image: {alt}] {shown}")
}

impl<'a> Walk<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            parts: Vec::new(),
            top: None,
            last: 0..0,
            frames: Vec::new(),
            leaf: None,
            skipped: 0,
            wide_table: 0,
            strong: 0,
            emphasis: 0,
            strike: 0,
            link: None,
            inner_links: 0,
            image: None,
            spare: InlineBuf::default(),
            targets: HashMap::new(),
        }
    }

    /// `target` as this render's one string for it.
    fn shared(&mut self, target: String) -> SharedString {
        if let Some(shared) = self.targets.get(&target) {
            return shared.clone();
        }
        let shared = SharedString::from(target.clone());
        self.targets.insert(target, shared.clone());
        shared
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        self.last = range.clone();
        if self.wide_table > 0 {
            match event {
                Event::Start(Tag::Table(_)) => self.wide_table += 1,
                Event::End(TagEnd::Table) => self.wide_table -= 1,
                Event::Start(_)
                | Event::End(_)
                | Event::Text(_)
                | Event::Code(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Rule
                | Event::TaskListMarker(_) => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(end) => self.end(end),
            Event::Text(text) => self.text(&text, false),
            Event::Code(text) => self.text(&text, true),
            // Never produced under these options; shown as written.
            Event::InlineMath(_) | Event::DisplayMath(_) | Event::FootnoteReference(_) => {
                let written = self.source.get(range).unwrap_or_default();
                self.inline(written, self.style(), false);
            }
            Event::Html(html) => match &mut self.leaf {
                Some(Leaf::Html(text)) => text.push_str(&html),
                Some(Leaf::Paragraph(_) | Leaf::Heading(..) | Leaf::Code { .. } | Leaf::Table(_)) | None => {
                    self.inline(&html, self.style(), false)
                }
            },
            Event::InlineHtml(html) => self.inline(&html, self.style(), false),
            Event::SoftBreak => self.inline(" ", self.style(), true),
            Event::HardBreak => self.inline("\n", self.style(), true),
            Event::Rule => {
                self.begin_block(range);
                self.emit(RichBlock::Rule);
            }
            Event::TaskListMarker(done) => {
                if self.skipped == 0
                    && let Some(Frame::Item { task, .. }) = self.frames.last_mut()
                {
                    *task = Some(if done { Task::Done } else { Task::Open });
                }
            }
        }
    }

    fn style(&self) -> RunStyle {
        RunStyle { strong: self.strong > 0, emphasis: self.emphasis > 0, strike: self.strike > 0, ..RunStyle::default() }
    }

    fn depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| match frame {
                Frame::Quote(_) | Frame::List { .. } => true,
                Frame::Item { .. } => false,
            })
            .count()
    }

    /// A block starts: an open implicit paragraph (a tight item's text)
    /// ends, and at the top level it starts a part.
    fn begin_block(&mut self, range: Range<usize>) {
        self.close_leaf();
        if self.frames.is_empty() && self.top.is_none() {
            self.top = Some(range);
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => {
                self.begin_block(range);
                self.leaf = Some(Leaf::Paragraph(InlineBuf::default()));
            }
            Tag::Heading { level, .. } => {
                self.begin_block(range);
                self.leaf = Some(Leaf::Heading(heading_level(level), InlineBuf::default()));
            }
            Tag::CodeBlock(kind) => {
                self.begin_block(range);
                let label = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(|word| word.to_string().into()),
                    CodeBlockKind::Indented => None,
                };
                self.leaf = Some(Leaf::Code { label, text: String::new() });
            }
            Tag::HtmlBlock => {
                self.begin_block(range);
                self.leaf = Some(Leaf::Html(String::new()));
            }
            Tag::BlockQuote(_) => {
                self.begin_block(range);
                self.open_container(Frame::Quote(Vec::new()));
            }
            Tag::List(start) => {
                self.begin_block(range);
                let kind = match start {
                    Some(start) => ListKind::Numbered { start },
                    None => ListKind::Bullet,
                };
                self.open_container(Frame::List { kind, items: Vec::new() });
            }
            Tag::Item => {
                self.close_leaf();
                if self.skipped > 0 {
                    self.skipped += 1;
                } else {
                    self.frames.push(Frame::Item { task: None, blocks: Vec::new() });
                }
            }
            Tag::Table(aligns) => {
                self.begin_block(range.clone());
                if aligns.len() > MAX_COLUMNS {
                    let text = self.source.get(range).unwrap_or_default().to_string();
                    self.emit(RichBlock::Code(Code { label: None, text: text.into() }));
                    self.wide_table = 1;
                } else {
                    let columns = aligns.iter().map(column_align).collect();
                    self.leaf = Some(Leaf::Table(TableBuf { columns, ..TableBuf::default() }));
                }
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(Leaf::Table(table)) = &mut self.leaf {
                    table.row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(Leaf::Table(table)) = &mut self.leaf {
                    table.cell = Some(InlineBuf::default());
                }
            }
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { link_type, dest_url, .. } => {
                if self.link.is_some() || self.image.is_some() {
                    self.inner_links += 1;
                } else {
                    let start = self.inline_len();
                    self.link = Some(OpenLink { start, target: link_target(link_type, &dest_url) });
                }
            }
            Tag::Image { link_type, dest_url, .. } => match &mut self.image {
                Some(image) => image.depth += 1,
                None => {
                    let target = link_target(link_type, &dest_url);
                    self.image = Some(OpenImage { target, alt: String::new(), depth: 1 });
                }
            },
            // Never produced under these options: their content flows into
            // the block around them.
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => {}
        }
    }

    /// Opens a list or quote, or skips it past [`MAX_DEPTH`].
    fn open_container(&mut self, frame: Frame) {
        if self.skipped > 0 || self.depth() >= MAX_DEPTH {
            self.skipped += 1;
        } else {
            self.frames.push(frame);
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::Table => {
                self.close_leaf()
            }
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => {
                self.close_leaf();
                if self.skipped > 0 {
                    self.skipped -= 1;
                } else if let Some(frame) = self.frames.pop() {
                    self.close_frame(frame);
                }
            }
            TagEnd::TableHead => {
                if let Some(Leaf::Table(table)) = &mut self.leaf {
                    table.head = std::mem::take(&mut table.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(Leaf::Table(table)) = &mut self.leaf {
                    let row = std::mem::take(&mut table.row);
                    table.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                if let Some(Leaf::Table(table)) = &mut self.leaf {
                    let cell = table.cell.take().unwrap_or_default();
                    table.row.push(cell.finish());
                }
            }
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                if self.inner_links > 0 {
                    self.inner_links -= 1;
                } else if let Some(link) = self.link.take() {
                    self.close_link(link);
                }
            }
            TagEnd::Image => {
                let done = match &mut self.image {
                    Some(image) => {
                        image.depth -= 1;
                        image.depth == 0
                    }
                    None => false,
                };
                if done && let Some(image) = self.image.take() {
                    self.close_image(image);
                }
            }
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn close_link(&mut self, link: OpenLink) {
        let mut start = link.start;
        if self.inline_len() == start {
            // `[](url)`: the target is its text, so there is something to
            // press.
            self.inline(&link.target, self.style(), false);
        }
        let buf = self.buf();
        let end = buf.text.len();
        if start > end {
            start = end;
        }
        let differs = buf.text.get(start..end).unwrap_or_default() != link.target;
        let target = self.shared(link.target);
        let tooltip = differs.then(|| target.clone());
        self.buf().links.push(InlineLink { range: start..end, target, tooltip });
    }

    fn close_image(&mut self, image: OpenImage) {
        let text = image_text(&image.alt, &image.target);
        let style = RunStyle { muted: true, ..self.style() };
        let linked = self.link.is_none();
        let target = self.shared(image.target);
        let buf = self.buf();
        let start = buf.text.len();
        buf.push(&text, style, false);
        if linked {
            let end = buf.text.len();
            buf.links.push(InlineLink { range: start..end, target: target.clone(), tooltip: Some(target) });
        }
    }

    /// Text in the current leaf: a code block's or an HTML block's as it
    /// is, else a run (`code` for a code span).
    fn text(&mut self, text: &str, code: bool) {
        match &mut self.leaf {
            Some(Leaf::Code { text: own, .. }) | Some(Leaf::Html(own)) if !code => own.push_str(text),
            Some(Leaf::Code { .. } | Leaf::Html(_) | Leaf::Paragraph(_) | Leaf::Heading(..) | Leaf::Table(_)) | None => {
                let style = RunStyle { code, ..self.style() };
                self.inline(text, style, !code);
            }
        }
    }

    /// A run in the current inline: an image's alt text while one is open,
    /// else the leaf's text (a paragraph opened for it when there is none).
    fn inline(&mut self, text: &str, style: RunStyle, plain: bool) {
        if let Some(image) = &mut self.image {
            image.alt.push_str(text);
            return;
        }
        let plain = plain && self.link.is_none();
        self.buf().push(text, style, plain);
    }

    fn inline_len(&self) -> usize {
        match &self.leaf {
            Some(Leaf::Paragraph(buf) | Leaf::Heading(_, buf)) => buf.text.len(),
            Some(Leaf::Table(table)) => table.cell.as_ref().map_or(0, |cell| cell.text.len()),
            Some(Leaf::Code { .. } | Leaf::Html(_)) | None => 0,
        }
    }

    /// The inline being built, a paragraph opened when the leaf has none
    /// (a tight list item's text) or isn't inline (never, from the parser).
    fn buf(&mut self) -> &mut InlineBuf {
        let inline = match &self.leaf {
            Some(Leaf::Paragraph(_) | Leaf::Heading(..) | Leaf::Table(_)) => true,
            Some(Leaf::Code { .. } | Leaf::Html(_)) | None => false,
        };
        if !inline {
            let last = self.last.clone();
            self.begin_block(last);
            self.leaf = Some(Leaf::Paragraph(InlineBuf::default()));
        }
        match &mut self.leaf {
            Some(Leaf::Table(table)) => table.cell.get_or_insert_with(InlineBuf::default),
            Some(Leaf::Paragraph(buf) | Leaf::Heading(_, buf)) => buf,
            // A paragraph was just opened, so this can't be; text written
            // here is dropped rather than panicking.
            Some(Leaf::Code { .. } | Leaf::Html(_)) | None => &mut self.spare,
        }
    }

    /// Ends the open leaf, if any, and adds its block where it belongs.
    fn close_leaf(&mut self) {
        let Some(leaf) = self.leaf.take() else { return };
        self.link = None;
        self.inner_links = 0;
        self.image = None;
        (self.strong, self.emphasis, self.strike) = (0, 0, 0);
        let block = match leaf {
            Leaf::Paragraph(buf) => RichBlock::Paragraph(buf.finish()),
            Leaf::Heading(level, buf) => RichBlock::Heading { level, text: buf.finish() },
            Leaf::Code { label, text } => RichBlock::Code(Code { label, text: text.into() }),
            Leaf::Html(text) => RichBlock::Paragraph(Inline::plain(text.trim_end_matches('\n').to_string())),
            Leaf::Table(table) => RichBlock::Table(Table::new(table.columns, table.head, table.rows)),
        };
        self.emit(block);
    }

    fn close_frame(&mut self, frame: Frame) {
        match frame {
            Frame::Quote(blocks) => self.emit(RichBlock::Quote(blocks)),
            Frame::List { kind, items } => self.emit(RichBlock::List { kind, items }),
            Frame::Item { task, blocks } => match self.frames.last_mut() {
                Some(Frame::List { items, .. }) => items.push(ListItem { task, blocks }),
                Some(Frame::Quote(_) | Frame::Item { .. }) | None => {
                    for block in blocks {
                        self.emit(block);
                    }
                }
            },
        }
    }

    /// A finished block: into the container around it, else a part.
    fn emit(&mut self, block: RichBlock) {
        match self.frames.last_mut() {
            Some(Frame::Quote(blocks) | Frame::Item { blocks, .. }) => blocks.push(block),
            Some(Frame::List { items, .. }) => items.push(ListItem { task: None, blocks: vec![block] }),
            None => {
                let range = self.top.take().unwrap_or_else(|| self.last.clone());
                self.parts.push(Part { range, block: Arc::new(block) });
            }
        }
    }

    fn finish(mut self) -> Vec<Part> {
        self.close_leaf();
        self.skipped = 0;
        while let Some(frame) = self.frames.pop() {
            self.close_frame(frame);
        }
        if self.parts.is_empty() {
            let block = RichBlock::Paragraph(Inline::default());
            self.parts.push(Part { range: 0..self.source.len(), block: Arc::new(block) });
        }
        self.parts
    }
}

/// What a ctrl-click on a link an agent wrote does (design.md decision 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Open it now: an http or https link the terminal would open.
    Open,
    /// Ask first: a link the terminal would open, of another scheme.
    Confirm,
    /// Never: the terminal's refusal.
    Refuse(Refusal),
}

/// What a click on `target` does, given the terminal's `plan` for it: its
/// refusals stand, and of what it would open only http and https links
/// open without asking.
pub fn link_action(target: &str, plan: Plan) -> LinkAction {
    match plan {
        Plan::Refuse(refusal) => LinkAction::Refuse(refusal),
        Plan::Open(_) | Plan::Reveal(_) => {
            let scheme = target.split_once(':').map_or("", |(scheme, _)| scheme);
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
                LinkAction::Open
            } else {
                LinkAction::Confirm
            }
        }
    }
}

/// The dump's word for a link's action.
pub fn action_word(action: &LinkAction) -> &'static str {
    match action {
        LinkAction::Open => "open",
        LinkAction::Confirm => "confirm",
        LinkAction::Refuse(Refusal::NoScheme) => "refuse:no-scheme",
        LinkAction::Refuse(Refusal::Scheme(_)) => "refuse:scheme",
        LinkAction::Refuse(Refusal::OtherHost(_)) => "refuse:other-host",
        LinkAction::Refuse(Refusal::NoDrive) => "refuse:no-drive",
    }
}

/// The dump's `kind` for a block: `paragraph`, `heading:L`, `code`,
/// `code:LANG` (the label with every character but an ASCII letter, digit
/// or one of `+#._-` dropped; `code` when nothing is left), `list`,
/// `quote`, `table` or `rule`.
pub fn kind_word(block: &RichBlock) -> String {
    match block {
        RichBlock::Paragraph(_) => "paragraph".into(),
        RichBlock::Heading { level, .. } => {
            let level = match level {
                HeadingLevel::H1 => 1,
                HeadingLevel::H2 => 2,
                HeadingLevel::H3 => 3,
                HeadingLevel::H4 => 4,
                HeadingLevel::H5 => 5,
                HeadingLevel::H6 => 6,
            };
            format!("heading:{level}")
        }
        RichBlock::Code(code) => {
            let lang: String = code
                .label
                .as_ref()
                .map_or("", |label| label.as_ref())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '.' | '_' | '-'))
                .collect();
            if lang.is_empty() { "code".into() } else { format!("code:{lang}") }
        }
        RichBlock::List { .. } => "list".into(),
        RichBlock::Quote(_) => "quote".into(),
        RichBlock::Table(_) => "table".into(),
        RichBlock::Rule => "rule".into(),
    }
}

/// A block's text as the dump shows it: a paragraph's or heading's runs
/// joined; code exactly; a list's items one per line after `- ` or `N. `
/// (and `[ ] ` or `[x] ` for a task), nested blocks two spaces in; a
/// quote's lines each after `> `; a table's rows one per line, cells
/// joined by ` | `; a rule nothing.
pub fn shown_text(block: &RichBlock) -> String {
    match block {
        RichBlock::Paragraph(text) | RichBlock::Heading { text, .. } => text.text().to_string(),
        RichBlock::Code(code) => code.text.to_string(),
        RichBlock::List { kind, items } => {
            let mut lines = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let marker = match kind {
                    ListKind::Bullet => "- ".to_string(),
                    ListKind::Numbered { start } => format!("{}. ", start.saturating_add(index as u64)),
                };
                let task = match item.task {
                    None => "",
                    Some(Task::Open) => "[ ] ",
                    Some(Task::Done) => "[x] ",
                };
                let body = blocks_text(&item.blocks);
                for (at, line) in body.split('\n').enumerate() {
                    lines.push(if at == 0 { format!("{marker}{task}{line}") } else { format!("  {line}") });
                }
            }
            lines.join("\n")
        }
        RichBlock::Quote(blocks) => {
            blocks_text(blocks).split('\n').map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n")
        }
        RichBlock::Table(table) => {
            let row = |cells: &[Inline]| cells.iter().map(|cell| cell.text().as_ref()).collect::<Vec<_>>().join(" | ");
            std::iter::once(row(table.head())).chain(table.rows().iter().map(|cells| row(cells))).collect::<Vec<_>>().join("\n")
        }
        RichBlock::Rule => String::new(),
    }
}

/// Blocks' texts one after another, each without its final line break.
fn blocks_text(blocks: &[RichBlock]) -> String {
    blocks
        .iter()
        .map(|block| {
            let text = shown_text(block);
            text.strip_suffix('\n').map(str::to_string).unwrap_or(text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A block's links in document order (in its lists, quotes and table cells
/// too), each with the text it covers.
pub fn block_links(block: &RichBlock) -> Vec<(&InlineLink, &str)> {
    let mut found = Vec::new();
    collect_links(block, &mut found);
    found
}

fn inline_links<'b>(inline: &'b Inline, found: &mut Vec<(&'b InlineLink, &'b str)>) {
    for link in inline.links() {
        found.push((link, inline.text().get(link.range.clone()).unwrap_or_default()));
    }
}

fn collect_links<'b>(block: &'b RichBlock, found: &mut Vec<(&'b InlineLink, &'b str)>) {
    match block {
        RichBlock::Paragraph(text) | RichBlock::Heading { text, .. } => inline_links(text, found),
        RichBlock::Code(_) | RichBlock::Rule => {}
        RichBlock::List { items, .. } => {
            for block in items.iter().flat_map(|item| &item.blocks) {
                collect_links(block, found);
            }
        }
        RichBlock::Quote(blocks) => {
            for block in blocks {
                collect_links(block, found);
            }
        }
        RichBlock::Table(table) => {
            for cell in table.head().iter().chain(table.rows().iter().flatten()) {
                inline_links(cell, found);
            }
        }
    }
}

/// The last code block's text in `block`, at any depth.
pub fn last_code(block: &RichBlock) -> Option<&SharedString> {
    match block {
        RichBlock::Code(code) => Some(&code.text),
        RichBlock::List { items, .. } => items.iter().rev().flat_map(|item| item.blocks.iter().rev()).find_map(last_code),
        RichBlock::Quote(blocks) => blocks.iter().rev().find_map(last_code),
        RichBlock::Paragraph(_) | RichBlock::Heading { .. } | RichBlock::Table(_) | RichBlock::Rule => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::links::plan;

    /// Each part's kind and shown text.
    fn kinds(source: &str) -> Vec<(String, String)> {
        render(source).iter().map(|part| (kind_word(&part.block), shown_text(&part.block))).collect()
    }

    fn pair(kind: &str, text: &str) -> (String, String) {
        (kind.to_string(), text.to_string())
    }

    /// The one block `source` renders to.
    fn one(source: &str) -> RichBlock {
        let parts = render(source);
        assert_eq!(parts.len(), 1, "{source:?}: {parts:#?}");
        (*parts[0].block).clone()
    }

    /// The inline of the one paragraph `source` renders to.
    fn para(source: &str) -> Inline {
        match one(source) {
            RichBlock::Paragraph(inline) => inline,
            other => panic!("{source:?}: not a paragraph: {other:?}"),
        }
    }

    /// The inline's runs as (text, style).
    fn runs(inline: &Inline) -> Vec<(String, RunStyle)> {
        let mut at = 0;
        inline
            .runs()
            .iter()
            .map(|(len, style)| {
                let text = inline.text()[at..at + len].to_string();
                at += len;
                (text, *style)
            })
            .collect()
    }

    /// The inline's links as (shown text, target, tooltip).
    fn links_of(inline: &Inline) -> Vec<(String, String, Option<String>)> {
        inline
            .links()
            .iter()
            .map(|link| {
                let shown = inline.text()[link.range.clone()].to_string();
                (shown, link.target.to_string(), link.tooltip.as_ref().map(|t| t.to_string()))
            })
            .collect()
    }

    fn style(f: impl FnOnce(&mut RunStyle)) -> RunStyle {
        let mut style = RunStyle::default();
        f(&mut style);
        style
    }

    /// Every inline in `block` checks again, and nesting is bounded.
    fn check(block: &RichBlock, depth: usize) {
        assert!(depth <= MAX_DEPTH, "nested {depth} deep");
        let inline_ok = |inline: &Inline| {
            let again = Inline::new(inline.text().clone(), inline.runs().to_vec(), inline.links().to_vec());
            assert_eq!(again.as_ref(), Ok(inline), "{inline:?}");
        };
        match block {
            RichBlock::Paragraph(text) | RichBlock::Heading { text, .. } => inline_ok(text),
            RichBlock::Code(_) | RichBlock::Rule => {}
            RichBlock::List { items, .. } => {
                for block in items.iter().flat_map(|item| &item.blocks) {
                    check(block, depth + 1);
                }
            }
            RichBlock::Quote(blocks) => {
                for block in blocks {
                    check(block, depth + 1);
                }
            }
            RichBlock::Table(table) => {
                assert!(table.width() <= MAX_COLUMNS);
                for cell in table.head().iter().chain(table.rows().iter().flatten()) {
                    inline_ok(cell);
                }
            }
        }
    }

    /// `source` renders without a panic, every inline valid, its parts in
    /// order inside the source.
    fn renders_cleanly(source: &str) {
        let parts = render(source);
        assert!(!parts.is_empty(), "{source:?}");
        let mut end = 0;
        for part in &parts {
            assert!(part.range.start >= end && part.range.end <= source.len().max(end), "{source:?}: {parts:?}");
            end = part.range.end;
            check(&part.block, 0);
            // The dump's words never panic either.
            let _ = (shown_text(&part.block), kind_word(&part.block), block_links(&part.block));
        }
    }

    #[test]
    fn an_empty_source_is_one_empty_paragraph() {
        for source in ["", "\n\n", "   \n", "[d]: https://example.com"] {
            let parts = render(source);
            assert_eq!(parts, [Part { range: 0..source.len(), block: Arc::new(RichBlock::Paragraph(Inline::default())) }]);
        }
    }

    #[test]
    fn each_top_level_block_is_a_part_with_its_source_range() {
        let source = "Try:\n\n```rust\nfn main() {}\n```\nSee [docs](https://example.com).";
        let parts = render(source);
        assert_eq!(
            kinds(source),
            [pair("paragraph", "Try:"), pair("code:rust", "fn main() {}\n"), pair("paragraph", "See docs.")]
        );
        let texts: Vec<&str> = parts.iter().map(|part| source[part.range.clone()].trim_end()).collect();
        assert_eq!(texts, ["Try:", "```rust\nfn main() {}\n```", "See [docs](https://example.com)."]);
    }

    #[test]
    fn paragraphs_keep_tabs_and_read_soft_breaks_as_spaces_and_hard_breaks_as_line_breaks() {
        assert_eq!(
            kinds("one\n\ntwo\tthree\n\nfour"),
            [pair("paragraph", "one"), pair("paragraph", "two\tthree"), pair("paragraph", "four")]
        );
        assert_eq!(para("a\nb").text().as_ref(), "a b");
        assert_eq!(para("a  \nb").text().as_ref(), "a\nb");
        assert_eq!(para("a\\\nb").text().as_ref(), "a\nb");
    }

    #[test]
    fn headings_atx_and_setext() {
        assert_eq!(one("## Plan"), RichBlock::Heading { level: HeadingLevel::H2, text: Inline::plain("Plan") });
        assert_eq!(kinds("# a\n\n###### f"), [pair("heading:1", "a"), pair("heading:6", "f")]);
        assert_eq!(kinds("Title\n====="), [pair("heading:1", "Title")]);
        assert_eq!(kinds("Title\n---"), [pair("heading:2", "Title")]);
    }

    #[test]
    fn code_blocks_keep_their_text_and_take_the_info_strings_first_word() {
        let code = |label: Option<&str>, text: &str| {
            RichBlock::Code(Code { label: label.map(|l| l.to_string().into()), text: text.to_string().into() })
        };
        assert_eq!(one("```rust\nfn main() {}\n```"), code(Some("rust"), "fn main() {}\n"));
        assert_eq!(one("```rust ignore\n  a\n\n  b\n```"), code(Some("rust"), "  a\n\n  b\n"));
        assert_eq!(one("```\nx\n```"), code(None, "x\n"));
        assert_eq!(one("~~~py\nx\n~~~"), code(Some("py"), "x\n"));
        assert_eq!(one("    a\n      b\n"), code(None, "a\n  b\n"));
        assert_eq!(one("    a"), code(None, "a"), "the parser's text exactly, at the end of input too");
        assert_eq!(one("```\n```"), code(None, ""));
    }

    #[test]
    fn lists_bulleted_numbered_tasks_and_nested() {
        let RichBlock::List { kind, items } = one("3. a\n4. b\n   - c") else { panic!() };
        assert_eq!(kind, ListKind::Numbered { start: 3 });
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].blocks, [RichBlock::Paragraph(Inline::plain("a"))]);
        assert!(matches!(&items[1].blocks[1], RichBlock::List { kind: ListKind::Bullet, items } if items.len() == 1));
        assert_eq!(kinds("3. a\n4. b\n   - c"), [pair("list", "3. a\n4. b\n  - c")]);

        let RichBlock::List { kind, items } = one("- [x] build\n- [ ] test\n- plain") else { panic!() };
        assert_eq!(kind, ListKind::Bullet);
        let tasks: Vec<Option<Task>> = items.iter().map(|item| item.task).collect();
        assert_eq!(tasks, [Some(Task::Done), Some(Task::Open), None]);
        assert_eq!(kinds("- [x] build\n- [ ] test"), [pair("list", "- [x] build\n- [ ] test")]);

        // A loose list's items hold paragraphs; blank lines inside stay one part.
        assert_eq!(kinds("- a\n\n- b\n\n  more"), [pair("list", "- a\n- b\n  more")]);
        assert_eq!(kinds("* a\n* b"), [pair("list", "- a\n- b")]);
        assert_eq!(kinds("1. a\n\n```\nx\n```"), [pair("list", "1. a"), pair("code", "x\n")]);
        assert_eq!(kinds("- ```\n  x\n  ```"), [pair("list", "- x")]);
    }

    #[test]
    fn quotes_hold_their_blocks() {
        assert_eq!(one("> a\n> b"), RichBlock::Quote(vec![RichBlock::Paragraph(Inline::plain("a b"))]));
        assert_eq!(kinds("> a\n>\n> - b\n> > c"), [pair("quote", "> a\n> - b\n> > c")]);
        // GitHub's alert tags are text.
        assert_eq!(kinds("> [!NOTE]\n> x"), [pair("quote", "> [!NOTE] x")]);
    }

    #[test]
    fn tables_with_their_alignments() {
        let source = "| name | size | x |\n| :--- | ---: | :-: |\n| a | 1 |\n| b | 2 | 3 | 4 |";
        let RichBlock::Table(table) = one(source) else { panic!() };
        assert_eq!(table.columns(), [ColumnAlign::Start, ColumnAlign::End, ColumnAlign::Center]);
        assert_eq!(table.head(), [Inline::plain("name"), Inline::plain("size"), Inline::plain("x")]);
        assert_eq!(table.rows()[0], [Inline::plain("a"), Inline::plain("1"), Inline::default()]);
        assert_eq!(table.rows()[1], [Inline::plain("b"), Inline::plain("2"), Inline::plain("3")]);
        assert_eq!(kinds(source), [pair("table", "name | size | x\na | 1 | \nb | 2 | 3")]);
        let RichBlock::Table(table) = one("| **a** |\n| --- |\n| `b` |") else { panic!() };
        assert_eq!(runs(&table.head()[0]), [("a".to_string(), style(|s| s.strong = true))]);
        assert_eq!(runs(&table.rows()[0][0]), [("b".to_string(), style(|s| s.code = true))]);
    }

    #[test]
    fn rules() {
        assert_eq!(kinds("a\n\n---\n\nb"), [pair("paragraph", "a"), pair("rule", ""), pair("paragraph", "b")]);
        assert_eq!(one("***"), RichBlock::Rule);
    }

    #[test]
    fn emphasis_strong_strikethrough_and_inline_code() {
        let inline = para("Run **cargo test** and *read* the ~~old~~ `log` ***both***");
        assert_eq!(inline.text().as_ref(), "Run cargo test and read the old log both");
        assert_eq!(
            runs(&inline),
            [
                ("Run ".to_string(), RunStyle::default()),
                ("cargo test".to_string(), style(|s| s.strong = true)),
                (" and ".to_string(), RunStyle::default()),
                ("read".to_string(), style(|s| s.emphasis = true)),
                (" the ".to_string(), RunStyle::default()),
                ("old".to_string(), style(|s| s.strike = true)),
                (" ".to_string(), RunStyle::default()),
                ("log".to_string(), style(|s| s.code = true)),
                (" ".to_string(), RunStyle::default()),
                ("both".to_string(), style(|s| (s.strong, s.emphasis) = (true, true))),
            ]
        );
        assert_eq!(runs(&para("**a `b`**"))[1], ("b".to_string(), style(|s| (s.strong, s.code) = (true, true))));
    }

    #[test]
    fn links_and_email_autolinks() {
        let inline = para("[docs](https://example.com/docs) <https://example.com> <dev@example.com> [r][d]\n\n[d]: https://r.example");
        assert_eq!(
            links_of(&inline),
            [
                ("docs".into(), "https://example.com/docs".into(), Some("https://example.com/docs".into())),
                ("https://example.com".into(), "https://example.com".into(), None),
                ("dev@example.com".into(), "mailto:dev@example.com".into(), Some("mailto:dev@example.com".into())),
                ("r".into(), "https://r.example".into(), Some("https://r.example".into())),
            ]
        );
        assert_eq!(inline.text().as_ref(), "docs https://example.com dev@example.com r");
        // An empty link shows its target.
        assert_eq!(links_of(&para("[](https://x.example)")), [("https://x.example".into(), "https://x.example".into(), None)]);
        // Styled link text keeps its styles.
        let inline = para("[**a** b](https://x.example)");
        assert_eq!(runs(&inline)[0], ("a".to_string(), style(|s| s.strong = true)));
        assert_eq!(links_of(&inline)[0].0, "a b");
    }

    #[test]
    fn html_is_text_as_written() {
        let inline = para("a <b>bold</b> claim <script>x()</script>");
        assert_eq!(inline.text().as_ref(), "a <b>bold</b> claim <script>x()</script>");
        assert!(inline.runs().iter().all(|(_, style)| !style.strong), "{inline:?}");
        assert_eq!(kinds("<div>\n<b>x</b>\n</div>\n\nafter"), [pair("paragraph", "<div>\n<b>x</b>\n</div>"), pair("paragraph", "after")]);
        assert_eq!(kinds("<!-- note -->"), [pair("paragraph", "<!-- note -->")]);
    }

    #[test]
    fn images_are_muted_text_linked_to_their_whole_target() {
        let inline = para("see ![chart](https://example.com/c.png) now");
        assert_eq!(inline.text().as_ref(), "see [image: chart] https://example.com/c.png now");
        let muted = style(|s| s.muted = true);
        assert_eq!(runs(&inline)[1], ("[image: chart] https://example.com/c.png".to_string(), muted));
        assert_eq!(
            links_of(&inline),
            [(
                "[image: chart] https://example.com/c.png".into(),
                "https://example.com/c.png".into(),
                Some("https://example.com/c.png".into())
            )]
        );
        let long = format!("data:image/png;base64,{}", "A".repeat(500));
        let inline = para(&format!("![x *y*]({long})"));
        let cut: String = long.chars().take(120).collect();
        assert_eq!(inline.text().as_ref(), format!("[image: x y] {cut}…"));
        assert_eq!(inline.links()[0].target.as_ref(), long);
        // Inside a link the link's target is the one pressed.
        let inline = para("[![a](https://i.example/a.png)](https://example.com)");
        assert_eq!(links_of(&inline).len(), 1);
        assert_eq!(inline.links()[0].target.as_ref(), "https://example.com");
    }

    #[test]
    fn footnotes_math_and_smart_quotes_stay_as_written() {
        assert_eq!(kinds("See [^1]\n\n[^1]: the note"), [pair("paragraph", "See [^1]"), pair("paragraph", "[^1]: the note")]);
        assert_eq!(kinds("$x^2$ and $$y$$"), [pair("paragraph", "$x^2$ and $$y$$")]);
        assert_eq!(kinds("\"quoted\" -- 'it's' ..."), [pair("paragraph", "\"quoted\" -- 'it's' ...")]);
        assert_eq!(kinds("# H {#id}"), [pair("heading:1", "H {#id}")]);
        assert_eq!(kinds("H~2~O ^x^"), [pair("paragraph", "H~2~O ^x^")]);
    }

    #[test]
    fn bare_urls_link_in_plain_text_only() {
        let inline = para("see https://example.com/a for more");
        assert_eq!(links_of(&inline), [("https://example.com/a".into(), "https://example.com/a".into(), None)]);
        let inline = para("(see https://example.com/a), and https://b.example/x_(y).");
        assert_eq!(
            links_of(&inline).into_iter().map(|(shown, ..)| shown).collect::<Vec<_>>(),
            ["https://example.com/a", "https://b.example/x_(y)"]
        );
        assert_eq!(links_of(&para("[https://example.com/a]")).len(), 1);
        assert_eq!(links_of(&para("run `https://example.com` now")), []);
        assert!(block_links(&one("```\nhttps://example.com\n```")).is_empty());
        assert!(block_links(&one("    https://example.com")).is_empty());
        let inline = para("[see https://a.example](https://b.example)");
        assert_eq!(links_of(&inline), [("see https://a.example".into(), "https://b.example".into(), Some("https://b.example".into()))]);
        assert_eq!(links_of(&para("www.example.com and a@example.com")), []);
        // Across styles and in a list.
        assert_eq!(links_of(&para("**https://example.com/a**")).len(), 1);
        let RichBlock::List { items, .. } = one("- https://example.com/a_b_c") else { panic!() };
        let RichBlock::Paragraph(inline) = &items[0].blocks[0] else { panic!() };
        assert_eq!(links_of(inline), [("https://example.com/a_b_c".into(), "https://example.com/a_b_c".into(), None)]);
        // Multi-byte text before a URL keeps the ranges on char boundaries.
        let inline = para("é → https://example.com/ü ok");
        assert_eq!(links_of(&inline)[0].0, "https://example.com/ü");
    }

    #[test]
    fn nesting_past_max_depth_is_kept_as_paragraphs() {
        let quotes = format!("{}deep", "> ".repeat(20));
        let parts = render(&quotes);
        assert_eq!(parts.len(), 1);
        check(&parts[0].block, 0);
        let mut block = &*parts[0].block;
        let mut depth = 0;
        while let RichBlock::Quote(blocks) = block {
            depth += 1;
            block = &blocks[0];
        }
        assert_eq!(depth, MAX_DEPTH);
        assert_eq!(block, &RichBlock::Paragraph(Inline::plain("deep")));

        let lists = format!("{}x\n{}y", "- ".repeat(20), "  ".repeat(19));
        let parts = render(&lists);
        check(&parts[0].block, 0);
        let mut block = &*parts[0].block;
        let mut depth = 0;
        while let RichBlock::List { items, .. } = block {
            depth += 1;
            block = &items[0].blocks[0];
        }
        assert_eq!(depth, MAX_DEPTH);
        assert!(matches!(block, RichBlock::Paragraph(_)), "{block:?}");
    }

    #[test]
    fn a_table_past_max_columns_is_its_source_as_code() {
        let head = format!("|{}\n|{}\n|{}", " h |".repeat(MAX_COLUMNS + 1), " - |".repeat(MAX_COLUMNS + 1), " c |".repeat(MAX_COLUMNS + 1));
        let source = format!("before\n\n{head}\n\nafter");
        let parts = render(&source);
        assert_eq!(parts.len(), 3);
        let RichBlock::Code(code) = &*parts[1].block else { panic!("{parts:?}") };
        assert_eq!(code.label, None);
        assert_eq!(code.text.trim_end(), head);
        assert_eq!(kind_word(&parts[2].block), "paragraph");
        let fits = format!("|{}\n|{}", " h |".repeat(MAX_COLUMNS), " - |".repeat(MAX_COLUMNS));
        assert!(matches!(one(&fits), RichBlock::Table(table) if table.width() == MAX_COLUMNS));
    }

    /// Awkward inputs; every prefix of each must render.
    fn corpus() -> Vec<String> {
        vec![
            "```rust\nfn main() {\n".into(),
            "**unclosed *emphasis ~~strike `code".into(),
            "- ".repeat(100) + "x",
            "> ".repeat(100) + "x",
            "1. ".repeat(100) + "x",
            format!("|{}\n|{}\n|{}", " a |".repeat(100), " - |".repeat(100), " b |".repeat(100)),
            "| a | b |\n| - | - |\n| c".into(),
            "<div>\n<script>alert(1)</script>\n<b>x".into(),
            "a\0b\0\0[\0](\0)".into(),
            "line\r\n\r\n- item\r\n  more\r\n```\r\ncode\r\n".into(),
            "\tcode\n\t\tmore\n- a\tb".into(),
            "![img](".into(),
            "[a](b \"t".into(),
            "[x][y]\n\n[y]: <".into(),
            "&#0; &#xD800; &amp &nbsp; \\".into(),
            "> - [ ] task\n>   ```\n>   code".into(),
            "- a\n\n  > b\n  >\n  > ```\n  > c".into(),
            "# \n## #\nsetext\n===\n---\n***\n___".into(),
            "<https://a.example> <a@b.example> https://c.example/(x) [https://d](e)".into(),
            "é🎉 **ü** `ß` [ñ](https://ñ.example) ![ü](ü)".into(),
            "[^1]\n\n[^1]: note\n$x$ $$y$$ \"q\"".into(),
            "*a **b *c* d** e*".into(),
            "- a\n- b\n\n\n- c\n  1. d\n     - [x] e".into(),
        ]
    }

    /// OpenSpec change `agents-absorb-cost` decision 8: a reference used
    /// many times holds one target, which its tooltip shares.
    #[test]
    fn a_reference_used_many_times_shares_one_target() {
        let source = format!("{}\n\n[d]: https://example.com/docs", "[a][d] ".repeat(100));
        let inline = para(&source);
        let links = inline.links();
        assert_eq!(links.len(), 100);
        let first = links[0].target.as_ptr();
        for link in links {
            assert_eq!(link.target.as_ref(), "https://example.com/docs");
            assert_eq!(link.target.as_ptr(), first, "one target");
            let tooltip = link.tooltip.as_ref().expect("its text differs from its target");
            assert_eq!(tooltip.as_ptr(), first, "the tooltip is the target");
        }
    }

    #[test]
    fn every_prefix_of_awkward_input_renders() {
        for source in corpus() {
            for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
                renders_cleanly(&source[..end]);
            }
        }
    }

    #[test]
    fn random_markdown_renders() {
        let alphabet: Vec<&str> = vec![
            "*", "_", "`", "~", "#", ">", "-", "+", "=", "|", ":", "[", "]", "(", ")", "!", "<", ">", "\\", "\n", "\n\n",
            " ", "  ", "\t", "1.", "a", "é", "&", ";", "```", "- [ ] ", "https://x.example/", "<b>", "\r\n", "\0", "🎉",
        ];
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..10_000 {
            let len = (next() % 40) as usize;
            let source: String = (0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect();
            renders_cleanly(&source);
        }
    }

    fn here(windows: bool) -> impl Fn(&str) -> Plan {
        move |uri| plan(uri, "thishost", windows)
    }

    #[test]
    fn link_actions_follow_the_terminals_plan() {
        let linux = here(false);
        let action = |target: &str| link_action(target, linux(target));
        assert_eq!(action("https://example.com"), LinkAction::Open);
        assert_eq!(action("HTTPS://EXAMPLE.COM"), LinkAction::Open);
        assert_eq!(action("http://example.com"), LinkAction::Open);
        assert_eq!(action("mailto:dev@example.com"), LinkAction::Confirm);
        assert_eq!(action("ftp://example.com/x"), LinkAction::Confirm);
        assert_eq!(action("file:///tmp/x"), LinkAction::Confirm);
        assert_eq!(action("javascript:void"), LinkAction::Refuse(Refusal::Scheme("javascript".into())));
        assert_eq!(action("file://otherhost/share/x"), LinkAction::Refuse(Refusal::OtherHost("otherhost".into())));
        assert_eq!(action("no scheme"), LinkAction::Refuse(Refusal::NoScheme));
        let windows = here(true);
        let action = |target: &str| link_action(target, windows(target));
        assert_eq!(action("file:///C:/x"), LinkAction::Confirm, "a Reveal confirms");
        assert_eq!(action("file:///etc/hosts"), LinkAction::Refuse(Refusal::NoDrive));
        // Every plan variant, whatever the target says.
        assert_eq!(link_action("https://x", Plan::Open("https://x".into())), LinkAction::Open);
        assert_eq!(link_action("https://x", Plan::Reveal("C:\\x".into())), LinkAction::Open);
        assert_eq!(link_action("ftp://x", Plan::Open("ftp://x".into())), LinkAction::Confirm);
        assert_eq!(link_action("https://x", Plan::Refuse(Refusal::NoDrive)), LinkAction::Refuse(Refusal::NoDrive));
    }

    #[test]
    fn action_words() {
        let words: Vec<&str> = [
            LinkAction::Open,
            LinkAction::Confirm,
            LinkAction::Refuse(Refusal::NoScheme),
            LinkAction::Refuse(Refusal::Scheme("javascript".into())),
            LinkAction::Refuse(Refusal::OtherHost("h".into())),
            LinkAction::Refuse(Refusal::NoDrive),
        ]
        .iter()
        .map(action_word)
        .collect();
        assert_eq!(words, ["open", "confirm", "refuse:no-scheme", "refuse:scheme", "refuse:other-host", "refuse:no-drive"]);
    }

    #[test]
    fn the_dumps_kind_words() {
        let code = |label: Option<&str>| {
            RichBlock::Code(Code { label: label.map(|l| l.to_string().into()), text: "x\n".into() })
        };
        assert_eq!(kind_word(&code(Some("c++ \"x\""))), "code:c++x");
        assert_eq!(kind_word(&code(Some("C#"))), "code:C#");
        assert_eq!(kind_word(&code(Some("objective-c.v2_x"))), "code:objective-c.v2_x");
        assert_eq!(kind_word(&code(Some("\"é\""))), "code");
        assert_eq!(kind_word(&code(None)), "code");
        assert_eq!(kind_word(&RichBlock::Paragraph(Inline::default())), "paragraph");
        assert_eq!(kind_word(&RichBlock::Heading { level: HeadingLevel::H3, text: Inline::default() }), "heading:3");
        assert_eq!(kind_word(&RichBlock::List { kind: ListKind::Bullet, items: Vec::new() }), "list");
        assert_eq!(kind_word(&RichBlock::Quote(Vec::new())), "quote");
        assert_eq!(kind_word(&RichBlock::Table(Table::new(Vec::new(), Vec::new(), Vec::new()))), "table");
        assert_eq!(kind_word(&RichBlock::Rule), "rule");
        // A fence's info string as written gives the label's first word.
        assert_eq!(kinds("``` c++ \"x\"\ny\n```"), [pair("code:c++", "y\n")]);
    }

    #[test]
    fn the_dumps_shown_text() {
        assert_eq!(shown_text(&one("a *b* `c`")), "a b c");
        assert_eq!(shown_text(&one("```\n  x\n\ny\n```")), "  x\n\ny\n");
        assert_eq!(shown_text(&one("- a\n  - b\n    - c\n- d")), "- a\n  - b\n    - c\n- d");
        assert_eq!(shown_text(&one("7. a\n8. [x] b")), "7. a\n8. [x] b");
        assert_eq!(shown_text(&one("> a\n>\n> b")), "> a\n> b");
        assert_eq!(shown_text(&one("| a | b |\n| - | - |\n| 1 | 2 |")), "a | b\n1 | 2");
        assert_eq!(shown_text(&RichBlock::Rule), "");
        // The dump escapes quotes and line breaks with `{:?}`.
        let text = shown_text(&one("say \"hi\"  \nthere"));
        assert_eq!(format!("{text:?}"), "\"say \\\"hi\\\"\\nthere\"");
    }

    #[test]
    fn links_in_document_order_through_lists_quotes_and_tables() {
        let block = one("- [a](https://a.example)\n  > [b](https://b.example) https://c.example");
        let found: Vec<(&str, &str)> = block_links(&block).into_iter().map(|(link, text)| (link.target.as_ref(), text)).collect();
        assert_eq!(found, [("https://a.example", "a"), ("https://b.example", "b"), ("https://c.example", "https://c.example")]);
        let table = one("| [h](https://h.example) |\n| - |\n| https://r.example |");
        assert_eq!(block_links(&table).len(), 2);
    }

    #[test]
    fn the_last_code_block_at_any_depth() {
        assert_eq!(last_code(&one("```\na\n```")).map(|t| t.as_ref()), Some("a\n"));
        assert_eq!(last_code(&one("- ```\n  a\n  ```\n- > ```\n  > b\n  > ```")).map(|t| t.as_ref()), Some("b\n"));
        assert_eq!(last_code(&one("plain")), None);
    }
}
