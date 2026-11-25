# Design

## Context

See proposal.md for why. Facts this design rests on, from the code on `feat/agents-profiles` (2026-11-25), the designs stacked under this change, the pinned schema crate's sources and the captured agent traffic:

- **The reducer drops tool content today.** `remote-agent/src/acp/wire.rs` decodes `tool_call` and `tool_call_update` through the schema crate's v1 types into Brindle's own `ToolCall` and `ToolChange` (call id, title, kind, status), and a permission request's `toolCall` into a `ToolChange` used only for its title. `brindle-transcript`'s `Entry::Tool` is `{ call, title, kind, status }` and `Event::ToolChanged` carries the three fields that can change. `reduce.rs` cleans every text (`clean`: control characters other than `\n` and `\t` replaced) and bounds ids and titles (`bound`).
- **The reducer is already version-free in shape.** `reduce.rs` reads only `wire`'s own closed types and writes through `rpc.rs`'s encoders. What is v1-specific is inside `wire.rs` (the only file naming `agent_client_protocol_schema::v1`, plus the method names and field names it reads by hand) and `rpc.rs` (`initialize`, `new_session`, `prompt`, `cancel`, `permission_answer`, and `PROTOCOL_VERSION = 1`). The checkpoint records the negotiated ACP version (`agents-sessions`, decision 11).
- **One event fits one frame.** Transcript events travel packed in `TRANSCRIPT` frames (`pack_transcript`, bounded by `MAX_FRAME_PAYLOAD_SIZE`); text that grows is split into `TEXT_CHUNK` = 64 KiB pieces. The log is capped at 16 MiB (`LOG_LIMIT`), oldest entries dropped whole. The reducer runs on the core's one thread (`process R8`), so its work per line must be bounded.
- **What real ACP v1 agents send** (`remote-agent/tests/fixtures/acp/`, all captured with a rejected write or command):

  | Agent | Tool content | Permission's `toolCall` | Other |
  | --- | --- | --- | --- |
  | claude-agent-acp 0.112 | `diff` (`path`, `oldText: null`, `newText`) on the `tool_call`; a text block in a ```` ``` ```` fence for the result | `content` with the diff, `locations`, `rawInput` | `rawOutput` repeats the text |
  | antigravity 1.5.16 | none on the `tool_call` | `content` with the diff (`oldText` absent) | `rawInput`, `locations` |
  | codex-acp 2.12 | `terminal` with a `terminalId`, and `_meta.terminal_info` | `rawInput` (`command`, `cwd`), no content | Brindle offers no terminal capability |
  | copilot 1.0.115 | none | `rawInput` (a patch as text), `locations` | `rawOutput` |

  No capture holds an allowed command's output or an edit of an existing file. Task group 1 captures both before the mapping is written.
- **ACP v2 differs** (schema crate 1.22.0, `src/v2/tool_call.rs`, `terminal.rs`; v2 is in Preview): a tool call is an upsert with patch semantics (omitted keeps, `null` clears, a value replaces; collections replace whole); `tool_call_content_chunk` appends one content item; a `diff` is `changes` (add, delete, modify, move, copy, each with absolute paths) plus an optional `patch` whose only defined format is Git's `--patch` text; `terminal` content names an agent-owned terminal whose `command`, `cwd`, `output` and `exitStatus` arrive in `terminal_update` and whose output is appended by `terminal_output_chunk`; statuses add `paused` and `cancelled`; almost every enum accepts unknown values.
- **`agents-markdown`** gives `src/agent/markdown.rs` (`render(source) -> Vec<Part>`), `brindle-ui`'s `Response` and `CodeBlock` (a label, a pointer-only copy button, text in the terminal's family on `muted`), and a chat model whose list items are parts of an entry.
- **`agents-log-paging`** makes the fold ignore events for entries below the client's floor and returns an entry's current state from a fetch, tool and permission entries included.
- **Theme.** `src/theme.rs` has the theme's 16 ANSI colours; `ui_tokens` derives `destructive` from red, and status colours use green for done and red for error.

## Goals / Non-Goals

**Goals:**
- A user reads an edit before allowing it, and can see afterwards what a tool call changed, ran and printed.
- A tool call's content is Brindle's own data: Brindle and later clients never learn which ACP version produced it, and a version's differences are written once, in that version's module.
- Everything an agent sends is bounded in size and in the work it costs the core, and one unreadable item loses only itself (`railway R5`).
- A long diff or output costs the view only what is on screen.
- Every decision is a pure function or model with unit tests; the dump and e2e cases pin the rest.

**Non-Goals:** syntax highlighting; word-level and side-by-side diffs; command output through the terminal emulator (colours, cursor movement, a size); opening a location or a diff's file, and links in a detail other than those Markdown text gives; applying, reverting or editing a diff; images, audio and embedded resources in tool content (a placeholder, as in messages); text selection and find in a detail; tool details as keyboard focus stops; a tool call's v2-only statuses and v2's session references (subagents), cancel controls and terminal sizes (`agents-acp-v2`); ACP's `fs` and `terminal` client capabilities. No new client message, config key or state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `ToolItem`, `FileChange`, `FileOp`, `Hunk`, `DiffLine`, `Output`, `Exit`, `Location`; `Event::ToolItems`, `Event::ToolOutput`; the fold and its bounds | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| The JSON-RPC envelope (`Id`, `Message`, `decode`) | pure | `brindle-remote-agent`, `acp/envelope.rs` (from `wire.rs`) |
| The forms the reducer reads and the requests it asks for (`Initialized`, `Update`, `ToolChange`, `Patch`, `PermissionRequest`, `Request`, …) | pure, plain data | `acp/forms.rs` (from `wire.rs` and `rpc.rs`) |
| `Dialect`, the one `match` on the ACP version | pure | `acp/dialect.rs` (new) |
| ACP v1: decoding its messages into the forms, encoding the requests | pure | `acp/v1/` (`decode.rs`, `encode.rs`), the only files naming `agent_client_protocol_schema::v1` |
| Line diff into hunks | pure | `acp/diff.rs` (new) |
| Command output as plain text (escape sequences, carriage returns) | pure | `acp/plain.rs` (new) |
| Tool handling: upsert, bounds, the permission's tool call | pure | `acp/reduce.rs` |
| The tool detail's parts, open state, summaries, copy text, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| Drawing details, the copy buttons, the toggle action | shell (entity) | `src/agent/view.rs`, `src/agent/mod.rs`, `src/actions.rs` |
| `Tool` (trigger, summary), `Diff` | components | `brindle-ui`, `ui/src/components/tool.rs`, `diff.rs` (new) |
| `diff_added`, `diff_removed` | tokens | `ui/src/theme.rs`; values in `src/theme.rs` `ui_tokens` |

Crate references don't change (`ports R1`). One closed list does (`ports R3`): `brindle-remote-agent` gains `similar` (decision 4). `acp/{envelope, forms, dialect, diff, plain}.rs` and `acp/v1/*.rs` are whole pure modules and join `PURE_MODULES` (`fcis R2`, `R3`, `R7`).

## Decisions

### 1. Brindle's own forms, and why the diff is made on the server

A tool call's content is kept as transcript forms that name no ACP version (`types R1`, `R2`, `R7`):

```rust
pub enum ToolItem {
    Text { text: String, cut: bool },      // Markdown source; `cut` when its end was dropped
    Diff(FileChange),                       // one file
    Output(Output),                         // one command's output
}
pub struct FileChange { pub op: FileOp, pub path: String, pub hunks: Vec<Hunk>, pub omitted: u32 }
pub enum FileOp { Add, Delete, Modify, Move { from: String }, Copy { from: String }, Other { name: String } }
pub struct Hunk { pub old_start: u32, pub new_start: u32, pub lines: Vec<DiffLine> }
pub enum DiffLine { Context(String), Added(String), Removed(String) }
pub struct Output {
    pub command: Option<String>, pub cwd: Option<String>,
    pub text: String, pub omitted: u64,     // bytes dropped from the start
    pub exit: Option<Exit>,
}
pub enum Exit { Code { code: i32 }, Signal { name: String } }
pub struct Location { pub path: String, pub line: Option<u32> }
```

`Entry::Tool` gains `content: Vec<ToolItem>`, `more: u32` (items dropped past the item bound), `locations: Vec<Location>`, `input: Option<String>` and `result: Option<String>`, each defaulted and omitted when empty, so every existing fixture still decodes and encodes byte for byte.

- **What the types rule out.** A change is one operation, and only a move or copy has a second path. A hunk line is context, added or removed, never two. An exit is a code or a signal. "No command", "no exit yet" and "no raw input" are `None`, not empty strings. `FileOp::Other { name }` keeps an operation a later ACP adds, as `StopReason::Other` does.
- **Encoding.** `DiffLine` is one JSON string per line whose first character is a space, `+` or `-`; any other first character, or an empty string, is a `DecodeError`. That is a third of the size of an object per line and is parsed at the boundary (`types R5`).
- **A change with no hunks** is valid: a binary file, a v2 change with no patch text, a move with no edit. The row then shows the operation and the path alone.
- **Why hunks, not the old and new text.** ACP v1 gives a diff as the whole old and new file. Keeping those would put two copies of every edited file in a 16 MiB log and make every client compute the diff. ACP v2 gives changes and patch text, with no file texts at all. Hunks are the form both reduce to, so the core computes them for v1 (decision 4) and reads them from the patch for v2 (decision 9). *Rejected:* keeping v1's texts and v2's patch as two item kinds, which would put the version difference in every client.
- **Forms v1 can't fill are added now.** No v1 agent can report a delete, move or copy, and decision 6 says when v1 fills an `Output`. They are in the forms anyway, with unit tests and fixtures, because protocol 4 can still be amended (decision 8) and adding them with `agents-acp-v2` after a release would cost a protocol version. Spec scenarios use only what a v1 agent can send.
- **Text is Markdown source**, as ACP says of text content blocks, and the Claude adapter's fenced results show. A content block that isn't text becomes the placeholder text the reducer already uses for it in messages.

### 2. One implementation per ACP version (`ports R4`–`R8`, `types R2`)

The user's rule: v1 and v2 are both supported for good, and no `if v1` / `if v2` spread through the code. `remote-agent/src/acp/` becomes:

```
acp/
  envelope.rs    JSON-RPC: Id, Message, decode. No ACP version in it.
  forms.rs       What the reducer reads (Initialized, NewSession, Update, ToolChange, PermissionRequest, …)
                 and what it asks to be written (Request::{Initialize, NewSession, Prompt, Cancel, PermissionAnswer, Error}).
  dialect.rs     pub enum Dialect { V1 }  and the only `match` on it.
  v1/decode.rs   ACP v1 → forms.     v1/encode.rs   Request → ACP v1 lines.
  diff.rs  plain.rs  lines.rs  rpc.rs  reduce.rs  checkpoint.rs
```

- **`Dialect` is a closed enum, not a trait** (confirmed by the user, 2026-11-25; `ports R6`: no indirection to make a decision mockable; `types R2`). Its methods (`session_update`, `permission_request`, `initialize_result`, `new_session_result`, `prompt_result`, `request_kind`, `notification_kind`, `encode`) each hold one exhaustive `match self` that calls the same-named item of that version's module, as a per-platform module set exports the same names (`ports R5`). Adding `V2` then fails to compile until every method has its arm.
- **Chosen once.** `Dialect::newest()` writes `initialize`. The answer's `protocolVersion` is read from the raw result (it is a number in every version) and `Dialect::from_version(n) -> Result<Dialect, Unsupported>` picks the session's dialect, which the session keeps and the checkpoint already records as the version. `Unsupported` is today's "the agent speaks ACP version N" failure, unchanged. Until `agents-acp-v2`, `newest()` is `V1` and `from_version` accepts only 1.
- **`reduce.rs` and `rpc.rs` name no version.** `rpc.rs` keeps ids, pending requests and held permissions; the line for a request comes from `dialect.encode(Request)`. The reducer holds a `Dialect` value and calls its methods; it never matches on it.
- **`Patch<T> { Keep, Clear, Set(T) }`** carries a field's change in `ToolChange`. v1 produces only `Keep` and `Set`; v2's `null` is `Clear`. The reducer handles all three once.
- **The rule and its check.** `docs/architecture/ports-and-adapters.md` gains a rule: an ACP protocol version has one module under `acp/`; that module alone names its `agent_client_protocol_schema` version module, its method names and its field names; `Dialect`'s variants are matched only in `acp/dialect.rs`; everything else in the crate reads `acp/forms.rs`. `ports R6`'s text gains "a protocol version an outside program chooses is a seam of the same kind as a platform". `tests/architecture.rs` gains `acp_versions_stay_in_their_modules`: `agent_client_protocol_schema::v1` only under `acp/v1/` (and `::v2` only under `acp/v2/`), `Dialect::V` only in `acp/dialect.rs`, and neither `v1::` nor `v2::` in `reduce.rs`, `rpc.rs` or `checkpoint.rs`. `only_the_acp_modules_name_the_schema_crate` stays.
- **The move changes no behaviour.** It is its own commit, before any tool content: every existing unit test, the capture replays and `remote-agent/tests/agents.rs` pass unchanged apart from paths. It moves whatever v1 code the stack's tip has, including what `agents-session-recovery` and `agents-login` add.
- *Rejected:* a `trait Dialect` with `V1` and `V2` structs behind `dyn` (no second implementation is swapped in for tests, and a closed enum makes a missing arm a compile error); version checks at each use (the rule's whole point); a separate OpenSpec change for the move (it changes no requirement, so it would have no spec delta, and this change is the first that needs it).

### 3. The v1 mapping (`acp/v1/decode.rs`; `railway R5`)

| ACP v1 | Form |
| --- | --- |
| `content` on `tool_call` and `tool_call_update` (replaces the list) | `ToolChange.content = Set(items)`; absent is `Keep` |
| `{ "type": "content", "content": { "type": "text", … } }` | `ToolItem::Text` |
| `{ "type": "content", … }` with another block | `ToolItem::Text` holding the placeholder |
| `{ "type": "diff", "path", "oldText", "newText" }` | `RawDiff { path, old: Option<String>, new }`, which the reducer turns into a `FileChange` (decision 4): `Add` when `oldText` is absent or `null`, else `Modify` |
| `{ "type": "terminal", "terminalId" }` | nothing, unless decision 6 finds output to show |
| any other `type` | `ToolItem::Text` holding `[content Brindle doesn't show: TYPE]` |
| `locations` | `ToolChange.locations` |
| `rawInput`, `rawOutput` | `ToolChange.input`, `.result`: a JSON string as its text, anything else as indented JSON |
| a permission request's `toolCall` | the same `ToolChange` (decision 7) |

Content items are decoded one by one from the raw JSON, as permission options are: an item that doesn't decode is left out and counted in one notice per tool call ("N content items of tool call TITLE couldn't be read"), where the schema crate's enum would lose the whole update.

### 4. The line diff (`acp/diff.rs`, pure; `fcis R1`–`R3`, `process R8`)

`pub fn hunks(old: &str, new: &str) -> Diffed` where `Diffed { hunks: Vec<Hunk>, omitted: u32 }`.

- **The `similar` crate does the comparing** (the user's choice, 2026-11-25, over a hand-written Myers). Facts read from the crate's sources and `cargo info` on 2026-11-25: `similar` 3.2.1, Apache-2.0, `rust-version` 1.85 (the pin is 1.99.1); features `default = [std, text]`; every dependency (`bstr`, `hashbrown`, `serde`, `unicode-segmentation`, `web-time`) is optional. Its default `Algorithm::Myers` trims the common prefix and suffix, then follows Git's xdiff policy: it accepts a long snake once the search is expensive and forces a split at a size-dependent work limit, so its work is bounded without a deadline and its result is a correct, sometimes non-minimal, script.
- **The dependency.** `similar = { version = "3.2.1", default-features = false, features = ["text"] }` on `brindle-remote-agent`, added with `cargo add similar@3.2.1 --no-default-features -F text -p brindle-remote-agent`. Without `std` the crate's deadline type is `()`, so it can't read a clock and `diff.rs` stays pure (`fcis R2`). If `text` turns out not to build without `std`, the default features are used instead and `diff.rs` never calls `deadline` or `timeout`, which the architecture check below pins. `CLAUDE.md`'s lock rule holds: `git diff Cargo.lock` shows exactly one added `[[package]]` and the one added name in `brindle-remote-agent`'s dependencies; anything else moved means reverting and pinning, never regenerating.
- **The closed list and one door** (`ports R3`). `similar` joins `brindle-remote-agent`'s list in `tests/architecture.rs`, `ports R3`'s text and `.claude/rules/remote-agent.md`. Only `acp/diff.rs` names it (`only_the_diff_module_names_similar`, as only the ACP modules name the schema crate), so swapping the library touches one file. The static musl helper must still build, and its size is noted in the PR.
- **No clock, so the same result twice.** No deadline or timeout is ever set: a time limit would make a restarted core compute different hunks from the same bytes (decision 14).
- **What `diff.rs` adds around it.** `TextDiff::configure().algorithm(Algorithm::Myers).diff_lines(old, new)` and `grouped_ops(3)` give the groups; `diff.rs` turns each into a `Hunk` (1-based starts, each line without its line break; a trailing `\r` stays part of the line and is cleaned later with the rest), keeps lines until their text passes `ITEM_LIMIT` (decision 5) and counts the rest in `omitted`. As a guard on memory, texts with more than `MAX_DIFF_LINES` = 200,000 lines together aren't compared: the change becomes one hunk removing the old lines and adding the new, cut the same way.
- Tests: `apply(old, hunks) == new` for a table of edits and for 10,000 deterministic pseudo-random pairs; an added file (`old` absent) is one hunk of added lines; line numbers are 1-based and match the texts; a text with and without a final line break; 50,000 lines against 50,000 unrelated lines finishes and is cut at `ITEM_LIMIT`; the `MAX_DIFF_LINES` guard; the same inputs give the same hunks twice.
- *Rejected:* a hand-written Myers (no dependency, but our own code to get right where a maintained library already carries xdiff's safeguards); `similar`'s deadline (a clock in the reducer, and results that differ between runs).

### 5. Bounds (`railway R5`, `process R9`)

| Bound | Value | Past it |
| --- | --- | --- |
| `ITEM_LIMIT`: one item's text (a `Text`'s text, a `FileChange`'s lines, an `Output`'s text) | 64 KiB | `Text`: the end is dropped, `cut` set. `FileChange`: later lines dropped, counted in `omitted`. `Output`: whole lines dropped from the start, bytes counted in `omitted` |
| `MAX_ITEMS`: items in a tool call | 32 | later items dropped, counted in `more` |
| `CALL_LIMIT`: a tool call's items together | 256 KiB | later items dropped, counted in `more` |
| `RAW_LIMIT`: each of `input` and `result` | 16 KiB | the end is dropped, with `…` |
| `MAX_LOCATIONS` | 32 | later ones dropped |
| a path, a command, a `cwd` | as titles are (`bound`) | cut as titles are |

- `ITEM_LIMIT` equals `TEXT_CHUNK`, so any one item fits one event and one frame.
- The reducer applies every bound before an event is made, except an output's tail, which the fold applies (decision 8), so the core's log and a client's copy can't differ.
- A tool entry is at most about 300 KiB, so `LOG_LIMIT` still holds at least 50 of the largest. The log's trim is unchanged: oldest entries whole.
- Every text is cleaned as today; paths, commands and raw values included.

### 6. Command output (`acp/plain.rs`; needs the capture of task 1.1)

`Output.text` is plain text. `plain::feed(state, bytes) -> String` removes escape sequences (CSI, OSC, DCS and the other string forms, single-character escapes), keeps `\n` and `\t`, and resolves a carriage return by keeping only what follows the last one on a line, so a progress bar leaves its final state. Its state (inside a sequence, the current line) is part of the session and of the checkpoint, so a sequence split across two updates is still removed.

How a v1 agent delivers an allowed command's output isn't in any capture. Task 1.1 records it for the agents we can run. The mapping then follows these rules, in the v1 module only:
- output that arrives as a text content block (expected of the Claude adapter) stays a `ToolItem::Text`, with nothing more to do;
- output that arrives only through the `_meta` terminal convention some v1 agents share (`terminal_info`, `terminal_output`, `terminal_exit`, which Codex's `terminal_info` suggests) becomes an `Output` item, if the capture shows it is sent to a client that offered no terminal capability, or only needs a `_meta` capability flag to be sent, which `initialize` then carries. The user decided this on 2026-11-25: read the convention when a capture shows an agent's output reaches Brindle no other way. It is outside the ACP spec, so it stays inside `acp/v1/`, is pinned by the captures, and reads only those three keys;
- a bare `terminal` item with no output to show adds nothing.

**The `terminal` client capability stays unoffered here** (user, 2026-11-25). It is ACP v1's way for the agent to have the client run a command (`terminal/create`, `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, `terminal/release`), which changes who runs the command and what an agent can make the server do, and v2 has no such methods. Task 1.1 also records which agents call `terminal/create` when it is offered, as evidence for the proposal-only change `agents-client-terminals`.

Either way v2 fills the same `Output` (decision 9), so its transcript forms and fold are added now. If the answer is that v1 never fills an `Output`, nothing can produce one until `agents-acp-v2`: the requirement "Command output is kept as plain text", `plain.rs`, the zone's output block and their tests then move to that change, and this one keeps only the forms (tasks 3.2, 4.1's output half, 7.3, 8.2 and 8.4's output half go with them).

### 7. The reducer (`acp/reduce.rs`)

- **Upsert.** `tool_change` applies `title`, `kind`, `status` as today and now `content`, `locations`, `input` and `result`, each a `Patch`. It emits `Event::ToolChanged` for the scalar fields that differ and `Event::ToolItems` for content (decision 8). An update that changes nothing emits nothing, as today.
- **A diff is diffed once.** `RawDiff` is turned into a `FileChange` by `diff::hunks` in the reducer, then dropped. An agent that resends the same content list (the Claude adapter resends it with each update) produces equal items, and equal content emits no event.
- **A permission request's `toolCall`** is applied to the tool call it names before the permission entry is added: its `content`, `locations` and `input` (never its title, kind or status, since the request's title may differ, as Copilot's does). When the log has no entry for that call, one is added from the request's fields first. So the entry a permission row points at holds whatever the agent sent in either place.
- **Output.** A form `OutputAppended { call, item, bytes }` or `OutputSet { call, item, text, exit, … }` from a dialect goes through `plain::feed` and becomes `Event::ToolOutput` or a replaced item.
- **Entry size.** The transcript recomputes a tool entry's kept size when its content changes, so `LOG_LIMIT` counts it.

### 8. Events and the fold (`brindle-transcript`; `process R4`–`R6`)

```rust
Event::ToolChanged { id, title?, kind?, status?, locations?: Vec<Location>, input?: Option<String>, result?: Option<String>, more?: u32 }
Event::ToolItems  { id, from: u32, items: Vec<ToolItem> }   // the content from index `from` on is replaced by `items`
Event::ToolOutput { id, item: u32, text: String, exit?: Exit }  // appended to an Output item's text
```

- `ToolItems` covers every case with one rule: a replaced list is `from: 0`; an appended item is `from: len`; `from: 0` with no items clears. The core sends one item per event when two wouldn't fit `TEXT_CHUNK` together.
- `ToolOutput` appends, then the fold drops whole lines from the start while the text is over `ITEM_LIMIT`, adding their bytes to `omitted` (a single line longer than the limit is cut at a character boundary). The rule is in the shared fold, so both sides compute the same text.
- **Fold errors** (`railway R2`): `NotATool` as today; `ItemsOutOfRange { id, from, len }` when `from` is past the end; `NotAnOutput { id, item }` for `ToolOutput` on a missing or other item. The log is unchanged on an error, as for every `FoldError`.
- **Replay.** `Added` carries the tool entry with no content, then one `ToolItems` per item (an output's text is already within the limit). `fold(replay(t)) == t` holds and its property test gains tool content.
- **Versions.** Client protocol 4 has not been released, and the user decided on 2026-11-25 that the agent series amends it instead of adding versions. So protocol 4 gains two event kinds and optional fields, and "Transcript event forms are fixed for each protocol version" is unchanged: these are protocol 4's forms. New files are added to `tests/fixtures/v4/` for each new form; none is edited, and the old ones must still pass, which pins that the new fields are omitted when unset. The protocol's own transcript tests don't change: `TRANSCRIPT` carries events it doesn't read. If a release ships protocol 4 before this change lands, the new kinds need protocol 5 and a gate, and this decision is reopened.
- **The checkpoint** keeps format 1, amended the same way: the log's entries carry the new fields, and the session state gains the output filter's state per open output (absent means "at a line's start, in no sequence"). A checkpoint written before this change decodes unchanged.
- **Paging** (`agents-log-paging`): its fold ignores an event for an entry below the client's floor, which covers both new kinds by their `id`, and its fetch returns a tool entry's current content. Its page must also reach the tool entry that a pending permission names, which is normally the entry before it (task 5.6).

### 9. What ACP v2 maps to (for `agents-acp-v2`)

| ACP v2 | Form |
| --- | --- |
| `tool_call_update` upsert: omitted, `null`, value | `Patch::Keep`, `Clear`, `Set`; a concrete list replaces (`ToolItems from 0`) |
| `tool_call_content_chunk` | one item appended (`ToolItems from len`) |
| `diff { changes, patch }` | one `FileChange` per change, its `FileOp` from `operation`; hunks read from the patch's section for that path (a pure `acp/v2/patch.rs` parser of Git's `--patch` text, bounded as decision 5); no patch, or a format other than `git_patch`, gives no hunks |
| `terminal` content, `terminal_update`, `terminal_output_chunk` | one `Output`: `command`, `cwd`, `output` replacing the text, a chunk appending to it, `exitStatus` as `Exit` |
| `content` text, other blocks, `session` references, unknown content | as v1: `Text`, or a placeholder |
| `paused`, `cancelled`, unknown statuses and kinds | new `ToolStatus` values, which `agents-acp-v2` adds |

Nothing in `brindle-transcript`, `reduce.rs` or Brindle changes for these: `agents-acp-v2` writes `acp/v2/` and `Dialect::V2`'s arms.

### 10. The chat model (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **Parts.** A tool entry's first list item is its row. When open, its detail follows as more items of the same entry: one per text item (rendered with `markdown::render`, under `catch_unwind` as messages are), one per `DETAIL_LINES` = 50 lines of a diff or an output, one for the locations, and one each for the raw input and result. So the scroller lays out only the parts on screen, however long the diff.
- **Open state.** `tools_open: BTreeMap<EntryId, bool>`, as thoughts' `toggled`: a tool row is closed until the user opens it. `toggle_tool(id)` flips one; `toggle_tool_details()` opens every tool call that has a detail when any is closed, else closes them all. Both return `Splice`s. The state is the zone's and isn't kept across a restart.
- **The summary** on a row: `+A −R` over a call's diffs (lines added and removed, hunk lines only), and `exit N` or the signal's name for its last output with an exit. `summary(entry) -> Option<Summary>` is pure.
- **What a detail holds**, in order: the content items; the locations; then, only when there are no content items, the raw input and the raw result (`detail(entry) -> Vec<DetailPart>`, pure).
- **A pending permission** whose `call` names a tool entry the model holds shows that entry's detail parts as items of the permission entry, before its confirmation, whatever the tool row's own open state. Once answered, they go, and the tool row keeps the detail. A permission naming no call, or one the model doesn't hold, shows as today.
- **Copy text:** a diff copies as unified diff text (`--- path`, `+++ path`, `@@ -a,b +c,d @@`, the lines), an output as its text, a text item as its source (`copy_text(part)`, pure).
- **Marks for what was left out** are lines of the detail: "N more lines not shown", "the first N bytes of output aren't kept", "N more items not shown".

### 11. The zone and `brindle-ui` (`ports R2`)

- **`Tool`** gains `.summary(text)`, `.open(Option<bool>)` and `.on_toggle(…)`: with `Some`, the row shows a chevron and a pointer press on it asks the owner to flip it, as `Reasoning`'s trigger does. It still takes no focus.
- **`Diff`** (new, plain data in): a header with the operation's word, the path (and `from → to` for a move or copy) and a pointer-only copy button; then its lines, each with a sign column (` `, `+`, `−`), the old and new line numbers, and the text in the owner's code family, added lines on `diff_added`, removed on `diff_removed`. Hunks are separated by a `border` line. The sign column means colour is never the only cue.
- **Output** draws with `agents-markdown`'s `CodeBlock`: the command as its label, the exit beside it, the copy button, the text in the code family.
- **Tokens:** `diff_added` = `card` mixed with the theme's green, `diff_removed` = `card` mixed with its red, each mixed as far as keeps `foreground` at 4.5:1 on it (`Color::legible_on` gives the bound), in `src/theme.rs` `ui_tokens`; the neutral token sets take fixed tints. The legibility test gains `foreground` on each.
- **The action** `toggle_agent_tool_details` ("Toggle Tool Details", no default key) is handled only by `AgentView`, so the palette lists it only in an agent zone.
- The detail adds no keyboard stop: "Keyboard path through an agent zone" is unchanged. A keyboard user opens details with the action, and a pending permission shows its detail on its own.

### 12. The dump (`testing R2`)

A tool's entry line gains `open=yes|no|none` (`none`: nothing to open) and `items=N` before its title. After it, whether open or closed:

```
--- entry 4 tool call="c1" kind=edit status=completed open=no items=2 "Edit main.rs"
--- item entry=4 index=0 kind=diff op=modify path="/p/main.rs" added=1 removed=1 omitted=0
--- hunk entry=4 index=0 old=10 new=10 " fn main() {\n-    old();\n+    new();\n }"
--- item entry=4 index=1 kind=output command="cargo test" exit=0 omitted=0 "ok\n"
--- location entry=4 path="/p/main.rs" line=10
--- raw entry=4 which=input "{…}"
```

- `kind` is `text` (with `cut=yes|no` and the source), `diff` or `output`; `op` is `add`, `delete`, `modify`, `move`, `copy` or `other:NAME`, with `from="…"` for a move or copy; `exit` is a number, `signal:NAME` or `none`; texts are escaped with `{:?}` as entry lines are. `--- raw` lines print only when the detail shows them.
- A permission entry line gains `detail=N`, the number of the named call's parts shown with it (0 when none).
- `wait_text` holds the same lines, so `--wait-for 'kind=diff'` works.

### 13. Failures (`railway R6`, `R8`)

Each is shown in the conversation of the session it belongs to, and none ends the session:
- a content item that can't be read: left out, one notice per tool call;
- content past a bound: kept up to the bound, with the count in the detail;
- a fold error on a new event: the zone's existing "can no longer show the session" state;
- a Markdown render panic in a text item: the source as plain text, as for messages.

### 14. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Tool content is in the transcript, so it is checkpointed with it at the existing moments. After a core restart, the holder's kept stdout since the last checkpoint goes through the reducer again; the diff and the output filter are deterministic, so the log comes out the same. A Brindle crash loses only the open state of rows; the content replays on attach. An upgrade's next core reads the same format 1. Agent output is still handled under the reducer's `catch_unwind`, per session (`process R3`).

### 15. Tests

- **Unit:** the fold over every new event, error and bound; `fold(replay(t)) == t` with tool content; the fuzz corpus gains the new forms; `diff::hunks` (decision 4); `only_the_diff_module_names_similar` and the closed list; `plain::feed` over split sequences, carriage returns and every escape form; `v1::decode` over each captured line and awkward ones (an item of the wrong shape, a diff with no `newText`, a 9 MiB text); the reducer's upsert, the permission's tool call, equal content emitting nothing; the model's parts, summaries, toggles, copy text and dump lines.
- **Captures:** the replay test over `tests/fixtures/acp/` asserts what each agent's diff becomes, and gains the new captures of task 1.1.
- **Real roles** (`remote-agent/tests/agents.rs`, the fake ACP agent): a diff and a streamed output reach an attached client; both survive a core restart mid-output and an upgrade.
- **Architecture:** `acp_versions_stay_in_their_modules`; the pure modules.
- **E2E** against the fake agent: `agent-tool-diff`, `agent-tool-output`, `agent-tool-raw`, `agent-tool-bounds`, `agent-tool-toggle`, `agent-permission-detail`, `agent-tool-restore`.
- **Manual:** pointer presses on a row's chevron and the copy buttons; a real agent's edit and command; the diff colours in a light and a dark theme.

## Open decisions

Each has a recommendation, which the text above already follows. They are the user's to settle before implementation.

Decided on 2026-11-25: `Dialect` is a closed enum (decision 2); the `_meta` terminal convention is read for v1 when a capture shows it is an agent's only way to deliver output, and the `terminal` client capability stays out, as `agents-client-terminals` (decision 6); the diff comes from the `similar` crate, which the user chose over the recommended hand-written one (decision 4).

Still open:

3. **Tool rows start closed, and a pending permission always shows its detail** (recommended). The alternative is rows that start open while a turn runs.
4. **Raw input and result only when a call has no content** (recommended), or always under a second trigger.
5. **The bounds** of decision 5, 64 KiB an item and 256 KiB a call: a single-file diff past about 1,000 changed lines shows its first 64 KiB and a count.

## Risks / Trade-offs

- A long diff is cut at 64 KiB → the row says how many lines aren't shown, and the agent's file is on disk to read.
- The line diff isn't always minimal (`similar` forces a split when a search gets expensive) → still correct, and bounded work on the core's thread is the point.
- A new dependency in the helper that is copied to remote hosts → one package with no dependencies of its own, named in one file, with the lock diff and the musl build checked in the tasks.
- Command output loses its colours → plain and legible; drawing it through the emulator is a later change.
- The Claude adapter resends content with every update → each resend costs a diff on the core. The prefix and suffix skip makes that proportional to the file's size, and equal results emit nothing.
- Amending protocol 4 assumes no release ships it first → decision 8 says what changes if one does.
- Moving the v1 code touches files the stack's earlier changes also edit → the move is one commit made at the stack's tip, with no behaviour change, checked by the unchanged tests.

## Migration Plan

Nothing to migrate: no config key, state file or released protocol changes. A session checkpointed before the change keeps its tool rows as one line; content appears for tool calls made after it.
