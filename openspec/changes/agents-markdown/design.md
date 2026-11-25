# Design

## Context

See proposal.md for why. The user answered the proposal's eight questions on 2026-11-25; decision 1 lists the answers and where each is worked out. Facts from the code as built (`agents-profiles`, archived 2026-11-25) that shape this design:

- **The chat model** (`src/agent/model.rs`, a whole pure module) keeps a copy of each entry and a flat `blocks: Vec<Block>` the view's list draws, one list item per block. `blocks_from` turns a user or agent message into `BlockBody::Text { speaker, text }` parts cut by `parts(text, from)` (at blank lines and, past `BLOCK_LIMIT` = 4 KiB, at the next line break); a thought, tool, permission or notice is one block. `absorb` folds `Changes` from `AgentShared`: a `Dirty::Appended { from_byte }` on a message copies its new tail and redraws only its last part (`blocks_from(id, entry, last.part, last.start, …)`), returning `Splice`s the view applies to `MessageScroller`.
- **Pacing.** The reader thread folds events into `AgentShared`; the sink's wake is merged ("at most one waits") and drained paced (`terminal/pace.rs`), so `absorb` runs at most once per paced wake however many chunks arrived, and a streaming message is one `Dirty::Appended` per absorb.
- **The view** (`src/agent/view.rs`, `render_block`) draws a `Text` block as `brindle_ui::Message::text(…).part(bubble_part(part, last)).action(copy)`, a thought as `Reasoning` with its text as a child, and copies through `ChatModel::message_text` / `last_agent_message` (the entry's text, which is the Markdown source).
- **`brindle-ui`** has `Message`/`Bubble` with `BubblePart::{Whole, First, Middle, Last}`, `Reasoning` (muted foreground content, no background), `MessageScroller` (an entity over GPUI's `list`), `Dialog`, `Button` with `tooltip_for`. Its closed list is `gpui`, `log` (`ports R3`). GPUI 0.2.2 has `StyledText::with_runs`, `InteractiveText` (`on_click(ranges, …)` fires only when press and release fall in one range; `on_hover`; `tooltip(index → view)`), `grid()`/`grid_cols(n)` and `text_left/center/right`, and a frame-to-frame line layout cache (`LineLayoutCache::previous_frame`), so an unchanged line is not shaped again on the next frame.
- **Links** (`src/terminal/links.rs`, the link seam): `SCHEMES` (http, https, ftp, file, ssh, git, gemini, gopher, ipfs, ipns, mailto, news, magnet), pure `find_url(chars, at)`, pure `plan(uri, this_host, windows) -> Plan::{Open, Reveal, Refuse(Refusal)}`, `open(uri) -> Opened`, `failure_text`. `TerminalView::open_link` emits `LinkFailed(text)`, forwarded by `ZoneTreeView`, shown by `Workspace::show_notice` on the config line. The gesture is `terminal_view::wants_links` (ctrl, not alt), private to the view.
- **Tokens** (`src/theme.rs` `ui_tokens`): `primary` and `ring` are the theme's accent; brindle-ui's `accent` is shadcn's hover surface (`bg.mix(fg, 0.12)`), about 1.2:1 against the card, so it can't colour text. `muted` = `secondary` = `bg.mix(fg, 0.08)`.
- **`pulldown-cmark` 0.13.6** (MIT, MSRV 1.71.1; read from the crate's sources): default features are `getopts` and `html`; with `default-features = false` it needs `bitflags` ^2, `memchr` ^2.5 and `unicase` ^2.6, which the lock already has (2.9.4, 2.7.6, 2.8.1). Its options are `ENABLE_TABLES`, `ENABLE_STRIKETHROUGH`, `ENABLE_TASKLISTS`, footnotes, smart punctuation, heading attributes, metadata blocks, math, `ENABLE_GFM` (GitHub's `> [!NOTE]` blockquote tags), definition lists, super/subscript and wiki links. It has **no GFM extended autolinks**: only `<scheme:…>` and `<user@host>` (`LinkType::Autolink`, `LinkType::Email`). `into_offset_iter()` gives each event's source range.

## Goals / Non-Goals

**Goals:**
- An agent message or thought always shows what its whole text so far means as Markdown, while it streams too, and the parts of a message the new text doesn't change aren't laid out again.
- Every decision (the mapping from Markdown to blocks, what a link click does, the dump's words, the token values) is a pure function with unit tests; the view and the dialog only gather, call and apply (`fcis R1`, `R5`, `R8`).
- `brindle-ui` gains components that draw plain data and know no Markdown, with its closed list unchanged (`ports R2`, `R3`).
- Links reuse the terminal's vetted plan, refusals and opener; nothing an agent wrote is fetched or run without a user's gesture.
- E2E cases against the fake ACP agent pin rendering, streaming, code copying and links through the dump.

**Non-Goals:** syntax highlighting; text selection in the conversation (still `agents-profiles`' non-goal; the copy actions are the way out); links as keyboard focus stops; horizontal scrolling of wide tables or code; Markdown in user messages, tool, permission and notice titles, alerts or the prompt input; plan entries (there is no plan entry kind; showing plans is a new event kind and protocol version); GFM's `www.` and bare e-mail autolinks; footnotes, math, smart punctuation, heading attributes and `> [!NOTE]` tags; a switch to turn Markdown off. No change to the terminal server, the transcript, the client protocol, config keys or state files.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| The rich-text data (`RichBlock`, `Inline`, `RunStyle`, `InlineLink`, …) | plain data, shared contract | `brindle-ui`, `ui/src/components/response.rs` |
| The `link` token | token | `brindle-ui`, `ui/src/theme.rs`; its value in `src/theme.rs` `ui_tokens` |
| Markdown → blocks (`render`), bare-URL autolinks, tooltips, depth and width bounds, a part's dump text, `link_action`, `opens_link` | pure | `brindle`, `src/agent/markdown.rs` (new, joins `PURE_MODULES`) |
| Parts of a message, the rendered cache, the diff, `describe`/`wait_text`, `last_code_block` | GPUI-free model | `brindle`, `src/agent/model.rs` |
| `plan_here(uri)` (this host, this platform) and the shared gesture `wants_links` | seam (links) | `brindle`, `src/terminal/links.rs` |
| `Response`, `CodeBlock`, links and tables drawing | components | `brindle-ui`, `ui/src/components/response.rs`, `code_block.rs` |
| Drawing parts, code copy, link presses, `copy_last_code_block` | shell (entity) | `src/agent/view.rs`, `src/agent/mod.rs`, `src/actions.rs` |
| The link confirmation | shell (overlay view) | `src/link_confirm.rs` (new), `src/workspace.rs`, `src/zone_view.rs` |
| The pointer hook's names, targets, event plan and hold check (decision 16) | pure | `brindle`, `src/pointer_hook.rs` (new, joins `PURE_MODULES`) |
| The hit-target registry and `.hit_target(name)` | primitive | `brindle-ui`, `ui/src/primitives/hit_targets.rs` (new) |
| Running a plan through `Window::dispatch_event` and frame waits; `--- hit` and miss lines | shell | `src/main.rs` (`run_debug_steps`), `src/workspace.rs` (dump) |

Crate references don't change (`ports R1`): `pulldown-cmark` goes on the app crate, which has no closed list, and `brindle-ui` stays on `gpui` and `log`. The pure renderer names `brindle_ui`'s plain data types, as `model.rs` already names `gpui::SharedString` (`fcis R5`: plain values, no entity or context).

## Decisions

### 1. The user's answers, and where each is worked out

1. **Parser:** `pulldown-cmark` with `default-features = false` in the app crate, behind one pure function that returns `brindle-ui` plain data; `brindle-ui`'s list stays `gpui`, `log`; `Cargo.lock` gains one package (decisions 2–4).
2. **Subset:** CommonMark plus GFM strikethrough, task lists, tables (a grid) and autolinks; no footnotes, math or smart punctuation; raw HTML as literal text; images never fetched, drawn as a muted `[image: alt]` with the target and clickable as a link (decisions 4, 9).
3. **Entries:** agent messages and thoughts; user messages and tool, permission and notice titles stay plain; no plan entries (decision 5).
4. **Streaming:** the whole message is parsed again whenever it grows; parts follow the parser's top-level blocks; an unclosed fence is code to the end (decision 5, cost in decision 6).
5. **Links:** through `terminal/links.rs` with the terminal's gesture; the target on hover when it differs; confirmation for schemes other than http and https; no focus stops (decision 7).
6. **Copying:** the copy button and Copy Last Agent Message copy the source; a pointer-only copy button per code block; a palette action Copy Last Code Block (decision 8).
7. **Dump:** `--- entry` keeps the source; `--- block` lines per part (decision 11).
8. **Tokens:** `muted`, `border` and `muted_foreground` for code, the terminal's family passed as a value, the accent for links; tokens added only if the legibility test fails on a built-in. It does, for links, so one token is added (decision 10).

### 2. The dependency and the lock

`Cargo.toml` (the `brindle` package): `pulldown-cmark = { version = "0.13.6", default-features = false }`, added with `cargo add pulldown-cmark@0.13.6 --no-default-features -p brindle`. `CLAUDE.md`'s rule holds: `git diff Cargo.lock` must show exactly one added `[[package]]` (`pulldown-cmark` 0.13.6 with `bitflags` 2.9.4, `memchr` 2.7.6 and `unicase` 2.8.1 as its dependencies) and the one added name in `brindle`'s `dependencies`, nothing else. Any other line moved (a version bumped, a second package, a removed entry) means the add resolved differently: revert it and pin what moved, never regenerate the lock or run a broad `cargo update`. `getopts` and `pulldown-cmark-escape` must not appear.

**One door.** Only `src/agent/markdown.rs` names `pulldown_cmark`, as only `src/agent/transcript.rs` names `brindle_transcript`; `tests/architecture.rs` gains the same kind of check (`only_the_markdown_module_names_the_parser`), so a parser swap touches one file.

Options: `ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS | ENABLE_TABLES`, nothing else. Without `ENABLE_FOOTNOTES`, `ENABLE_MATH`, `ENABLE_SMART_PUNCTUATION`, `ENABLE_HEADING_ATTRIBUTES`, the metadata blocks and `ENABLE_GFM`, their syntax stays literal text, as the spec's "Footnote stays text" says. *Rejected:* a parser in `brindle-ui` (its list would grow and the UI crate would know Markdown); a hand-written subset (CommonMark's emphasis and list rules are large and would need their own fuzzing).

### 3. The rich-text data in `brindle-ui` (`types R1`, `R7`, `R11`)

Plain data, `Clone + PartialEq + Debug`, no entity, no callback:

```rust
pub enum RichBlock {
    Paragraph(Inline),
    Heading { level: HeadingLevel, text: Inline },
    Code(Code),                                   // Code { label: Option<SharedString>, text: SharedString }
    List { kind: ListKind, items: Vec<ListItem> },
    Quote(Vec<RichBlock>),
    Table(Table),
    Rule,
}
pub enum HeadingLevel { H1, H2, H3, H4, H5, H6 }
pub enum ListKind { Bullet, Numbered { start: u64 } }
pub struct ListItem { pub task: Option<Task>, pub blocks: Vec<RichBlock> }
pub enum Task { Open, Done }
pub struct Table { columns: Vec<Align>, head: Vec<Inline>, rows: Vec<Vec<Inline>> }   // Table::new checks rows' widths
pub enum Align { Start, Center, End }
pub struct Inline { text: SharedString, runs: Vec<(usize, RunStyle)>, links: Vec<InlineLink> }
pub struct RunStyle { pub strong: bool, pub emphasis: bool, pub strike: bool, pub code: bool, pub muted: bool }
pub struct InlineLink { pub range: Range<usize>, pub target: SharedString, pub tooltip: Option<SharedString> }
```

- **What the types rule out.** A heading level outside 1–6 and a list that is both numbered and bulleted can't be built; a task mark is `Option<Task>`, not two bools; "no label" is `None`, not an empty string (`types R7`). `RunStyle`'s flags are independent (text can be bold and struck at once), so they stay bools (`types R1` binds exclusive states only).
- **`Inline` has private fields** and one constructor, `Inline::new(text, runs, links) -> Result<Inline, InlineError>` (`types R11`, `railway R1`): run lengths sum to the text's length and end on char boundaries, link ranges lie inside the text on char boundaries and don't overlap, so the component can slice without a check that could panic. `InlineError::{RunsDontCoverText, NotCharBoundary, LinkOutOfRange, LinksOverlap}`. The renderer only builds valid values, so its `Err` arm is a bug it reports by falling back (decision 4).
- **`Table::new(columns, head, rows)`** pads short rows with empty cells and drops cells past the header's width, as GFM does, so every row has one cell per column.

The token set gains `link: Hsla` (decision 10).

### 4. The renderer (`src/agent/markdown.rs`, pure; `fcis R1`–`R3`, `R5`, `R8`; `railway R5`)

```rust
pub struct Part { pub range: Range<usize>, pub block: Arc<RichBlock> }
pub fn render(source: &str) -> Vec<Part>
```

`render` walks `Parser::new_ext(source, OPTIONS).into_offset_iter()` once. Each top-level block (a `Start(tag)` at depth 0 to its `End`) becomes one `Part` whose `range` is its source range. Source with no block (empty, or blank lines only) is one empty `Paragraph` part covering it, so a message that has started shows its bubble, as `parts` did.

- **Blocks.** `Paragraph`, `Heading(level)`, `CodeBlock(Fenced(info) | Indented)` (the label is the info string's first word; the text is the code's content exactly as the parser gives it, which keeps indentation and ends with the last line break), `List(start)` with `Item` and `TaskListMarker(done)`, `BlockQuote(_)` (the `ENABLE_GFM` kinds can't occur), `Table(aligns)`/`TableHead`/`TableRow`/`TableCell`, `Rule`. A list item's or quote's inline content outside a paragraph (a tight list) becomes a `Paragraph` block.
- **Inline.** `Text` and `Code` add runs (code sets `code`); `Emphasis`, `Strong`, `Strikethrough` set flags while open; `SoftBreak` is a space and `HardBreak` a `\n`; `Link { dest_url, .. }` makes an `InlineLink` over its text; `LinkType::Email` gets `mailto:` before its address, as CommonMark's HTML does.
- **HTML** (`Html`, `InlineHtml`) is text, as written: an HTML block becomes a `Paragraph` of its lines (line breaks kept as `\n`), inline HTML a plain run. Nothing is sanitized because nothing is interpreted (ACP's "disable or sanitize").
- **Images.** `Image { dest_url, .. }` with its alt text becomes a `muted` run `[image: ALT]`, a space and the target (cut to its first 120 characters with `…` after), the whole of it one `InlineLink` to the full target. Nothing is ever fetched: no component draws an image, and the target only reaches the opener through a user's gesture (decision 7).
- **Bare URLs.** pulldown-cmark has no GFM extended autolinks, so each plain text run (not code, not already inside a link or image) is scanned with the terminal's `links::find_url`, which knows the same `SCHEMES` and the terminal's rules for trailing punctuation and brackets: what a terminal underlines, a message links. `www.` forms and bare e-mail addresses stay text (Non-Goals).
- **Tooltips.** `InlineLink::tooltip` is `Some(target)` when the link's shown text isn't exactly its target, else `None` (ACP v2: show a link's target when it differs from its text). An e-mail autolink shows its `mailto:` target.
- **Bounds (`railway R5`: the text is another process's).** Blocks nested deeper than `MAX_DEPTH` = 16 (lists and quotes) are kept as paragraphs of their text at depth 16, so drawing never nests elements without limit. A table with more than `MAX_COLUMNS` = 32 columns is shown as a code block of its source lines, unlabelled. Every slice is on a range the parser gave or a char boundary checked first.
- **No panic.** `render` holds no `unwrap` on its input. The model calls it under `std::panic::catch_unwind` (decision 5), so a parser defect shows the source as one plain paragraph and logs once, instead of ending Brindle. Tests render every prefix of a corpus of awkward inputs (unclosed fences and emphasis, nested lists and quotes 100 deep, a table row of 100 cells, HTML, NULs and lone surrogate-like escapes, CRLF, tabs, every CommonMark container at the end of input) and 10,000 deterministic pseudo-random strings over a Markdown-heavy alphabet, asserting no panic and that every `Inline` is valid.

### 5. Parts and streaming (replaces `blocks_of`' blank-line cut for agent messages)

`BlockBody::Text { speaker, text }` splits by speaker, since the two now draw differently (`types R2` makes every `match` say which):

```rust
pub enum BlockBody {
    UserText { text: SharedString },                    // plain, cut by `parts` as in step 1
    AgentPart { block: Arc<RichBlock> },                // one top-level Markdown block
    Thought { blocks: Arc<[Arc<RichBlock>]>, open: bool },
    Tool { .. }, Permission { .. }, Notice { .. },     // unchanged
}
```

- **An agent message's parts are its top-level Markdown blocks**, one list item each, `Block.part` its index and `Block.start` its `range.start`; `last` marks the bubble's end, so `BubblePart` reads them as one bubble. `BLOCK_LIMIT` no longer cuts agent messages: a fenced block or a list holding blank lines stays whole, and a block longer than 4 KiB is one item.
- **User messages keep `parts`** (blank lines, and 4 KiB): they arrive whole and stay plain, and a pasted 256 KiB prompt still lays out in pieces. `parts` and `BLOCK_LIMIT` stay for them only.
- **A thought stays one list item** (its collapsible can't be split across items), holding all its blocks; it is rendered again as a whole, as it is redrawn whole today.
- **The rendered cache** (`types R10`): `ChatModel` keeps `rendered: BTreeMap<EntryId, Vec<Part>>` for agent messages and thoughts, refreshed on every `absorb` that marks the entry `Appended` or `Replaced`, rebuilt for every entry on `reset`, dropped with the entry on `Trimmed`. Nothing else writes it.
- **Absorbing a growing message:** after copying the tail as today, the model renders the whole text again (`render`, under `catch_unwind`), compares the new parts with the cached ones from the first, finds the first index whose `range` or `block` differs, and splices the items from that index to the end of the entry with the new parts from there. Parts before it keep their list items, so the list lays out only the changed tail (the scroller already re-lays out only spliced items). Because the whole text is parsed, a later construct that changes an earlier part is shown correctly: a reference definition arriving after its use, a blank line that makes a list loose, a setext underline that turns the last paragraph into a heading. Those splices start earlier; nothing is imprecise.
- **An unclosed fence** is a `CodeBlock` to the end of the text in pulldown-cmark, so a streaming fence shows as a code block from its first line; when the closing fence arrives, the part's text stops growing and the next block becomes a new part.

### 6. What re-parsing costs, stated honestly

- **Per absorb, one parse of the growing message's whole text and one comparison of its parts:** O(n) in the message's length, on the UI thread, inside `absorb`. Absorbs are merged and paced, not per chunk, so a stream costs at most one parse per paced wake, about one per frame while text arrives. Other entries are never parsed again. A message of n bytes streamed over t seconds costs at most about 60·t parses of at most n bytes. For a 64 KiB answer streamed over 20 s that is under 1,200 parses averaging 32 KiB, about 40 MB of input; pulldown-cmark is compiled at `opt-level` 2 in every build (the dependency profile), but the walk in `markdown.rs` is `brindle` code, unoptimized in a debug build.
- **Layout:** only the spliced parts. The worst case is one long block that grows (a 2,000-line fence): its item is laid out on each absorb, and GPUI's frame-to-frame line layout cache keeps every unchanged line's shaping, so the cost is mostly hashing its lines.
- **Attach and replay** render every agent message and thought once: O(log size), at most the core's 16 MiB `LOG_LIMIT`. Memory grows by the rendered copy beside the zone's copy of each entry, about one more log's worth in the worst case.
- **The guard** (`testing R8`: assert the rule, not a time): unit tests that streaming a 256 KiB message in 64-byte appends ends with exactly `render(whole)`'s parts, that each absorb's splice starts no earlier than the first part whose text changed and touches no other entry, and that an absorb with no change to an entry renders nothing. A perf-suite case, `agent-markdown-stream` (`testing R14`: numbers against a baseline, never pass or fail), times the stream end to end (decision 13). **The fallback**, if those numbers show dropped frames: render on the reader thread after `apply`, outside `AgentShared`'s lock, and hand the parts over with the changes. It is not built now.

### 7. Links (`links`; `fcis R1`; `railway R6`, `R8`; `ports R4`, `R7`)

- **The gesture** is the terminal's: `wants_links(modifiers)` (ctrl held, alt not) moves from `terminal_view.rs` to `terminal/links.rs` as `pub fn`, used by both. A press and release on the same link (`InteractiveText` checks both indices) with the gesture opens it; without it nothing happens, and links are still drawn as links (link colour, underlined), since they are rendered Markdown, unlike terminal text. The pointer is a pointing hand over a link while the gesture's modifiers are held (`Response::link_modifiers`).
- **What a click does** is pure: `markdown::link_action(target: &str, plan: Plan) -> LinkAction`:
  ```rust
  pub enum LinkAction { Open, Confirm, Refuse(Refusal) }
  ```
  `Plan::Refuse(r)` → `Refuse(r)`; `Plan::Open(_)` or `Plan::Reveal(_)` → `Open` when the target's scheme is `http` or `https` (without case), else `Confirm`. So everything a terminal would refuse is refused before any dialog, and the dialog only ever asks about a link the terminal would open.
- **The seam** gains `links::plan_here(uri) -> Plan`, which calls `plan` with this machine's host and `cfg!(windows)`, the two platform facts `open` already reads; `open` uses it too. The view calls `plan_here`, then `link_action`.
- **Open** → `links::open(target)`; a failure, and a `Refuse`, become `LinkFailed(links::failure_text(…))`, which `AgentView` now emits and `ZoneTreeView` forwards from agent zones as it does from terminals, so the window's config line shows it (`railway R6`, `R8`: the action, then the cause). Nothing is written into the conversation.
- **Confirm** → `AgentView` emits `ConfirmLink(target)`, forwarded by `ZoneTreeView`; `Workspace` opens `Overlay::LinkConfirm { uri }` with `src/link_confirm.rs`'s `LinkConfirm`, a `brindle-ui` `Dialog` like the close confirmation: title "Open this link?", the description "An agent wrote this link. Brindle opens only http and https links without asking.", the whole target in the terminal's family (wrapped, never cut), and Cancel (outline, focused on open, so enter by reflex keeps it closed) and Open (default variant). Escape and a backdrop press cancel. Open calls `links::open` (which plans again) with the same failure path. Focus returns to the zone's prompt input either way. Its own `LinkStop::{Cancel, Open}` names the focus line's stops; no shared type with `ConfirmStop` (`types R12`).
- **Hover:** the component's `InteractiveText::tooltip` shows the link's `tooltip` (decision 4), with the existing tooltip look.

### 8. Code blocks and copying (`agent-zones`)

- **`CodeBlock`** draws a header row (the label in `muted_foreground`, a ghost `IconXs` copy button, lucide `copy`, tooltip "Copy code") on the bubble's own surface, and under it the code in the owner's family on `muted`, inside a rounded `border` frame. The button is drawn always (not on hover), takes no focus and calls the owner's `on_copy(text)`; the view writes the clipboard, as the message copy button does.
- **Inline code** is a run on `muted` in the code family, `foreground` text.
- **Copy Last Code Block:** `copy_last_code_block` in `ACTIONS` ("Copy Last Code Block", no default key), handled only by `AgentView`, so the palette lists it only in an agent zone. `ChatModel::last_code_block() -> Option<&str>`: the latest agent message (thoughts don't count) whose cached parts hold a code block, at any depth, and its last code block's text; `None` leaves the clipboard alone.
- **The message copy button and Copy Last Agent Message** keep copying the entry's text, which is the Markdown source; the code is unchanged, the spec now says so.

### 9. Tables

`RichBlock::Table` draws as a GPUI grid (`grid().grid_cols(n)`, equal-width columns, cells wrapping, each cell `text_left/center/right` from its `Align`, the default `Start`), inside a `border` frame, the header row bold with a `border` line under it and a line between body rows. A table wider than the bubble wraps its cells' text; there is no horizontal scroll. `Table::new` already made every row as wide as the header.

### 10. Tokens and fonts (`ui-theme-tokens`)

The user's rule was: reuse `muted`, `border`, `muted_foreground` and the accent; add tokens only if the legibility test fails on a built-in. The pairs this change draws, with the ratios the current derivation gives (computed while writing this design with the WCAG formula `src/theme.rs` uses; the extended test is what pins them):

| Pair | Minimum | brindle-dark | brindle-light | tokyo-night | gruvbox-dark | solarized-dark |
| --- | --- | --- | --- | --- | --- | --- |
| accent (`primary`) on `card` (links in a message) | 4.5 | 8.38 | **3.64** | 6.94 | 9.40 | 4.55 |
| `foreground` on `muted` (code, inline code) | 4.5 | 10.89 | 11.43 | 9.92 | 10.57 | 5.01 |
| `muted_foreground` on `muted` | 3.0 | 4.09 | **2.98** | 3.85 | 4.23 | **2.85** |
| `muted_foreground` on `card` (code label, image text, quote) | 3.0 | 4.46 | 3.21 | 4.20 | 4.65 | 3.01 |

- **Links fail on brindle-light, so one token is added:** `link`, the accent made legible on `card` at 4.5:1 with the existing `Color::legible_on` (unchanged where it already passes, so brindle-dark's links are `#e6a756`). It lands in `brindle-ui`'s `Tokens` (the neutral sets take their `primary`) and the "Legible token pairs" requirement. Since `card` sits between `background` and the foreground, a colour legible on `card` is legible on `background` too; the test checks both, for thoughts, which have no bubble.
- **`muted_foreground` on `muted` fails twice, so the design avoids that pair** rather than adding a code token: the code block's label sits on the bubble's surface (`card`), not on the code's `muted`, and inline code uses `foreground`. The legibility test gains `foreground` on `muted` and `muted_foreground` on `card`.
- `accent` (brindle-ui's hover surface) is never a text colour.
- **Fonts:** code takes the terminal's family in effect (`Settings::get(cx).font().family`, profiles have no family of their own), handed to `Response` as a `SharedString`; sizes stay rem-based, so the zone's rem scope (profile font size) scales code with the text. Headings are bold at 1.25, 1.125 and 1 rem (levels 1–3) and the body size (4–6).

### 11. The dump (`testing R2`; `command-line`)

After each `--- entry` line of an agent message or thought, unchanged (the source, escaped with `{:?}`), come its parts in order, each a block line followed by its links in document order (links nested in its lists, quotes and table cells included):

```
--- entry 2 agent "Try:\n\n```rust\nfn main() {}\n```\nSee [docs](https://example.com)."
--- block entry=2 part=0 kind=paragraph "Try:"
--- block entry=2 part=1 kind=code:rust "fn main() {}\n"
--- block entry=2 part=2 kind=paragraph "See docs."
--- link entry=2 part=2 target="https://example.com" text="docs" action=open
```

- `kind` is `paragraph`, `heading:1`…`heading:6`, `code` (no label) or `code:LANG`, `list`, `quote`, `table` or `rule`. `LANG` is the label with every character other than an ASCII letter, digit, `+`, `#`, `.`, `_` or `-` dropped, and `code` alone when nothing is left, so the field never holds a space or a quote.
- The quoted text is the part's shown text, escaped with `{:?}` as entry lines are: a paragraph's or heading's runs joined (soft breaks as spaces, hard breaks as `\n`, an image as drawn); code exactly; a list's items one per line as `- `, `N. ` (from its start) and then `[ ] ` or `[x] ` for a task, nested blocks indented two spaces; a quote's lines each after `> `; a table's header and rows one per line, cells joined by ` | `; a rule `""`.
- `action` is `link_action(target, plan_here(target))` in words: `open`, `confirm`, `refuse:no-scheme`, `refuse:scheme`, `refuse:other-host`, `refuse:no-drive`, an exhaustive `match` (`railway R3`). The dump passes the plan in, so the words are pure and tested on both platforms' plans.
- A thought prints its parts whether open or closed (its `open=` is on its entry line); user messages, tools, permissions and notices print none.
- `wait_text` holds the same lines, so `--wait-for 'kind=code:rust'` works.
- While the link confirmation is open: `--- confirm link "URI"` after the menu lines, where `--- confirm close` goes, and `--- focus confirm stop=cancel|open modality=…`.

### 12. The `brindle-ui` components (`ports R2`; `ui-components`)

Named after AI Elements' Response and Code Block (anatomy and behaviour, no code): `Response::new(id, blocks)` with `.code_font(family)`, `.link_modifiers(Modifiers)`, `.on_link(|&InlineLink, Modifiers, window, cx|)` and `.on_copy_code(|SharedString, window, cx|)`; `CodeBlock` used by it and usable alone. Paragraphs and headings draw an `Inline` as one `StyledText::with_runs` (bold weight, italic, strikethrough, the code family and `muted` background for code runs, `muted_foreground` for muted runs, `link` colour and underline for link ranges) wrapped in `InteractiveText` when it has links. Lists draw a marker column (`•`, the number, or a checkbox mark drawn from `Task`, not a focusable `Checkbox`) and indent nested blocks; quotes a `border` bar at the start and `muted_foreground` text; rules a `border` line. `Message` is unchanged: the view passes a `Response` as its child. Differences from AI Elements, listed in `ui/README.md`: no syntax highlighting, no HTML, images never loaded, the link gesture is the owner's, the code copy button is pointer-only and always shown, tables have equal-width columns, no Mermaid or math.

### 13. Tests (`testing R1`, `R2`, `R8`, `R13`, `R14`, `R16`, `R17`)

- **Unit, pure:** `render` for every block and inline kind, HTML as text, images, footnote and math syntax kept literal, soft and hard breaks, labels, bare URLs (trailing punctuation, inside code, inside a link), tooltips, `MAX_DEPTH`, `MAX_COLUMNS`, an empty source, the no-panic corpus and random strings; `Inline::new` and `Table::new`; `link_action` for each scheme class and each `Plan` (Windows `Reveal` included); the dump's words for each kind, `LANG` cleaning and escaping; the model's streaming rules and the guard (decision 6); `last_code_block`; the theme's `link` values and the extended legibility test.
- **`TestAppContext`** (brindle-ui): styled runs on one line, nested list indentation, a code block's copy press reported with focus unchanged, a link press with ctrl reported and a release elsewhere not, a short table row.
- **E2E** (`tests/e2e/cases/agent.rs`, the fake ACP agent): `agent-markdown-render` (every block kind and inline style in the dump, a thought's parts, a user message's none, HTML and an image's text), `agent-markdown-stream` (a fence streamed in chunks behind gates: `--wait-for` its unclosed `kind=code:rust` part, then the closed fence and a following paragraph as the next part, and a late reference definition turning earlier text into a link), `agent-markdown-copy` (Copy Last Code Block and Copy Last Agent Message through `--action`, the clipboard read as `agent-copy` reads it), `agent-markdown-links` (inline, reference, bare, e-mail, `javascript:`, `file://otherhost/…`, image and in-code links in `--- link` lines), and `agent-palette-commands` listing Copy Last Code Block. Windows: `windows-agent-markdown` (block lines, and `file:///C:/x` as `confirm` and `file:///etc/hosts` as `refuse:no-drive`), so the plan's other platform runs where it applies (`testing R17`). The archive jobs' filters (`agent-`, `windows-agent-`) pick the new cases up.
- **Perf:** `agent-markdown-stream` in `tests/perf` (decision 6): the fake agent (through `tests/common/fake_agent.rs`) streams a 64 KiB Markdown answer (prose, lists, a 1,500-line fence) in 32-byte `agent_message_chunk`s once a gate opens, and again as one chunk; the case opens the gate when the record file shows the prompt and reports the time from the gate to Brindle's exit on `--wait-for` of the last line, for both, and their difference.
- **Manual** (`CLAUDE.md`'s list, `testing R16`): what needs a real pointer or eyes: the code block's copy button; a link's tooltip, pointing hand with ctrl, ctrl-click, plain click doing nothing, and the confirmation's Open, Cancel and backdrop; the table grid; legibility of links, code and labels in brindle-light, brindle-dark and solarized-dark.

### 14. Docs

`docs/code-map.md` (the `agent/` entry: parts, the renderer and its door; `brindle-ui`: `Response`, `CodeBlock`, the `link` token; links: `plan_here`, `wants_links`, agent links; the link confirmation beside the close confirmation), `docs/architecture/functional-core-imperative-shell.md` (the pure-module table gains `src/agent/markdown.rs`), `ui/README.md`, `README.md` (Markdown in agent zones, ctrl-click and the confirmation, Copy Last Code Block) and `CLAUDE.md` (the dump's `--- block`, `--- link` and `--- confirm link` lines and `confirm stop=open`, the manual checks). Each lands with the task group that makes it true.

### 15. Choices made beyond the user's eight answers

Each is called out so review can overturn it:

1. **A `link` token is added** (decision 10): the rule in answer 8 triggers, since the accent reads 3.64:1 on brindle-light's card.
2. **The code label sits on the card, not on `muted`** (decision 10), to avoid `muted_foreground` on `muted`, which fails 3:1 on brindle-light and solarized-dark, without a second token.
3. **Bare-URL autolinks use the terminal's `find_url`** (decision 4), because pulldown-cmark has no GFM extended autolinks; `www.` forms and bare e-mail addresses stay text.
4. **A soft line break reads as a space** (CommonMark), which changes how a plain multi-line reply looks; the `agent-zones` scenario "Multi-line text" changes with it.
5. **No 4 KiB cut inside one agent block** (decision 5): a long fence or list is one list item. User messages keep step 1's cut.
6. **Thoughts stay one list item** (decision 5).
7. **A `--- link` dump line** (decision 11) beside answer 7's `--- block` line, since no e2e case can click a link in an agent zone (there is no pointer hook outside the terminal grid), and a `--- confirm link` line for manual runs. (Decision 16, added later, gives e2e a pointer hook; the line stays, as the dump's account of every link and what a press would do.)
8. **Bounds on agent text:** nesting at 16 levels and tables at 32 columns, and `render` under `catch_unwind` with the source as the fallback (decision 4, `railway R5`).
9. **Images show their target cut to 120 characters** (a `data:` URI can be megabytes); the link keeps the whole target.
10. **One door for the parser** (decision 2), mirroring the transcript crate's.
11. **The gesture function moves** from `terminal_view.rs` to `terminal/links.rs`, so terminals and agent zones share it.

### 16. An in-process pointer hook for window elements (2026-11-25, added at the user's request; `testing R2`, `R13`, `R16`; `fcis R1`; `types R1`, `R11`)

**Why.** `--hover`, `--click`, `--right-click`, `--press` and `--release` reach only a terminal: `Workspace::debug_pointer` hands the step to the active `TerminalView`, whose `debug_pointer` calls its own mouse handlers with a cell's centre. Everything else that takes a press (a code block's copy button, a link, the link and close confirmations, permission options, the stop button, the queue row, a thought's trigger, a message's copy button, zone headers, the settings dialog) is a GPUI element listener, which runs only through the window's input dispatch. So this change's own pointer features (decisions 7, 8) and the agent zone's pointer checks from `agents-profiles` sit on `CLAUDE.md`'s manual list. The user asked (2026-11-25) for a hook that presses real elements anywhere in the window.

**What GPUI offers.** `Window::dispatch_event(PlatformInput, cx)` is public, and it is exactly what the platform layer calls for real input (`Window::new` hands `platform_window.on_input` a closure that calls it): it stores the event's position and modifiers as the window's `mouse_position` and `modifiers`, hit-tests that position against the **last drawn frame** (`rendered_frame.hit_test`), updates the hovered hitbox and the cursor, and runs the frame's mouse listeners, capture then bubble. It is the mouse counterpart of `dispatch_keystroke`, which `--key` uses. Three facts shape a safe use (read from gpui 0.2.2's `window.rs`, `elements/div.rs` and `elements/text.rs`):
- **It never draws first.** A key event draws a dirty window before dispatching; a mouse event doesn't. A press dispatched right after a change that hasn't drawn hits the old frame.
- **Some listeners exist only after a frame.** `InteractiveText` (links) registers its mouse-up listener only in a frame painted after its mouse-down set `mouse_down_index`; a down and an up dispatched back to back never click a link. `Div`'s click keeps its pending down in shared state, so it would survive, but a real press has frames between its events too.
- **`click_count` and `pressed_button` are the caller's.** The platform counts clicks (`x11/client.rs` `current_count`); the hook sets them itself: 1 for a click, 1 then 2 for a double-click, `pressed_button` on moves while it holds a button.

`Window::on_next_frame` callbacks run at the start of the next frame request, before that frame's draw. So "a frame has drawn since now" is two hops: a callback that registers a second callback; the second runs after the first frame's draw. Before waiting, the hook calls `window.refresh()`, so a frame is drawn even when nothing changed (and Wayland, whose frame callbacks follow commits, gets one).

**The dispatch sequence** (pure: `pointer_hook::plan(action, modifiers, held) -> Vec<HookEvent>`, `HookEvent::{Modifiers(Modifiers), Move, Down(button, count), Up(button, count), Frame}`; the shell runs it):

| Step | Events |
| --- | --- |
| every step | `Frame` first, so the target is looked up in a frame that includes the previous steps |
| `--hover` | `Modifiers(m)` when they differ from the held ones, `Move` (with `pressed_button` while a press is held), `Frame` |
| `--click`, `--right-click` | `Modifiers(m)`, `Move`, `Frame`, `Down(b, 1)`, `Frame`, `Up(b, 1)`, `Frame`, `Modifiers(none)` |
| `--double-click` (new) | as `--click`, then `Down(left, 2)`, `Frame`, `Up(left, 2)`, `Frame`, `Modifiers(none)` |
| `--press` | `Modifiers(m)`, `Move`, `Frame`, `Down(left, 1)`, `Frame`; the left button and `m` stay held |
| `--release` | `Move` (pressed), `Frame`, `Up(left, 1)`, `Frame`, `Modifiers(none)` |

- `Modifiers(m)` is a `PlatformInput::ModifiersChanged`, as a real ctrl-click starts with ctrl going down; it reaches the focused path (so `Response`'s pointing-hand guard and an owner's `on_modifiers_changed` see it). It is sent only when `m` differs from what the hook holds, and released after the gesture, as a hand lifts the key.
- The target is looked up **once**, after the first `Frame`; the events go to that point. If a hover moves what's under it, the press lands on whatever is there now, as a real one would.
- A `Frame` that doesn't come within the step interval abandons the rest of the step (nothing more is dispatched against a stale frame) and records the miss `no-frame` (below).
- The steps run in `run_debug_steps` outside `Workspace::update`, through `cx.update_window(…, |_, window, cx| window.dispatch_event(…))`, as `--key` does, since the listeners update the workspace themselves.
- **Modality.** The hook writes no modality. A dispatched press sets `Modality::Pointer` exactly where a real one does, through the app's own listeners (`focus_from_pointer`, the agent zone's `capture_any_mouse_down`, menus and the command list); one that doesn't is the same finding a real press would give. `--modality` stays for checks that model a press without naming an element, and for keyboard modality.

**The option form: element names, not pixels.** Two forms were weighed:
- *Raw window pixels* (`--click-at X,Y`) with `rect=` added to dump lines. The steps are fixed at launch and the dump comes at the end, so a case would need one run to read a rectangle and a second to press it, against an agent conversation, a layout, fonts and a scale that may differ between the runs. Cases would break on every layout change.
- *Element names* that the app resolves at the step's time to the element's painted bounds (chosen). One run, no layout numbers in a case, and the names come from what the dump already prints (entry and part numbers, focus-stop words, zone ids, tab numbers). It costs a small registry of painted bounds, recorded only in runs that use it.

So the five existing options and a new `--double-click` take `POS` as either form:
- `[MODS-]COL,ROW`: a terminal cell, unchanged (the terminal's own handlers, `TerminalView::debug_pointer`);
- `[MODS-]@NAME[+FX,FY]`: the element painted under `NAME` in the last frame, pressed at fractions `FX,FY` of its bounds (each in `[0, 1]`, default `0.5,0.5`, the zone lines' convention), through `Window::dispatch_event`. The `@` makes the two forms unambiguous whatever the modifiers.

```
brindle agent fake -p 'show me' --step-interval 0.5 \
  --wait-for 'kind=code:rust' --click @copy-code:2.1.0 --action text_input_paste \
  --click ctrl-@link:2.2.0 --wait-for 'confirm link' --click @backdrop+0.02,0.02 \
  --click ctrl-@link:2.2.0 --wait-for 'confirm link' --click @confirm:open --dump-screen-after 2
```

**Names** (`pointer_hook::HitName`, a closed enum with `FromStr` and `Display`, round-trip tested; `types R1`, `R11`): `KIND[:ARGS]`, numbered as the dump numbers them.

| Name | Element |
| --- | --- |
| `tab:N` | the tab face numbered N in the tab lines |
| `profile-menu` | the strip's `⌄` button |
| `menu-item:N` | an open menu's entry N, counted from 0 with separators, as `--- menu … highlighted=N` |
| `zone:ID`, `zone-header:ID`, `zone-split-right:ID`, `zone-split-down:ID` | a zone's body, its header and the header's split buttons |
| `entry:N` | agent entry N's element (its bubble, row or card) |
| `copy-message:N` | entry N's copy button (painted while the entry is hovered) |
| `copy-code:N.P.K` | the copy button of the K-th code block (from 0, document order, nested ones included) of part P of entry N |
| `link:N.P.K` | the K-th link of part P of entry N, in its `--- link` lines' order: the box of the link's first character, so its centre maps back into the link's range |
| `thought:N` | thought N's trigger |
| `permission:N:OPTION`, `stop`, `prompt`, `queued:remove`, `queued:send-again`, `queued:discard`, `scroll-to-bottom` | the agent zone's controls, named as its focus stops are |
| `confirm:cancel`, `confirm:close`, `confirm:open` | the close and link confirmations' buttons, named as their focus stops |
| `backdrop`, `dialog-close` | the open modal's backdrop (the whole window, so pressed with `+FX,FY` outside the dialog) and its × |
| `settings-section:NAME`, `settings-row:NAME`, `settings-edit`, `settings-duplicate` | the settings dialog's section list, a Profiles row and its buttons |

An agent zone's names (`entry:` to `scroll-to-bottom`) can be qualified `ZONE/NAME` (`@2/stop`); unqualified, they resolve in the active zone. Any name resolves to the one element recorded under it in the active zone, else the only one in the window; none is the miss `absent`, more than one `ambiguous`.

**The registry** (`brindle-ui`, `primitives/hit_targets.rs`; `ports R2`: names are plain strings the owner passes, so the UI crate knows no Brindle vocabulary): a window-keyed global, like the focus-visible `Modalities`, that exists only when `main` enables it for a run with an `@` step (as latency probes exist only with their flags), so other runs pay one `has_global` check per target. `.hit_target(name)` on an element adds an absolutely placed, full-size `canvas` whose prepaint records the element's bounds clipped to `window.content_mask()` (a target scrolled out of a list is clipped to nothing, recorded `hidden`, and is the miss `absent` when pressed). Components whose pressable parts the owner can't reach take names from the owner: `CodeBlock::hit_target(name)` for its copy button, `Response::hit_targets(|ResponseTarget| -> SharedString)` with `ResponseTarget::{CopyCode { block, index }, Link { block, index }}` (a link's box from its `TextLayout` in prepaint), and the modal overlay records `backdrop`. `Workspace::render`, which runs on every draw (no view is `cached`), clears the window's entries first, so the registry always describes the last drawn frame.

**The dump.** In a run with an `@` step, `--dump-screen-after` prints one `--- hit NAME rect=X,Y,W,H[ hidden]` line per recorded target (logical window pixels, as `--- grid origin=`, rounded to whole pixels; sorted by name), right before the `--- focus` line, so a failing case shows what was there. Each element step that missed adds `--- pointer-step missed "POS" reason=absent|ambiguous|no-frame` with the `--- wait-for` lines, after the focus line. Runs without an `@` step print neither, so existing cases' dumps don't change.

**Usage errors** (`railway R1`, parsing at the boundary): an `@` step whose name has an unknown kind or malformed arguments, fractions that aren't two numbers in `[0, 1]`, unknown modifiers, a `--release` with no `--press` held before it, a second `--press` while one is held, and a press and release of different forms (a cell pressed, an element released) are refused at launch with status 2, naming the option and value (`pointer_hook::check_holds(&steps)`, pure). Whether a name is painted can only be known while running, hence the miss lines.

**Platforms.** The hook is in-process, so it behaves the same under `scripts/with-xvfb`, on `:0` and on Windows. Two differences, both from what the hook skips:
- On Windows it never goes through `WM_NCHITTEST`: a press on the caption area reaches GPUI's listeners only, never the system's drag, double-click maximize or caption buttons. The `windows-caption-hit-test` case keeps its message probe; the hook doesn't replace it.
- On a real display, a real pointer moving over the window between steps overwrites `mouse_position`; under `scripts/with-xvfb` and on CI's Windows desktop nothing moves. Element names make the window's scale irrelevant.

**The terminal's cell form stays on its own path in this change.** Moving `COL,ROW` onto the window dispatch (the cell's window point from `GridLayout`) would make terminal presses meet overlays, zone focusing and the grid's painted pointer as real ones do, but every existing terminal pointer case (selection drags, links, right-click paste, mouse reporting, `--- grid-pointer`) would change underneath, and `TerminalView::debug_pointer`'s `debug_held` would have to move into the hook. That is its own change, proposed (proposal-only, task 5.12) once the element path has proven itself.

**Rejected:** XTest or posted OS messages (`CLAUDE.md`: never on this desktop; Linux can't post into one window); GPUI's `debug_selector` and `VisualTestContext::debug_bounds` (compiled out without `test-support`, so absent from the release builds the archive jobs test); per-name steps such as `--click-copy-code N.P.K` (a dozen options instead of one grammar).

## Risks / Trade-offs

- [Re-parsing on the UI thread makes long streams janky] → absorbs are merged and paced, one parse per wake; the unit guard pins that only the growing entry is parsed and only changed parts are spliced; the perf case reports the cost; the reader-thread fallback is described (decision 6).
- [A long single block (a big fence) is laid out on each absorb] → GPUI's line layout cache reuses unchanged lines; the perf case's 1,500-line fence measures it.
- [Agent text crafted to break the renderer] → pulldown-cmark is fuzzed upstream; nesting and table width are bounded; `catch_unwind` falls back to plain text; the corpus and random tests run on every `cargo test`.
- [A link's text misleads] → its target shows on hover when it differs, only known schemes open, everything but http and https asks first with the whole target shown, and refusals are the terminal's.
- [Soft breaks as spaces surprise an agent that writes line-broken plain text] → that is CommonMark, which ACP names; agents that mean a line break write a list or a hard break. The source is still what Copy copies.
- [Ctrl-click is undiscoverable in a chat view] → it is the terminal's gesture, the pointer turns into a hand with ctrl held; README documents it. Plain-click opening can be a later change.
- [Equal-width table columns read poorly for narrow-and-wide data] → cells wrap; content-sized columns can come later without a spec change to the grid's existence.
- [The lock moves more than one package] → decision 2's check; the add is reverted and pinned rather than accepted.
- [Memory] → the rendered copy roughly adds one log's worth in the worst case; trimming drops it with its entry.

## Migration Plan

Nothing to migrate: no config key, state file or protocol changes, and the server and transcript are untouched, so any Brindle meets any server as before (`process R4`–`R6`, `R15` not engaged); a core restart, a Brindle crash or an upgrade only re-renders from the replayed log. Rolling back is reverting the change, `Cargo.lock` included.

## As built

**Section 1, shared contracts (2026-11-25).**
- The lock moved as decision 2 says: one `[[package]]` (`pulldown-cmark` 0.13.6 on `bitflags` 2.9.4, `memchr` 2.7.6, `unicase` 2.8.1) and its name in `brindle`'s list.
- **The table's alignment is `ColumnAlign`,** not `Align`: `brindle-ui` already exports `primitives::placement::Align` (a box against its trigger, `Center` by default) at its root, and two `Align`s would make the root's glob re-exports ambiguous. Same variants as decision 3 (`Start`, the default, `Center`, `End`).
- **`Inline` has two more ways in, both valid by construction:** `Default` (the empty text, no runs, no links; `Table::new` pads short rows with it) and `Inline::plain(text)` (one unstyled run, no links; for the renderer's fallback paragraph). `Inline::new` keeps its links sorted by start, so `links()` is in document order. An `InlineError` carries the offending values (`RunsDontCoverText { runs, text }`, `NotCharBoundary { at }`, `LinkOutOfRange { range, text }`, `LinksOverlap { first, second }`) and implements `Display` and `Error`. `Table` reads through `width()`, `columns()`, `head()` and `rows()`, and `Table::new` also fits `columns` to the header's width (missing alignments are `Start`).

**Section 2, pure rendering and the chat model (2026-11-25).**
- **Bare URLs go through `links::find_urls`,** a new pure function in the link seam beside `find_url`: every URL in a run of text, in order and apart, the first scheme that starts a word in each run of URL characters and then again after its end, each one `find_url`'s. Calling `find_url` at every index (or every word start) is quadratic in a run's length, which a long message can make large. `find_url`'s trailing-bracket trimming now counts the run's brackets once instead of on every dropped character, so it is linear too; the terminal's results are unchanged (its tests pass as they were).
- **Inline HTML keeps the styles around it** (it is a run of the enclosing emphasis, if any); its tags are never interpreted. An HTML block's final line break is dropped from its paragraph.
- **An empty link (`[](url)`) shows its target as its text,** so there is something to press. **An image inside a link** adds no link of its own; the enclosing link covers its text. **A link inside a link or an image** (which CommonMark doesn't produce) is kept as text.
- **Code text is the parser's exactly,** so an indented block that ends the input without a line break has none.
- **Bare URLs are looked for only in plain text holding a colon** (every scheme ends in one), which keeps the walk about four times faster on prose.
- **A splice starts at the first item that differs, which includes the end of the bubble moving:** when a message gains or loses parts, the old last part's item is spliced too (its `last` flag, so its `BubblePart`, changes) even though its source didn't. The guard (2.7) asserts exactly that start: the first changed part, or the old last part when the count changed. An absorb that leaves an agent message's parts as they were splices nothing; the parts before the first change keep their cached `Arc`s.
- **`describe` and `wait_text` take the plan** as `&dyn Fn(&str) -> Plan`; `ZoneTreeView` passes `links::plan_here`, the tests `links::plan` with a fixed host and platform. The `Speaker` enum is gone (the split bodies say who wrote a part).
- **The view is a stopgap until 4.1:** `AgentPart` draws its `shown_text` in the `Message` bubble as plain text, and a thought its blocks' shown texts joined by blank lines. `last_code_block` is `cfg_attr(not(test), allow(dead_code))` until 4.2 uses it.
- **`agent-scroll`'s fixture is 200 paragraphs now,** not 200 lines: with soft breaks as spaces (choice 4) its lines read as one short paragraph and the conversation no longer outgrew the zone. Its intent, a conversation longer than the zone, is unchanged.
- **The guard test takes about 20 s in a debug build:** 4,096 absorbs each render up to 256 KiB again, and the walk is unoptimized `brindle` code (pulldown-cmark itself, at `opt-level` 2, is under a tenth of it). One render of the 256 KiB message takes about 12 ms in a debug build.

**Section 3, `brindle-ui` components (2026-11-25).**
- **`Response::new(id, blocks)` takes `Arc<RichBlock>`s** (`impl IntoIterator<Item = Arc<RichBlock>>`), the model's cached values, so a render clones pointers, not blocks: an `AgentPart` passes `[block.clone()]`, a thought `blocks.iter().cloned()`. The response adds no outer margin (its blocks are `gap-3` apart), so an owner drawing one block per list item spaces the items itself.
- **`on_link` gets the modifiers held at the release** (`Window::modifiers`, which gpui sets from each mouse event), since `InteractiveText::on_click` passes only the range's index.
- **The pointing hand follows `.link_modifiers` through a guard:** `InteractiveText` sets the hand over every clickable range, so a canvas over each linked text (a hitbox that blocks nothing, painted after it) sets the plain pointer while the modifiers aren't held, and a pointer move whose modifiers change that redraws the window. A modifiers change with the pointer still reaches only the focused path, so the owner redraws on its own `on_modifiers_changed` (4.3). The modifiers are held when each one named is (`holds_modifiers`; others may be too), so ctrl-alt shows the hand although `links::wants_links` refuses it.
- **A link's colour wins over the muted one**, so an image's `[image: ALT]` text is drawn as a link; strong, emphasis, strike and code still apply under a link.
- **`CodeBlock::new(id, Code)`** with `.font(family)` and `.on_copy(…)`; it shows its text without the one line break that ends it (`shown_code`, an owned copy of the text only when it ends in one) and copies the whole text. Its copy button sits in a wrapper that prevents the press's default, so no ancestor that tracks focus takes it.
- **The tooltip's panel has a `debug_selector`** (`"tooltip"`, nothing in a release build), so a `TestAppContext` test can see whether a hover built one.
- **The gallery's copy-button check is a pointer press,** so it runs on Windows only and skips elsewhere, like the gallery's other pointer checks; `TestAppContext` tests cover every Verify clause of 3.1–3.4 on each platform.

**Section 4, the agent zone (2026-11-25).**
- **A message's parts are padded as one bubble.** Every list item had `py_1` around it, which opened a gap in the bubble between parts; with one part per Markdown block that gap showed between every paragraph. `view::item_padding` pads a message part's item only on its bubble's outer edges (others keep `py_1`), and a part after the first gets `pt_3` inside the bubble, the response's own `gap-3`. User messages' parts follow the same rule.
- **The link decision is two pure steps in `view.rs`:** `link_press(target, modifiers, plan) -> Option<LinkAction>` (the gesture, then `link_action`) and `link_step(target, action) -> LinkStep::{Open, Confirm, Fail(text)}` (a refusal's `failure_text`); `AgentView::press_link` only applies the step.
- **`links::open_reporting(uri)`** opens now and answers the failure's text later (logged); the terminal view, the agent view and the link confirmation's Open share it, so the failure path is one.
- **`ConfirmLink { uri, back }` carries the zone's prompt focus handle,** and the workspace focuses it when the confirmation closes, whichever way, rather than relying on the overlay's own restore (the press might have moved focus).
- **The link confirmation has its own modal slot** (`Workspace::link_confirm`, `Overlay::LinkConfirm { uri, back }`, `OverlayKind::LinkConfirm`, which replaces what is open) and shares `_confirm_subscriptions` with the close confirmation, since one overlay is open at most. It holds the workspace's actions back as the close confirmation does. The link is drawn on `muted` (foreground on muted, a pair the legibility test checks).
- **The view redraws on every modifiers change** (`on_modifiers_changed` → `cx.notify()`), not only ctrl's: modifier changes are rare, and filtering would need state for no gain.
