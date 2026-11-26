# Design

## Context

See proposal.md for why. The first design was written on 2026-11-25 from `feat/agents-profiles` and the designs stacked on it. It was refreshed on 2026-11-26 against the code as built, using two sources:
- `main` at 6c8ecd2, where these changes are archived: `agents-sessions`, `agents-profiles`, `agents-markdown`, `agents-session-recovery`, `agents-login`, `remote-login-environment`, `agents-markdown-layout-fixes`, `agents-zone-theme-scope`, `agents-session-title` and `agents-zone-attention`.
- `feat/agents-log-paging` at 20e9858, where `agents-log-paging` is implemented and archived (PR #144, CI pending). This change is branched from it.

What follows is the code this design builds on, as built.

- **The transcript** (`brindle-transcript`, `transcript/src/lib.rs`):
  - **The entry.** `Entry::Tool { call, title, kind, status }`, documented "Its content isn't kept". `Entry::Permission { call: Option<String>, title, options, outcome }` names the tool call it asks about.
  - **The events.** `Event` has `Added`, `Appended`, `ToolChanged { id, title?, kind?, status? }`, `Answered`, `Queue`, `Turn`, `Agent`, `Conversation`, `Trimmed`, and paging's `Window { from, earlier }`. They are JSON tagged by `event` in snake case.
  - **The log** is a `VecDeque<Kept { id, entry, size }>`:
    - `size` is `Entry::encoded_len`.
    - `Appended` grows it by `escaped_len` (no re-encode).
    - `ToolChanged` and `Answered` measure the entry again (`resize_entry`).
    - The sum is `encoded_len()`, which the log's bound counts.
  - **The replay.** `replay_entry` replays a tool entry as one `Added`, and text as `Added` plus `Appended`s of at most `TEXT_CHUNK` (64 KiB).
  - **Paging** (`agents-log-paging`):
    - `page(budget)` takes the newest whole entries up to the budget (always the newest), and lowers the start to the oldest permission with no outcome.
    - `fetch(before, budget) -> Fetched { events, more }` returns older entries in their current state.
    - `prepend(events, more)` folds a fetched page in front.
    - `PAGE_BYTES` is 256 KiB.
    - A paged copy ignores the events `entry_change` names (`Appended`, `ToolChanged`, `Answered`) for an entry below its window.
    - `FoldError` gained `WindowAfterEntries`, `WindowBehind`, `WindowEmpty`, `PageGap`, `WindowMismatch` and `NoWindow`.
  - **Tests.** `fuzz.rs` has `random_transcript` and the paging laws as property tests. `transcript/tests/fixtures/v4/` holds one file per form (`added-tool.json`, `tool-changed-*.json`, `window.json`, …), written when missing and never edited.
- **The ACP code** (`remote-agent/src/acp/`, whole pure modules in `PURE_MODULES`). The modules are `wire.rs`, `rpc.rs`, `lines.rs`, `reduce.rs` and `checkpoint.rs`, listed in `mod.rs`.
  - **`wire.rs`** (2,090 lines) is the only file naming `agent_client_protocol_schema::v1` (`use … v1 as acp`). It holds these items:
    - the envelope (`Id`, `Message`, `Malformed`, `decode`, `RpcError`);
    - `RequestKind` and `NotificationKind`, by method name;
    - `initialize_result` and `auth_methods`, the login methods read by hand;
    - `supported_versions`, `new_session_result`, `resume_result`, `load_result` and `prompt_result`;
    - `session_update`, dispatched by hand on `sessionUpdate`;
    - the `Patch<T> { Keep, Clear, Set }` that `agents-session-title` added for `session_info_update`'s title;
    - `permission_request`, decoded by hand with `call: Option<ToolChange>`, used today only for the request's title;
    - `cancel_request`.
  - **How `wire.rs` decodes tool calls.** `tool_call` and `tool_change` decode the update whole through the crate's `ToolCall` and `ToolCallUpdate`, so one malformed content item loses the whole update. They keep only call id, title, kind and status.
  - **`rpc.rs`** holds `Rpc` (ids, outgoing purposes, held permissions) and every line written:
    - the methods `initialize`, `authenticate`, `new_session`, `resume`, `load` and `prompt`;
    - the free functions `cancel`, `cancel_request`, `permission_answer` and `error`.
    - `PROTOCOL_VERSION = 1`.
    - Since `agents-login`, `initialize` offers `clientCapabilities: {"auth":{"terminal":true}}`, and still no `fs` or `terminal`. An agent's `terminal/*` request is `RequestKind::Unoffered`, answered `-32601`.
  - **`reduce.rs`**:
    - `Step::emit` is the only way the log changes.
    - `Step::tool_call`, `tool_change`, `add_tool` and `change_tool` keep `AgentSession.tools`, a map from call id to entry.
    - `Step::permission` adds the permission entry and doesn't touch the tool entry.
    - `Step::trim` trims past `LOG_LIMIT` (16 MiB) to `LOG_TRIM_TO` (12 MiB). It matches every `Event` variant exhaustively.
    - `clean` replaces control characters other than `\n` and `\t`, and `bound` cuts at `TEXT_CHUNK`.
    - Version names in the reducer:
      - `initialized` compares `protocol_version == PROTOCOL_VERSION` and writes the "the agent speaks ACP version N" failures;
      - `continued` reads `method == "session/load"`;
      - a muted load counts tool updates rather than applying them.
- **The checkpoint** is format 2, from `agents-log-paging` (`acp/checkpoint.rs`). It is `BACP`, the format, then records `B` (base) and `S` (segment), each with a sequence number, a JSON state and a partial line.
  - The base's state holds:
    - the log as `replay()` events;
    - `acp`, the version, always 1 today;
    - `cwd`, `version`, `phase`, `methods`, `rpc`, `run`, `tools`, `bad_lines` and `lines`.
  - A segment carries `events` in place of `log`, with consecutive `Appended`s of one entry merged up to `TEXT_CHUNK`.
  - Format 1 is still read.
  - The core decides between appending and writing whole with `policy::{agent_checkpoint_form, agent_appendable}` over `Stored` and the unsaved events.
  - `MAX_CHECKPOINT_LEN` is 64 MiB.
- **The wire.** Transcript events travel in `TRANSCRIPT` (42) frames, packed whole per frame under the 1 MiB frame limit (`pack_transcript`), and fetched pages in `AGENT_PAGE` (43, `pack_page`). Paging fixed `AGENT_FETCH` at 23 and `CHECKPOINT_APPEND` at H1 16, and `agents-commands` plans `AGENT_SET_OPTION` at 24. This change adds no message.
- **Versions.** v0.3.0, the latest release, speaks client protocols 1–3 and H1 1–2. Client protocol 4, H1 3 and checkpoint format 2 are unreleased, and the agent changes ship together.
- **Brindle's zone** (`src/agent/`):
  - **`transcript.rs`** re-exports the transcript types the app uses. It is the only app file naming `brindle_transcript`.
  - **`shared.rs`** has `AgentShared`, whose fold records `Changes { added_from, changed: BTreeMap<EntryId, Dirty>, trimmed_through, slot_changed, turn_or_agent, reset, from_replay, prepended }` with `Dirty::{Appended { from_byte }, Replaced}`.
  - **`sink.rs`** buffers replays and pages.
  - **`model.rs`** (`ChatModel`, GPUI-free):
    - It keeps `entries`, the Markdown cache `rendered`, and `blocks`.
    - A `Block` is `{ entry, part, start, last, body }`, with these bodies:
      - `BlockBody::Tool { kind, title, status }`;
      - `BlockBody::Permission { title, options, outcome }`, whose buttons are reject kinds first;
      - `BlockBody::Thought { blocks, open }`, with `toggled` and `auto_open` and `toggle_thought` and `toggle_thoughts`, which return `Splice`s.
    - `blocks_of` and `render_entry` build the blocks; the renderer runs under `catch_unwind`.
    - `working_row` and `tool_in_progress` read the held entries.
    - Paging's `Earlier` and `fetch_due` live here.
    - The dump is `describe` and `wait_text` through `entry_line`: `tool call="…" kind=… status=… "TITLE"`, and `permission[ call="…"] options=ID:"NAME"|… outcome=… "TITLE"`.
  - **`view.rs`** has `tool_icon`, `tool_status` and `permission_variant`. It draws the pending permission row as the destructive alert of `agents-zone-attention`, and the list through `MessageScroller`.
  - **`mod.rs`** has the `AgentView` action handlers.
- **`brindle-ui`**:
  - **`Tool`** (`ui/src/components/tool.rs`): an icon, a title cut by `trigger_label`, a status badge. It has no content and takes no focus.
  - **`CodeBlock`**: the text on `muted`, a label, and a pointer-only copy button.
  - **`Response`** and **`Reasoning`**. `Reasoning`'s trigger is the pattern for a row that opens.
  - **`MessageScroller`**: a GPUI `list`, bottom-aligned, with `Follow::{AtEnd, Away}`, `scroll_page`, and the Earlier row in its top padding.
  - **`Loader`** and **`Shimmer`**, on `src/motion.rs`'s clock.
  - **Tokens.** Components read `cx.ui_theme()`, which inside an agent zone is the zone token set's scope (`agents-zone-theme-scope`).
- **Theme** (`src/theme.rs`):
  - **`tokens_over(bg)`** derives both sets (`ui_tokens` over the chrome background, `zone_tokens` over the theme's own). `card` is `bg` mixed 4% towards the foreground and `muted` 8%. `foreground` is pushed to 4.5:1 on `muted`.
  - **`destructive_tint(card, text, destructive)`** mixes the card towards red in 1% steps, up to `DESTRUCTIVE_TINT_MAX` (12%), as far as keeps the text at 4.5:1 and the red at 3:1.
  - **Tests.** `ui_tokens_are_legible_for_every_builtin` checks the pairs over both sets.
  - **Export.** `--export-ui-tokens` exports shadcn's list only, and `destructive_tint` isn't in it.
- **Config and actions.**
  - `src/config.rs` has no `[agent]` table. `[notifications]` has `agent: bool`.
  - `src/actions.rs`'s `ACTIONS` lists the agent actions (`toggle_agent_thoughts`, `agent_scroll_to_bottom`, …), which the palette lists only in an agent zone (`palette_lists`, `PaletteZone::Agent`).
- **Architecture.**
  - `tests/architecture.rs` has `ALLOWED`, the closed lists; for `remote-agent` these are the three shared crates, `agent-client-protocol-schema`, `embed-resource`, `libc` and `windows`.
  - It also has `only_the_acp_modules_name_the_schema_crate`, `PURE_MODULES` with the five `acp/` files, and `the_pure_modules_read_nothing_ambient`.
  - `docs/architecture/ports-and-adapters.md` has rules R1–R13.
- **The fake ACP agent** (`remote-agent/src/fake_acp/`) `send`s any JSON line and gates on files. Tool content needs no new script step.
- **What real ACP v1 agents send.** These are the logs in `remote-agent/tests/fixtures/acp/`, with the agent's capability offer as it was when each was captured, `clientCapabilities: {}`:

  | Agent | Tool content | Permission's `toolCall` | Command output | Other |
  | --- | --- | --- | --- | --- |
  | claude-agent-acp 0.112 (`-reject`, `-haiku-reject`) | `diff` (`path`, `oldText: null`, `newText`) on the `tool_call`; a text block in a ```` ``` ```` fence for the result | `content` with the diff, `locations`, `rawInput` | none captured | `rawOutput` repeats the text |
  | claude-agent-acp 0.112 (`-auto`, an allowed write of a new file) | `content: []` on the `tool_call`, then a `tool_call_update` with the `diff` (`oldText: null`); `locations` | none (allowed without asking) | none | `rawOutput` is "File created successfully at: …" |
  | claude-code-acp 0.16.2 | a `diff` and a text block | as Claude's | none | |
  | antigravity 1.5.16 | none on the `tool_call` | `content` with the diff (`oldText` absent) | none captured | `rawInput`, `locations` |
  | codex-acp 2.12 | `terminal` (`terminalId`) with `_meta.terminal_info { cwd, terminal_id }` on the `tool_call` | `rawInput` (`command`, `cwd`), no content | **yes**: the allowed `pwd` ends with `_meta.terminal_output_delta { data: "/home/user/scratch\n", terminal_id }` and `terminal_exit { exit_code: 0, signal: null, terminal_id }` on its completing `tool_call_update`; the rejected one with `terminal_exit { exit_code: null, signal: null }` | the title is the command |
  | copilot 1.0.115 | none | `rawInput` (a patch as text), `locations` | none captured | `rawOutput` |

  No capture holds an edit of an existing file, or command output from the Claude adapter, Antigravity or Copilot. Task 1.1 captures them.
- **ACP v2 differs.** These facts come from schema crate 1.22.0's `src/v2/tool_call.rs` and `terminal.rs`; v2 is in Preview.
  - A tool call is an upsert with patch semantics: omitted keeps, `null` clears, a value replaces, and collections replace whole.
  - `tool_call_content_chunk` appends one content item.
  - A `diff` is `changes` (add, delete, modify, move, copy, each with absolute paths), plus an optional `patch` whose only defined format is Git's `--patch` text.
  - `terminal` content names an agent-owned terminal. Its `command`, `cwd`, `output` and `exitStatus` arrive in `terminal_update`, and `terminal_output_chunk` appends its output.
  - Statuses add `paused` and `cancelled`.
  - Almost every enum accepts unknown values.

## Goals / Non-Goals

**Goals:**
- A user reads an edit before allowing it, and can see afterwards what a tool call changed, ran and printed.
- A tool call's content is Brindle's own data: Brindle and later clients never learn which ACP version produced it, and a version's differences are written once, in that version's module.
- Everything an agent sends is bounded in size and in the work it costs the core, and one unreadable item loses only itself (`railway R5`).
- A long diff or output costs the view only what is on screen, and a page only what it holds.
- Every decision is a pure function or model with unit tests; the dump and e2e cases pin the rest.

**Non-Goals:**
- Syntax highlighting, and word-level or side-by-side diffs.
- Command output through the terminal emulator (colours, cursor movement, a size).
- Opening a location or a diff's file, and links in a detail other than those Markdown text gives.
- Applying, reverting or editing a diff.
- Images, audio and embedded resources in tool content: they show as a placeholder, as in messages.
- Text selection and find in a detail.
- Tool details as keyboard focus stops.
- `agents-acp-v2`'s parts: a tool call's v2-only statuses, v2's session references (subagents), cancel controls and terminal sizes.
- ACP's `fs` and `terminal` client capabilities.
- A minimum time a row stays open.
- Per-profile tool detail settings, and editing them in the settings dialog.
- Any new client or holder message, or new state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `ToolItem`, `FileChange`, `FileOp`, `Hunk`, `DiffLine`, `Output`, `Exit`, `Location`; `Event::ToolItems`, `Event::ToolOutput`; the fold, its bounds, `entry_change`, `replay_entry`; `page` reaching a waiting permission's tool entry | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| The JSON-RPC envelope (`Id`, `Message`, `Malformed`, `decode`, `RpcError`) | pure | `brindle-remote-agent`, `acp/envelope.rs` (from `wire.rs`) |
| The forms the reducer reads and the requests it asks for (`Initialized`, `Methods`, `NewSession`, `SessionUpdate`, `Update`, `Patch`, `ToolChange`, `ToolItemForm`, `OutputForm`, `PermissionRequest`, `Request`, …) | pure, plain data | `acp/forms.rs` (from `wire.rs` and `rpc.rs`) |
| `Dialect`, the one `match` on the ACP version | pure | `acp/dialect.rs` (new) |
| ACP v1: decoding its messages into the forms, encoding the requests | pure | `acp/v1/` (`decode.rs`, `encode.rs`), the only files naming `agent_client_protocol_schema::v1` |
| Line diff into hunks | pure | `acp/diff.rs` (new), the only file naming `similar` |
| Command output as plain text (escape sequences, carriage returns) | pure | `acp/plain.rs` (new) |
| Tool handling: upsert, bounds, open outputs, the permission's tool call | pure | `acp/reduce.rs` |
| The checkpoint's open outputs and merged `ToolOutput`s | pure | `acp/checkpoint.rs` |
| The tool detail's parts, open state, summaries, copy text, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| `Dirty` for replaced items and appended output | fold | `src/agent/shared.rs` |
| Drawing details, the copy buttons, the toggle action, the policy from the settings | shell (entity) | `src/agent/view.rs`, `src/agent/mod.rs`, `src/actions.rs` |
| `Tool` (trigger, summary), `Diff` | components | `brindle-ui`, `ui/src/components/tool.rs`, `diff.rs` (new) |
| `diff_added`, `diff_removed` | tokens | `ui/src/theme.rs` (`Tokens`, the neutral sets); values in `src/theme.rs` `tokens_over` |
| `[agent]`'s `tool_details` and `tool_details_when_done`, decoded into `Shown` | pure (boundary parse) | `brindle`, `src/config.rs` |

Crate references don't change (`ports R1`). One closed list does (`ports R3`): `brindle-remote-agent` gains `similar` (decision 4). `acp/{envelope, forms, dialect, diff, plain}.rs` and `acp/v1/*.rs` are whole pure modules and join `PURE_MODULES` and `fcis`'s table, and `acp/wire.rs` leaves them (`fcis R2`, `R3`, `R7`).

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
    pub partial: bool,                      // the kept text starts mid-line (flag 19)
    pub exit: Option<Exit>,
}
pub enum Exit { Code { code: i32 }, Signal { name: String } }
pub struct Location { pub path: String, pub line: Option<u32> }
```

`Entry::Tool` gains `content: Vec<ToolItem>`, `more: u32` (items dropped past the item bound), `locations: Vec<Location>`, `input: Option<String>` and `result: Option<String>`.
- Each is `#[serde(default, skip_serializing_if = …)]`, so `added-tool.json` and every other existing fixture decode and encode byte for byte.
- Its doc comment "Its content isn't kept" goes.

- **What the types rule out.**
  - A change is one operation, and only a move or copy has a second path.
  - A hunk line is context, added or removed, never two.
  - An exit is a code or a signal.
  - "No command", "no exit yet" and "no raw input" are `None`, not empty strings.
  - `FileOp::Other { name }` keeps an operation a later ACP adds, as `StopReason::Other` does.
- **Encoding.** `DiffLine` is one JSON string per line whose first character is a space, `+` or `-`. Any other first character, or an empty string, is a `DecodeError`. That is a third of the size of an object per line, and it is parsed at the boundary (`types R5`).
- **A change with no hunks** is valid: a binary file, a v2 change with no patch text, a move with no edit. The row then shows the operation and the path alone.
- **Why hunks, not the old and new text.**
  - ACP v1 gives a diff as the whole old and new file. Keeping those would put two copies of every edited file in a 16 MiB log, and make every client compute the diff.
  - ACP v2 gives changes and patch text, with no file texts at all.
  - Hunks are the form both reduce to, so the core computes them for v1 (decision 4) and reads them from the patch for v2 (decision 9).
  - *Rejected:* keeping v1's texts and v2's patch as two item kinds, which would put the version difference in every client.
- **Forms v1 can't fill are added now.** No v1 agent can report a delete, move or copy. They are in the forms anyway, with unit tests and fixtures, because protocol 4 can still be amended (decision 8), and adding them with `agents-acp-v2` after a release would cost a protocol version. Spec scenarios use only what a v1 agent can send.
- **Text is Markdown source**, as ACP says of text content blocks and as the Claude adapter's fenced results show. A content block that isn't text becomes the placeholder text the reducer already uses for it in messages (`reduce::placeholder`).

### 2. One implementation per ACP version (`ports R4`–`R8`, `types R2`)

The user's rule: v1 and v2 are both supported for good, with no `if v1` / `if v2` spread through the code. `remote-agent/src/acp/` becomes:

```
acp/
  envelope.rs    JSON-RPC: Id, Message, Malformed, decode, RpcError. No ACP version in it.
  forms.rs       What the reducer reads (Initialized, Methods, NewSession, SessionUpdate, Update, Patch,
                 ToolChange, PermissionRequest, …) and what it asks to be written
                 (Request::{Initialize, Authenticate, NewSession, Resume, Load, Prompt, Cancel,
                 CancelRequest, PermissionAnswer, Error}).
  dialect.rs     pub enum Dialect { V1 }  and the only `match` on it.
  v1/decode.rs   ACP v1 → forms (and request_kind, notification_kind by method name).
  v1/encode.rs   Request → ACP v1 lines.
  diff.rs  plain.rs  lines.rs  rpc.rs  reduce.rs  checkpoint.rs
```

- **`Dialect` is a closed enum, not a trait.** The user confirmed this on 2026-11-25. `ports R6` allows no indirection to make a decision mockable, and `types R2` asks for closed enums.
  - Its methods are `session_update`, `permission_request`, `initialize_result`, `auth_methods`, `supported_versions`, `new_session_result`, `continued_result`, `prompt_result`, `cancel_request`, `request_kind`, `notification_kind` and `encode`.
  - Each holds one exhaustive `match self` that calls the same-named item of that version's module, as a per-platform module set exports the same names (`ports R5`).
  - Adding `V2` then fails to compile until every method has its arm.
- **Chosen once.**
  - `Dialect::newest()` writes `initialize`.
  - The answer's `protocolVersion` is read from the raw result (it is a number in every version), and `Dialect::from_version(n) -> Result<Dialect, Unsupported>` picks the session's dialect.
  - The session keeps it. The checkpoint already records it as `acp`, and a decode restores it from there.
  - `Unsupported` is today's "the agent speaks ACP version N; Brindle speaks 1" failure, and `supported_versions`' texts are unchanged.
  - Until `agents-acp-v2`, `newest()` is `V1` and `from_version` accepts only 1.
- **`reduce.rs` and `rpc.rs` name no version.**
  - `rpc.rs` keeps ids, outgoing purposes and held permissions. The line for a request comes from `dialect.encode(Request)`.
  - The reducer holds a `Dialect` value and calls its methods; it never matches on it.
  - `continued`'s `method == "session/load"` becomes `dialect.continued_result(purpose, result)`, so the method name moves into `v1/`.
- **`Patch<T> { Keep, Clear, Set(T) }`** already exists in `wire.rs` for `session_info_update`'s title. It moves to `forms.rs` and now also carries each field's change in `ToolChange`. v1 produces `Keep` and `Set` for tool fields, and `Clear` only where it already does for a title; v2's `null` is `Clear`. The reducer handles all three once.
- **The rule and its check.**
  - `docs/architecture/ports-and-adapters.md` gains **R14**:
    - An ACP protocol version has one module under `acp/`.
    - That module alone names its `agent_client_protocol_schema` version module, its method names and its field names.
    - `Dialect`'s variants are matched only in `acp/dialect.rs`.
    - Everything else in the crate reads `acp/forms.rs`.
  - `ports R6`'s text gains "a protocol version an outside program chooses is a seam of the same kind as a platform". The doc's README gets the rule's row.
  - `tests/architecture.rs` gains `acp_versions_stay_in_their_modules`:
    - `agent_client_protocol_schema::v1` only under `acp/v1/` (and `::v2` only under `acp/v2/`);
    - `Dialect::V` only in `acp/dialect.rs`;
    - neither `v1::` nor `v2::` in `reduce.rs`, `rpc.rs` or `checkpoint.rs`.
  - `only_the_acp_modules_name_the_schema_crate` stays.
- **The move changes no behaviour.**
  - It is its own commit, before any tool content.
  - Every existing unit test, the capture replays, `remote-agent/tests/agents.rs` and the agent cases of `upgrade.rs`, `manage.rs` and `relay.rs` pass unchanged apart from paths.
  - It moves all of today's v1 code: `agents-login`'s `auth_methods`, `authenticate` and terminal capability; `agents-session-recovery`'s `resume` and `load`; `agents-session-title`'s `session_info_update`.
- *Rejected:*
  - a `trait Dialect` with `V1` and `V2` structs behind `dyn`: no second implementation is swapped in for tests, and a closed enum makes a missing arm a compile error;
  - version checks at each use, which the rule exists to stop;
  - a separate OpenSpec change for the move: it changes no requirement, so it would have no spec delta, and this change is the first that needs it.

### 3. The v1 mapping (`acp/v1/decode.rs`; `railway R5`)

| ACP v1 | Form |
| --- | --- |
| `content` on `tool_call` and `tool_call_update` (replaces the list) | `ToolChange.content = Set(items)`; absent is `Keep`; on a `tool_call`, absent is `Set([])` (a new call has none) |
| `{ "type": "content", "content": { "type": "text", … } }` | `ToolItemForm::Text` |
| `{ "type": "content", … }` with another block | `ToolItemForm::Text` holding the placeholder |
| `{ "type": "diff", "path", "oldText", "newText" }` | `ToolItemForm::RawDiff { path, old: Option<String>, new }`, which the reducer turns into a `FileChange` (decision 4): `Add` when `oldText` is absent or `null`, else `Modify` |
| `{ "type": "terminal", "terminalId" }` | `ToolItemForm::Terminal { terminal }` (decision 6) |
| any other `type` | `ToolItemForm::Text` holding `[content Brindle doesn't show: TYPE]` |
| `locations` | `ToolChange.locations` |
| `rawInput`, `rawOutput` | `ToolChange.input`, `.result`: a JSON string as its text, anything else as indented JSON |
| `_meta.terminal_info`, `_meta.terminal_output_delta`, `_meta.terminal_exit` | `ToolChange.outputs: Vec<OutputForm>` (decision 6) |
| a permission request's `toolCall` | the same `ToolChange` (decision 7) |

`tool_call` and `tool_change` stop decoding the update whole through the crate.
- The crate decodes the update with `content`, `locations`, `rawInput`, `rawOutput` and `_meta` taken out of the value. Those are read by hand, as login methods and permission options are.
- Content items are decoded one at a time (each through the crate's `ToolCallContent`).
- An item that doesn't decode is left out and counted, and the reducer adds one notice per tool call: "N content items of tool call TITLE couldn't be read". Today, the crate's enum loses the whole update.

### 4. The line diff (`acp/diff.rs`, pure; `fcis R1`–`R3`, `process R8`)

`pub fn hunks(old: &str, new: &str) -> Diffed` where `Diffed { hunks: Vec<Hunk>, omitted: u32 }`.

- **The `similar` crate does the comparing.** The user chose it on 2026-11-25 over a hand-written Myers.
  - Facts, re-read from `~/.cargo/registry/src/…/similar-3.2.1/Cargo.toml` on 2026-11-26: `similar` 3.2.1, Apache-2.0, `rust-version` 1.85 (the pin is 1.99.1).
  - Its features are `default = [std, text]`, with `std = ["bstr?/std"]` and `text = []`. Every dependency (`bstr`, `hashbrown`, `serde`, `unicode-segmentation`, `web-time`) is optional.
  - Without `std` the crate is `no_std` with `alloc`.
  - It is not in `Cargo.lock` yet.
  - Its default `Algorithm::Myers` trims the common prefix and suffix, then follows Git's xdiff policy: it accepts a long snake once the search is expensive, and forces a split at a size-dependent work limit. So its work is bounded without a deadline, and its result is a correct, sometimes non-minimal, script.
- **The dependency.**
  - It is `similar = { version = "3.2.1", default-features = false, features = ["text"] }` on `brindle-remote-agent`, added with `cargo add similar@3.2.1 --no-default-features -F text -p brindle-remote-agent`.
  - Without `std` the crate's deadline type is `()`, so it can't read a clock and `diff.rs` stays pure (`fcis R2`).
  - If `text` turns out not to build without `std`, the default features are used instead, and `diff.rs` never calls `deadline` or `timeout`, which the architecture check below pins.
  - `CLAUDE.md`'s lock rule holds: `git diff Cargo.lock` shows exactly one added `[[package]]` and the one added name in `brindle-remote-agent`'s dependencies. If anything else moved, revert and pin; never regenerate.
- **The closed list and one door** (`ports R3`).
  - `similar` joins `brindle-remote-agent`'s `ALLOWED` list in `tests/architecture.rs`, `ports R3`'s text and `.claude/rules/remote-agent.md`.
  - Only `acp/diff.rs` names it (`only_the_diff_module_names_similar`, as only the ACP modules name the schema crate), so swapping the library touches one file.
  - The static musl helper must still build, and its size is noted in the PR.
- **No clock, so the same result twice.** No deadline or timeout is ever set: a time limit would make a restarted core compute different hunks from the same bytes (decision 14).
- **What `diff.rs` adds around the crate.**
  - `similar::algorithms::myers::diff` over the two texts' lines (each with its `\n`), through a `Replace` hook into a `Capture`, and `group_diff_ops(ops, 3)` give the groups (flag 24).
  - `diff.rs` turns each group into a `Hunk`: 1-based starts, each line without its line break. A trailing `\r` stays part of the line and is cleaned later with the rest.
  - It keeps lines until their text passes `ITEM_LIMIT` (decision 5) and counts the rest in `omitted`.
  - As a guard on memory, texts with more than `MAX_DIFF_LINES` = 200,000 lines together aren't compared. The change becomes one hunk removing the old lines and adding the new, cut the same way.
- **Tests:**
  - `apply(old, hunks) == new` for a table of edits, and for 10,000 deterministic pseudo-random pairs (the crate's own `XorShift`-style generator, no new dev-dependency);
  - an added file (`old` absent) is one hunk of added lines;
  - line numbers are 1-based and match the texts;
  - a text with and without a final line break;
  - 50,000 lines against 50,000 unrelated lines finishes and is cut at `ITEM_LIMIT`;
  - the `MAX_DIFF_LINES` guard;
  - the same inputs give the same hunks twice.
- *Rejected:* a hand-written Myers, which needs no dependency but is our own code to get right where a maintained library already carries xdiff's safeguards; and `similar`'s deadline, which puts a clock in the reducer and gives results that differ between runs.

### 5. Bounds (`railway R5`, `process R9`)

| Bound | Value | Past it |
| --- | --- | --- |
| `ITEM_LIMIT`: one item's text (a `Text`'s text, a `FileChange`'s lines, an `Output`'s text) | 64 KiB | `Text`: the end is dropped, `cut` set. `FileChange`: later lines dropped, counted in `omitted`. `Output`: whole lines dropped from the start, bytes counted in `omitted` |
| `MAX_ITEMS`: items in a tool call | 32 | later items dropped, counted in `more` |
| `CALL_LIMIT`: a tool call's items' text together | 256 KiB | later items dropped, counted in `more` |
| `RAW_LIMIT`: each of `input` and `result` | 16 KiB | the end is dropped, with `…` |
| `MAX_LOCATIONS` | 32 | later ones dropped |
| `MAX_OPEN_OUTPUTS`: outputs a session still appends to | 64 | the oldest is closed (decision 6) |
| a path, a command, a `cwd` | as titles are (`bound`) | cut as titles are |

- `ITEM_LIMIT` equals `TEXT_CHUNK`, so any one item fits one event and one frame. With JSON escaping, an item's event is at most a few hundred KiB, under the 1 MiB frame.
- The reducer applies every bound before an event is made, except an output's tail, which the fold applies (decision 8). So the core's log and a client's copy can't differ.
- A tool entry is at most about 300 KiB of text, and its encoded size can be more than `PAGE_BYTES`. As for any entry larger than the page, a page then holds it alone (decision 15). `LOG_LIMIT` and `LOG_TRIM_TO` still hold dozens of the largest. The log's trim is unchanged: oldest entries whole.
- Every text is cleaned as today; paths, commands and raw values included.

### 6. Command output (`acp/plain.rs`; the `_meta` terminal convention)

`Output.text` is plain text.
- `plain::feed(state, bytes) -> String` removes escape sequences: CSI, OSC, DCS and the other string forms, and single-character escapes.
- It keeps `\n` and `\t`.
- It resolves a carriage return by keeping only what follows the last one on a line, so a progress bar leaves its final state.
- Its state (inside a sequence, the current line) is part of the session and of the checkpoint, so a sequence split across two updates is still removed.
- An unterminated string form holds back at most a bounded amount.

**How a v1 agent delivers output, as captured.** The first design left this to task 1.1. An existing capture answers it for Codex, which sends output to a client that offered no terminal capability (`codex-acp-2.12.0-reject.log`, captured with `clientCapabilities: {}`):
- **`terminal_info`.** The `tool_call` carries content `[{type: "terminal", terminalId}]` and `_meta.terminal_info { cwd, terminal_id }`.
- **`terminal_output_delta`.** The completing `tool_call_update` carries `_meta.terminal_output_delta { data, terminal_id }`.
- **`terminal_exit`.** The same update carries `_meta.terminal_exit { exit_code, signal, terminal_id }`. A rejected command has both `null`.

By the rule the user set on 2026-11-25 (read the convention when a capture shows an agent's output reaches Brindle no other way), the v1 module reads these three keys.
- The real names are `terminal_output_delta` and `terminal_exit`. The first design guessed `terminal_output`.
- They are outside the ACP spec, so they stay inside `acp/v1/`, are pinned by the capture, and are the only `_meta` keys read.
- `OutputForm::{Opened { terminal, cwd, command }, Appended { terminal, bytes }, Exited { terminal, exit }}` carries them to the reducer:
  - `command` is `terminal_info.command` when present. Codex sends none, and its title is the command.
  - `exit` is a code, a signal by name, or none when both are `null`.
- Output that arrives as a text content block (expected of the Claude adapter) stays a `ToolItem::Text`, with nothing more to do.
- A `terminal` item whose terminal never gets an `Opened` or output adds nothing.

**Open outputs** (`acp/reduce.rs`, version-free). The reducer keeps `outputs: BTreeMap<String, OpenOutput { entry, item, filter: plain::State }>`, keyed by the agent's terminal id.
- A `Terminal` item or an `Opened` form opens one: an `Output` item in that tool call's content, at the end, or in the place of the `Terminal` item.
- `Appended` goes through `plain::feed` and becomes `Event::ToolOutput`. `Exited` sets the exit and closes it.
- A replaced content list that still names a terminal keeps that output item as it is, so a resend of `[terminal]` doesn't wipe the text.
- Past `MAX_OPEN_OUTPUTS` the oldest is closed, its text kept.
- A trim drops the outputs of trimmed entries, as it drops their `tools`.
- v2's agent-owned terminals use the same map (decision 9), so it is in the reducer, not in `v1/`.

Task 1.1 still captures the Claude adapter's, Antigravity's and Copilot's allowed commands, to record how each delivers output and confirm the text-block path. It also records which agents call `terminal/create` when `clientCapabilities.terminal` is offered, as evidence for `agents-client-terminals`.

**As built: the task 1.1 captures (2026-11-26).** Each agent was asked to change `apple` to `banana` on line 3 of an existing `notes.txt` and to run `seq 1 4`, every permission allowed, once with today's `initialize` and once also offering `clientCapabilities.terminal: true`. The logs are `remote-agent/tests/fixtures/acp/{antigravity-1.5.16,claude-agent-acp-0.112.0,codex-acp-2.12.0}-edit-run[-terminal].log`, scrubbed as the earlier ones.

| Agent | The edit | The command's output | `terminal/create` |
| --- | --- | --- | --- |
| claude-agent-acp 0.112 (Haiku) | a `diff` on a `tool_call_update` before the permission and again on the request and the completing update; `oldText` and `newText` are the **replaced fragment** (`"an apple a day"`), not the file, so the hunk's line numbers count from the fragment. Its own `_meta.claudeCode.toolResponse.structuredPatch` holds the file's hunks, which Brindle doesn't read | a text block, ```` ```console ```` fenced, on the completing update; before it a text block of the command's description, replaced by the output. `rawOutput` repeats the text | never |
| antigravity 1.5.16 | the request's `toolCall` holds a fragment `diff` (both texts present); the completing `tool_call_update` holds a `diff` of the **whole file** (`oldText` and `newText`) | a text block on the completing update: a header (`Created At`, `Completed At`, `Command exited with code 0.`, `Command output:`) then the output with `\r\n` line ends; `rawOutput` is `{output, exit_code}` | never |
| codex-acp 2.12 | no diff: it edited through the shell (`python3 -c …` in one run, `sed -i` in the other) | `_meta.terminal_info` on the `tool_call` (with `terminal` content), then `terminal_output_delta` and `terminal_exit` on the completing update, as in the reject capture. Its file read (`kind: read`, `exec_command`) sends a `terminal_output_delta` for a terminal no `terminal_info` or `terminal` item opened | never, though offered |
| copilot 1.0.116 | not captured: its `session/new` failed with "GitHub CLI token lookup timed out after 5 seconds" in the capture's environment (twice) | | |

- No agent called `terminal/create`, so `agents-client-terminals` has no agent to test against today.
- Every new log replays through the reducer with no fault (`every_captured_session_reduces_with_no_fault_and_the_clients_fold_agreeing`), and every line decodes (`every_line_of_the_real_agents_captures_decodes`).
- No agent delivers output a third way: Codex through `_meta`, the others as a text block. Flags 16 to 18 in decision 17 follow from the table.

**The `terminal` client capability stays unoffered here** (user, 2026-11-25). It is ACP v1's way for the agent to have the client run a command (`terminal/create`, `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, `terminal/release`). That changes who runs the command and what an agent can make the server do, and v2 has no such methods.

### 7. The reducer (`acp/reduce.rs`)

- **Upsert.** `tool_change` applies `title`, `kind` and `status` as today, and now `content`, `locations`, `input` and `result`, each a `Patch`. It emits `Event::ToolChanged` for the scalar fields that differ, and `Event::ToolItems` for content (decision 8). An update that changes nothing emits nothing, as today.
- **A diff is diffed once.** The reducer turns `RawDiff` into a `FileChange` with `diff::hunks`, then drops it. An agent that resends the same content list (the Claude adapter resends it with each update) produces equal items, and equal content emits no event.
- **A permission request's `toolCall`** is applied to the tool call it names before the permission entry is added.
  - The request applies its `content`, `locations` and `input`. It never applies its title, kind or status, since the request's title may differ (Copilot's does).
  - When the log has no entry for that call, one is added from the request's fields first.
  - So the entry a permission row points at holds whatever the agent sent in either place.
- **Output** goes through the open outputs (decision 6).
- **Entry size.** The fold re-measures a tool entry when `ToolItems` or `ToolChanged` changes it, and grows it incrementally for `ToolOutput`, as for `Appended` (decision 8).
- **The trim.** `Step::trim`'s exhaustive `match` on `Event` gains the two new kinds, and the trim forgets `outputs` beside `tools`.
- **Loads.** A muted load counts tool updates as today, so none of their content is applied. A shown load adds them with their content.

### 8. Events and the fold (`brindle-transcript`; `process R4`–`R6`)

```rust
Event::ToolChanged { id, title?, kind?, status?, locations?: Vec<Location>, input?: Option<String>, result?: Option<String>, more?: u32 }
Event::ToolItems  { id, from: u32, items: Vec<ToolItem> }   // the content from index `from` on is replaced by `items`
Event::ToolOutput { id, item: u32, text: String, exit?: Exit }  // appended to an Output item's text
```

- **`ToolItems`** covers every case with one rule:
  - a replaced list is `from: 0`;
  - an appended item is `from: len`;
  - `from: 0` with no items clears.
  - The core sends one item per event when two wouldn't fit `TEXT_CHUNK` together.
- **`ToolOutput`** appends, then the fold drops whole lines from the start while the text is over `ITEM_LIMIT`, adding their bytes to `omitted`. A single line longer than the limit keeps its last `ITEM_LIMIT` bytes, cut at a character boundary. Either way the text kept is the longest suffix that fits. So folding `a` then `b` equals folding `a + b`, which a property test pins and the checkpoint's merge relies on. The rule is in the shared fold, so both sides compute the same text. The entry's size grows by the appended text's `escaped_len` and shrinks by the dropped bytes', with no re-encode per piece.
- **Fold errors** (`railway R2`):
  - `NotATool` as today;
  - `ItemsOutOfRange { id, from, len }` when `from` is past the end;
  - `NotAnOutput { id, item }` for `ToolOutput` on a missing or other item.
  - The log is unchanged on an error, as for every `FoldError`.
- **Below a window** (`agents-log-paging`): `entry_change` names `ToolItems` and `ToolOutput` by their `id`, so a paged copy ignores them for entries it doesn't hold, and a later fetch brings the entry's current content.
- **Replay.** `replay_entry` emits a tool entry as `Added` with no content, then one `ToolItems` per item, an output's text already within the limit. `page` and `fetch` use it, so pages carry content the same way. `fold(replay(t)) == t` holds, and the paging laws gain tool content through `fuzz::random_transcript`.
- **Versions.**
  - Client protocol 4 has not been released, and the user decided on 2026-11-25 that the agent series amends it instead of adding versions. So protocol 4 gains two event kinds and optional fields.
  - "Transcript event forms are fixed for each protocol version" stays true, since these are protocol 4's forms. It gains a scenario that a tool entry with no content encodes as before, as login methods and titles did.
  - New files are added to `tests/fixtures/v4/` for each new form. None is edited, and the old ones must still pass, which pins that the new fields are omitted when unset.
  - The protocol's own transcript tests don't change, since `TRANSCRIPT` and `AGENT_PAGE` carry events they don't read.
  - If a release ships protocol 4 before this change lands, the new kinds need protocol 5 and a gate, and this decision is reopened.
- **The checkpoint** is format 2, amended the same way (unreleased).
  - The log's entries carry the new fields.
  - The base's and every segment's state gain `outputs`, the open outputs with their filter state. Absent means none, so a format 2 checkpoint written before this change decodes unchanged, and format 1 is still read.
  - A segment merges consecutive `ToolOutput`s for one item up to `ITEM_LIMIT`, as it merges `Appended`s, keeping the later exit.
  - `decode` checks that every open output names a tool entry and an `Output` item it holds, as it checks `tools`.

### 9. What ACP v2 maps to (for `agents-acp-v2`)

| ACP v2 | Form |
| --- | --- |
| `tool_call_update` upsert: omitted, `null`, value | `Patch::Keep`, `Clear`, `Set`; a concrete list replaces (`ToolItems from 0`) |
| `tool_call_content_chunk` | one item appended (`ToolItems from len`) |
| `diff { changes, patch }` | one `FileChange` per change, its `FileOp` from `operation`; hunks read from the patch's section for that path (a pure `acp/v2/patch.rs` parser of Git's `--patch` text, bounded as decision 5); no patch, or a format other than `git_patch`, gives no hunks |
| `terminal` content, `terminal_update`, `terminal_output_chunk` | `OutputForm::Opened` with `command` and `cwd`, an `output` replacing the text, a chunk as `Appended`, `exitStatus` as `Exited` |
| `content` text, other blocks, `session` references, unknown content | as v1: `Text`, or a placeholder |
| `paused`, `cancelled`, unknown statuses and kinds | new `ToolStatus` values, which `agents-acp-v2` adds |

Nothing in `brindle-transcript`, `reduce.rs` or Brindle changes for these: `agents-acp-v2` writes `acp/v2/` and `Dialect::V2`'s arms.

### 10. The chat model (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **Parts.** A tool entry's first block is its row.
  - When the row is open, its detail follows as more blocks of the same entry (`part` 1, 2, …), as an agent message's parts do:
    - one per text item, rendered with `markdown::render` under `catch_unwind` as messages are, and kept in the Markdown cache by entry and item;
    - one per `DETAIL_LINES` = 50 lines of a diff or an output;
    - one for the locations;
    - one each for the raw input and result.
  - `BlockBody` gains `ToolDetail(DetailPart)`, and `BlockBody::Tool` gains `summary` and `open: Option<bool>`.
  - So the scroller lays out only the parts on screen, however long the diff.
- **Open state.** The user decided on 2026-11-25: a row is open while its tool call runs and closed when the call finishes, and both are configurable. A row's state is derived, never stored on its own (`types R10`):

  ```rust
  pub enum Shown { Open, Closed }                       // config's "open" | "closed"
  pub struct ToolPolicy { pub running: Shown, pub done: Shown }
  pub enum RowState { Open, Closed, WithPermission, Nothing }
  pub fn row_state(entry, by_hand: Option<Shown>, running: bool, pending_permission: bool, policy: ToolPolicy) -> RowState
  ```

  - `row_state` returns, in this order:
    - `Nothing` when the call has no detail;
    - else `WithPermission` while a pending permission names it (below);
    - else the user's own choice for that row, when there is one;
    - else `policy.running` or `policy.done`.
  - A tool call is **running** while all three hold:
    - its status is pending or in progress;
    - the session's turn is running (`Working`, `Blocked` or `Cancelling`);
    - its entry comes after the latest user message the zone holds, or the zone holds no user message, the turn's prompt being above the page.
  - Otherwise it has **finished**. So these are finished: a call that arrives completed, a call left in progress by a stopped turn, and every call of a restored conversation's earlier turns.
- **By hand.** `by_hand: BTreeMap<EntryId, Shown>`, as thoughts' `toggled`.
  - `toggle_tool(id)` flips one row from its current state and records it.
  - `toggle_tool_details()` opens every tool call that has a detail when any is closed, else closes them all, recording each.
  - Both return `Splice`s.
  - A recorded choice outlives the call's completion and a config reload. It is the zone's and isn't kept across a restart; a fetched page's rows start from the policy.
- **The policy is a value** (`fcis R3`). The view reads `[agent]`'s two keys from the settings and hands the model a `ToolPolicy`. `set_policy(policy)` returns the `Splice`s for the rows that change, so a reload applies at once.
- **Finishing closes a row at once.** With the defaults, a quick call's row opens and closes within one paced wake and may never be seen open; its line still carries the summary. If that proves jarring, the remedy is a minimum visible time: a third key and a `now` argument to `row_state`. It isn't in this change.
- **The summary** on a row (`summary(entry) -> Option<Summary>`, pure):
  - `+A −R` over a call's diffs: lines added and removed, hunk lines only;
  - `exit N` or the signal's name for its last output with an exit.
- **What a detail holds**, in order (`detail(entry) -> Vec<DetailPart>`, pure): the content items, then the locations, then the raw input and the raw result, only when there are no content items.
- **A pending permission** whose `call` names a tool entry the model holds shows that entry's detail parts as blocks of the permission entry, before its confirmation, whatever the policy and the user's choice for the row.
  - While it does, the tool row shows its line alone (`RowState::WithPermission`), so the detail is on screen once, next to the buttons.
  - The parts sit inside the dangerous row of `agents-zone-attention` (its destructive tint and border). The diff and code surfaces keep their own colours on it.
  - Once answered, the parts go from the permission entry, and the tool row is whatever `row_state` then says. With the defaults, it is open again while the allowed call runs.
  - A permission naming no call, or a call the model doesn't hold, shows as today. With decision 15 a held page always holds it.
- **Absorbing changes.** `shared.rs`'s fold gains `Dirty::ToolItems { from }` for replaced items and `Dirty::Output { item }` for appended output. `absorb` then re-renders only those items' parts and splices only an open row's last output part as output grows.
- **The working row and the copy actions** read the held entries as today.
- **Copy text** (`copy_text(part)`, pure):
  - a diff copies as unified diff text (`--- path`, `+++ path`, `@@ -a,b +c,d @@`, the lines);
  - an output copies as its text;
  - a text item copies as its source.
- **Marks for what was left out** are lines of the detail: "N more lines not shown", "the first N bytes of output aren't kept", "N more items not shown".

### 11. The zone and `brindle-ui` (`ports R2`)

- **`Tool`** gains `.summary(text)`, `.open(Option<bool>)` and `.on_toggle(…)`. With `Some`, the row shows a chevron, and a pointer press on it asks the owner to flip it, as `Reasoning`'s trigger does. It still takes no focus.
- **`Diff`** (new, plain data in):
  - A header with the operation's word, the path (and `from → to` for a move or copy), and a pointer-only copy button.
  - Then its lines on `muted`, as a code block's, each with a sign column (` `, `+`, `−`), the old and new line numbers, and the text in the owner's code family.
  - Added lines are on `diff_added`, removed on `diff_removed`.
  - Hunks are separated by a `border` line.
  - The sign column means colour is never the only cue.
- **Output** draws with `agents-markdown`'s `CodeBlock`: the command as its label, the exit beside it, the copy button, the text in the code family.
- **Tokens.**
  - `diff_added` and `diff_removed` are added to `Tokens`.
  - They are made in `tokens_over` by a `legible_tint(base, text, colour, max)` generalised from `destructive_tint`: `muted` mixed towards the theme's green (`ansi[2]`) or red (`ansi[1]`), in 1% steps up to `DIFF_TINT_MAX` = 20%, as far as keeps `foreground` at 4.5:1.
  - So they are in both the window and zone sets, and an agent zone's diffs follow its profile's theme through the token scope.
  - The neutral sets (`Tokens::neutral_light`, `neutral_dark`) take fixed tints.
  - `ui_tokens_are_legible_for_every_builtin` gains `foreground` on each, and a check that the two differ from `muted`.
  - They aren't exported by `--export-ui-tokens`, as `destructive_tint` isn't.
- **The action** `toggle_agent_tool_details` ("Toggle Tool Details", no default key) is handled only by `AgentView`, so the palette lists it only in an agent zone (agent-zones "Agent commands in the palette").
- The detail adds no keyboard stop: "Keyboard path through an agent zone" is unchanged. A keyboard user opens details with the action, and a pending permission shows its detail on its own.
- **Following the end.** While the zone follows its end (`Follow::AtEnd`), output appended to an open row stays in view through the scroller's existing rule; the working row is still the last item.
- **Motion.** Nothing here moves, so the motion clock is unchanged.
- **Config** (`src/config.rs`, parsed at the boundary, `types R5`):
  - A new `[agent]` table has `tool_details = "open" | "closed"` (default `"open"`) and `tool_details_when_done = "open" | "closed"` (default `"closed"`), each decoded into `Shown`.
  - Another value is an invalid config, reported as every invalid value is.
  - `assets/default-config.toml` documents both.
  - They are global: an agent profile has no key of its own for them, and the settings dialog doesn't edit them (it doesn't edit agent profiles either).
  - `agents-plans` adds `plan_details` and `agents-prompt-context` adds `attachments_dir` and `attachments_keep_days` to the same table.

### 12. The dump (`testing R2`)

A tool's entry line gains `open=yes|no|permission|none` (`RowState`'s four values) and `items=N`, after `status=` and before its title.
- `permission` means a pending request shows its detail; `none` means there is nothing to open.
- When the call left items out (its `more`), ` more=M` follows `items=N`; when the row's line has a summary, ` summary="…"` follows those (`items=2 summary="+1 −1 · exit 0"`). Each prints only when there is one (flags 31 and 33).
- After the entry line come these lines, whatever the row's state:

```
--- entry 4 tool call="c1" kind=edit status=completed open=no items=2 "Edit main.rs"
--- item entry=4 index=0 kind=diff op=modify path="/p/main.rs" added=1 removed=1 omitted=0
--- hunk entry=4 index=0 old=10 new=10 " fn main() {\n-    old();\n+    new();\n }"
--- item entry=4 index=1 kind=output command="cargo test" exit=0 omitted=0 "ok\n"
--- location entry=4 path="/p/main.rs" line=10
--- raw entry=4 which=input "{…}"
```

- **The fields.**
  - `kind` is `text` (with `cut=yes|no` and the source), `diff` or `output`.
  - `op` is `add`, `delete`, `modify`, `move`, `copy` or `other:NAME`, with `from="…"` for a move or copy.
  - `command=` and `cwd=` print only when set.
  - `exit` is a number, `signal:NAME` or `none`.
  - Texts are escaped with `{:?}`, as entry lines are.
  - `--- raw` lines print only when the detail shows them.
- **The permission line.** A permission entry line gains ` detail=N` after `outcome=`: the number of the named call's parts shown with it, printed only when N > 0. So every existing check of a permission line with no detail still matches byte for byte, `agent.rs`'s `ends_with("outcome=pending \"Write a file\"")` among them. No existing e2e expectation quotes a tool line.
- **Waits.** `wait_text` holds the same lines, so `--wait-for 'kind=diff'` works.
- **The block lines** (`--- block`, `--- link`) are unchanged. A text item's Markdown isn't listed as blocks, since its source is on its `--- item` line.

### 13. Failures (`railway R6`, `R8`)

Each failure is shown in the conversation of the session it belongs to, and none ends the session:
- a content item that can't be read: left out, with one notice per tool call;
- content past a bound: kept up to the bound, with the count in the detail;
- a fold error on a new event: the zone's existing "can no longer show the session" state;
- a Markdown render panic in a text item: the source as plain text, as for messages.

### 14. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

- **Checkpoints.** Tool content is in the transcript, so it is checkpointed with it at the existing moments, appended as events in segments or written whole (`agents-log-paging` decision 9).
- **A core restart.** The holder's kept stdout since the last checkpoint goes through the reducer again. The diff and the output filter are deterministic, so the log comes out the same.
- **A Brindle crash** loses only the open state of rows; the content replays on attach.
- **An upgrade's** next core reads the same format 2.
- **A restart** (Continue, or after a terminal login) carries the log on, content included. Its open outputs are closed, since a new agent process has no terminals of the old one.
- **Confinement.** Agent output is still handled under the reducer's `catch_unwind`, per session (`process R3`).

### 15. Pages, fetches and tool content (`agents-log-paging`)

- **The page reaches what a waiting permission asks about.**
  - `Transcript::page` already lowers its start to the oldest permission with no outcome.
  - It now also lowers it to the tool entry that any such permission names: the newest `Entry::Tool` with that `call` before the permission.
  - With at most 16 waiting permissions, the search is bounded by the log's length.
  - So the zone always holds the entry whose detail a pending permission row shows, however much the turn produced after it. Without this, a long turn's permission would show its title and buttons with no edit, which undercuts "nothing runs unseen".
  - The page law "holds every pending permission" gains "and the tool entry each names", and agent-sessions "Attach replays the transcript" says so.
- **A tool entry larger than the page.** A tool entry can encode to more than `PAGE_BYTES` (256 KiB of items plus raw values and escaping). A page or fetch then holds it alone, as for an agent message larger than the page. The budget is unchanged.
- **Below the window** (decision 8). Changes to tool entries the zone doesn't hold are ignored and fetched current later, which "Entries a client hasn't fetched stay current" now pins for output.
- **The zone.** Fetched tool entries' rows start from the policy. Their calls are finished, since their turn's user message is held and is older than the turn's end, or the turn has ended. They raise nothing (`from_replay`).

### 16. The built agent stack

- **`agents-markdown`, `agents-markdown-layout-fixes`:** a text item renders through `markdown::render` and draws through `Response` at the conversation's width. A tool detail is drawn beside the bubbles, not in one.
- **`agents-zone-theme-scope`:** the detail is drawn in the zone's token set, and the diff tokens are in both sets (decision 11).
- **`agents-zone-attention`:** the pending permission row is the destructive alert, and its detail sits inside it (decision 10). The working row is unchanged and reads the held entries. Nothing here wants the motion clock.
- **`agents-session-title`:** `Patch<T>` and `session_info_update` move into `forms.rs` and `v1/` (decision 2). The title is untouched.
- **`agents-session-recovery`, `agents-login`:** their requests move behind `Dialect` (decision 2). A muted load still applies no tool content. A restart closes open outputs (decision 14).
- **`agents-log-paging`:**
  - pages and fetches carry tool content by `replay_entry`;
  - the window ignores the new events below it;
  - the page reaches a waiting permission's tool entry (decision 15);
  - segments carry the new events and merge `ToolOutput`s (decision 8);
  - no message number changes.
- **Archive order:** `agents-log-paging` archives first (on #144); this change archives in its own implementation PR, after it. Every MODIFIED block below carries the main specs' text as of 20e9858. Task 10.3 re-checks them against the synced main specs before archiving.
- **Open changes that meet this one:**
  - `agents-commands` puts its forms in `forms.rs`, reads its options in `acp/v1/`, and numbers `AGENT_SET_OPTION` 24.
  - `agents-plans` and `agents-prompt-context` add keys to the `[agent]` table this change creates.
  - `agents-prompt-images` builds on `agents-prompt-context`.
  - `agents-acp-v2` maps v2 onto these forms (decision 9).
  - `agents-client-terminals` reads task 1.1's record.
  - No requirement of theirs is modified here, and none of them modifies one this change does.

### 17. Flags: choices the refresh made (decide and flag)

The user's decisions of 2026-11-25 all still fit (next section). These choices are new from the refresh. Each records the option taken and why, for the user to confirm or overturn:

1. **Codex's `_meta` convention is read now, and the command output parts stay in this change.** The first design left this to the captures of task 1.1, which may move the output parts to `agents-acp-v2`. An existing capture already shows Codex's output reaching Brindle only through `_meta.terminal_output_delta`, so decision 6's rule says read it. The keys are `terminal_info`, `terminal_output_delta` and `terminal_exit`, not the guessed `terminal_output`. Recommend: accept. Task 1.1 still captures the others.
2. **Open outputs live in the reducer, keyed by terminal id** (`outputs`, decision 6), version-free and checkpointed. The cap is 64 per session, the oldest closed. A replaced content list keeps the output of a terminal it still names. Recommend: accept. v2's agent-owned terminals need the same map.
3. **An output's label comes only from `terminal_info.command`.** Codex sends none, and its row's title is already the command. Reading `rawInput.command` would read an agent's private shape. Recommend: accept.
4. **The checkpoint is format 2, amended, not format 1.** The first design predates paging. Base and segments gain `outputs`, and segments merge consecutive `ToolOutput`s, which rests on the fold law "folding `a` then `b` equals folding `a + b`", pinned by a property test. Recommend: accept.
5. **The page reaches the tool entry a waiting permission names** (decision 15). This modifies agent-sessions "Attach replays the transcript". Recommend: accept. The alternative leaves a long turn's permission row without its edit.
6. **A tool entry may be bigger than `PAGE_BYTES`**, and is then a page alone. The budget is not raised, and `CALL_LIMIT` is not lowered to fit it. `ToolOutput` grows `Kept::size` incrementally. Recommend: accept.
7. **Running is judged from the held entries:** a call is running when it comes after the latest user message the zone holds, or when the zone holds none. Recommend: accept, as the working row already reads the held entries.
8. **The dump's ` detail=N` prints only when N > 0**, so existing permission lines and checks match byte for byte. `open=` and `items=` always print on tool lines, which no e2e check quotes today. Recommend: accept.
9. **Diff colours are tints of `muted`**, the code block's surface, not of `card` as the first design said. They go up to 20% by a `legible_tint` generalised from `destructive_tint`, are in both token sets, and aren't exported. Recommend: accept.
10. **"Toggle Tool Details" is listed in agent-zones "Agent commands in the palette"** (MODIFIED), rather than only in a scenario of its own requirement. Recommend: accept.
11. **"Transcript event forms are fixed…" gains a scenario that a tool entry without content encodes as before**, as login methods and titles did. Recommend: accept.
12. **A restart closes the session's open outputs** (decision 14). Recommend: accept.
13. **A perf case `agent-tool-output`** sits in `tests/perf/agent.rs` beside `agent-markdown-stream` and `agent-attach-long`. The fake agent streams 1 MiB of output in 4 KiB `_meta` deltas into an open row, timed from the gate to the last line shown (`testing R14`). Recommend: add. It is the check on "a long output costs the view only what is on screen".
14. **`v1/decode.rs` decodes a tool call's fields by hand around the crate.** The crate decodes the update with `content`, `locations`, `rawInput`, `rawOutput` and `_meta` taken out, and items one at a time. Recommend: accept. It is decision 3's "one unreadable item loses only itself", which today's whole-update decode breaks.
15. **The move also takes `continued`'s `"session/load"` method check** into the dialect, which the first design didn't list. Recommend: accept, since R14 forbids method names outside `v1/`.

Flags added while implementing (2026-11-26, sections 1–4):

16. **Output for a terminal nothing opened opens one.** Codex's file read sends `terminal_output_delta` with no `terminal_info` and no `terminal` item (decision 6's As built). An `Appended` (or `Exited`) for an unknown terminal id opens an output at the end of the update's own tool call, with no command or `cwd`, rather than dropping the text. Recommend: accept; dropping it would lose the only place the read's result shows.
17. **A fragment diff's line numbers count from the fragment.** The Claude adapter's `oldText`/`newText`, and Antigravity's in its permission request, are the replaced text alone, so `diff::hunks` numbers the lines from 1 within it. The row shows them as given; the file's own numbers are only in agent-private `_meta`, which decision 6 doesn't read. Recommend: accept, and note it in the dump's docs; a later change can hide numbers for a change the reducer can't place.
18. **Copilot's edit and command aren't captured.** Its `session/new` failed in the capture's environment (a GitHub CLI token lookup timeout), and the capture wasn't retried with the token passed in. Copilot's tool content is therefore read from the reject capture only (`rawInput` as a patch, no content items). Recommend: capture it by hand before the PR, with the user's go-ahead.
19. **An output's kept text tracks whether it starts mid-line** (`Output.partial`, omitted when false). Without it, "folding `a` then `b` equals folding `a + b`" fails when a line longer than `ITEM_LIMIT` was cut at a character boundary and a later piece then ends it: the whole-text fold drops the cut line's tail, a piecewise fold can't tell it was cut. The property test found the case by construction. Recommend: accept; it is one boolean in protocol 4's (unreleased) output item, and the zone can show it as "the start of this line isn't kept".
20. **`Rpc` keeps the session's `Dialect`**, rather than the reducer holding a second copy: every line `Rpc` writes (a permission answer included) needs it, and the reducer reaches it as `rpc.dialect()`. `initialize` is always written in `Dialect::newest()`, and its answer sets the session's (`Rpc::speak`), as a checkpoint's `acp` does on decode. `Request::Continue { how: Continued }` stands for the design's `Resume` and `Load`, and `Dialect::continue_method(how)` names the method in the "its answer to session/load isn't one Brindle can read" text, so that text stays as it was without a method name outside `v1/`. `AUTH_REQUIRED` and `UNSUPPORTED_VERSION` sit in `forms.rs`, as the codes every version's errors are read by. Recommend: accept.
21. **`initialize`'s answer is read for its version first.** An answer naming a version Brindle doesn't speak now always fails with "the agent speaks ACP version N", even when the rest of it isn't v1's shape (before, such an answer failed as unreadable), and N may be past 65535. An answer with no whole-number `protocolVersion` is unreadable, as before. Recommend: accept; it is the order decision 2 asks for.
22. **A terminal described after its output opened.** When a `[terminal]` item names a terminal before any `terminal_info` does, its `Output` opens with no command or directory. A later `terminal_info` for that open terminal sends the output again (`tool_items` from its index, its text kept) with them, when it is the entry's last item and has neither yet; otherwise it changes nothing. Codex sends both in one update, so its captures never take this path. Recommend: accept.
23. **An output stays open until its exit, not its call's end.** Codex's file read streams output and never sends `terminal_exit`, so its output stays open (its text whole, its exit absent) after the call completes, until the entry is trimmed, a restart, or the 64-output bound closes it. Closing it at `completed` could drop an exit sent in a later update, which the captures don't rule out. Recommend: accept; revisit if a capture shows an agent sending exits before completion.
24. **Myers is called directly, not through `TextDiff`.** `TextDiff` picks its algorithm at run time, so every algorithm `similar` has is built into the helper: the static musl helper grew from 3,822,464 to 4,760,448 bytes (+24.5%), about 760 KB of it `similar`'s. `diff.rs` now calls `algorithms::myers::diff` through a `Replace` hook (which orders a change's removed lines before its added ones, as `TextDiff` does) and `group_diff_ops`, with the same results under the same tests, and the helper is 4,023,168 bytes (+200,704 bytes, +5.3% over 3,822,464, `similar` about 38 KB of it). Recommend: accept.
25. **A pending permission's detail is drawn inside its one `Permission` block**, as the `Confirmation`'s children (`BlockBody::Permission { detail, shows }`), not as list blocks of their own, so the destructive alert stays one element round its detail and its buttons. It is bounded as the tool entry is (`CALL_LIMIT`), so the block's height is too. Recommend: accept.
26. **The detail's pure functions live in a new pure module, `src/agent/tool.rs`** (`detail`, `summary`, `copy_text`, `row_state`, `continues_into`, `ToolPolicy`, `RowState`), not in `model.rs`, which calls them and keeps the cache and the splices. It joins `PURE_MODULES` and `fcis`'s table. `Shown` is `config.rs`'s, re-exported there. The diff rows are `brindle-ui`'s plain `DiffRowData` (aliased `DiffRow`), so the view hands them to `Diff` as they are (`process R14`). Recommend: accept.
27. **A tool or permission entry's change splices the least range**: `redraw` keeps the blocks its old and new drawing share at the head and the tail, so a growing output replaces its last part alone, and a replaced list from its first changed part (the row's line too when its summary changes). Recommend: accept.
28. **Every entry-change event below the window records nothing** in `shared.rs` (`Appended`, `ToolChanged`, `ToolItems`, `ToolOutput`, `Answered` for an entry the fold doesn't hold), not only the new ones: before, such a change marked an entry the zone didn't have. Recommend: accept.
29. **`legible_tint` takes the colour's own minimum** (`colour_min`): `destructive_tint` passes 3.0, so its results stay byte for byte, and the diff tints pass 1.0 (no check), since their sign column, not their tint, carries the meaning. Recommend: accept.
30. **A long diff's or output's runs read as one frame**: `Diff` and `CodeBlock` gain `.joined(above, below)`, a run joined above drawing no header, and the view asks `tool::continues_into` whether the next block goes on with the same item. An output's label (`cargo test · exit 1`, `output` with no command) is built with its part (`OutputHead::label`). Recommend: accept.
31. **The dump doesn't show a row's summary** (`+1 −1 · exit 1`): decision 12's tool line has no field for it, so task 8.1's "the row's summary" was checked by `model.rs`'s unit tests only. Decided (recommended option): the tool line gains ` summary="…"`, printed only when the row has one. It goes after `items=N` rather than before it, so the spec's and the cases' `open=… items=N` checks still match; command-line "Tool call counts and summaries in the dump" (ADDED) says so, and `agent-tool-diff` and `agent-tool-output` check it.
32. **A permission request naming a call the agent never sent adds that tool entry first** (task 4's rule), so the request is the entry after it: `agent-permission-keys`' expectations move from entry 2 to entry 3 (`permission:3:reject-once`), and `agent-quit-restore` expects the tool entry between the prompt and the request. Recommend: accept; it is the rule the request-only content needs.

Flags added while writing the remaining cases (2026-11-26, section 8):

33. **The dump gives a call's items left out** (` more=M` after `items=N`, only when above 0). Task 8.4's "a 40-item call shows its count" had no field to read: the count is the detail's "N more items not shown" mark, which the dump doesn't list. Recommend: accept; additive, like flag 31's `summary=`.

## The user's decisions

None is open from the first design. The user settled each on 2026-11-25, one at a time:
- `Dialect` is a closed enum (decision 2).
- The `_meta` terminal convention is read for v1 when a capture shows it is an agent's only way to deliver output, and the `terminal` client capability stays out, as `agents-client-terminals` (decision 6). The refresh found that capture for Codex (flag 1).
- The diff comes from the `similar` crate, which the user chose over the recommended hand-written one (decision 4).
- A tool row is open while its call runs and closes when the call finishes. The user chose this over the recommended closed rows, and over closing at the turn's end. Both states are configurable in `[agent]`, and a minimum visible time is left for later, if closing at once proves jarring (decisions 10 and 11).

Accepted as recommended:
- raw input and result are shown only when a tool call has no content items, with no second trigger (decision 10);
- the bounds of decision 5, 64 KiB an item and 256 KiB a call, so a single-file diff past about 1,000 changed lines shows its first 64 KiB and a count;
- a call left in progress when its turn ends counts as finished;
- a pending permission's detail shows once, with the permission row;
- the two settings are global (decisions 10 and 11).

The refresh's choices are decision 17's flags.

## Risks / Trade-offs

- **A long diff is cut at 64 KiB.** The row says how many lines aren't shown, and the agent's file is on disk to read.
- **The line diff isn't always minimal**, since `similar` forces a split when a search gets expensive. It is still correct, and bounded work on the core's thread is the point.
- **A new dependency in the helper that is copied to remote hosts.** It is one package with no dependencies of its own, named in one file, with the lock diff and the musl build checked in the tasks.
- **A quick tool call's row closes before it can be read.** Its line keeps the summary, the row opens by hand and stays open, `tool_details_when_done = "open"` keeps every row open, and a minimum visible time is the next step if needed.
- **Command output loses its colours.** It is plain and legible; drawing it through the emulator is a later change.
- **The Claude adapter resends content with every update**, so each resend costs a diff on the core. The prefix and suffix skip makes that proportional to the file's size, and equal results emit nothing.
- **The `_meta` convention is Codex's, outside the spec.** It could change in a Codex release. It is confined to `v1/`, pinned by the capture, and a change costs output, never the session.
- **Amending protocol 4 and checkpoint format 2 assumes no release ships them first.** Decision 8 says what changes if one does.
- **Moving the v1 code touches files the archived changes edited.** The move is one commit with no behaviour change, checked by the unchanged tests.
- **A page holding a waiting permission's tool entry can reach further back than its budget.** It is bounded by 16 waiting permissions, as paging's own rule is.

## Migration Plan

Nothing to migrate: the two config keys are new and optional, and no state file or released protocol changes.
- A Brindle older than this change reports `[agent]` as an unknown key and starts with its defaults, as it does for any key it doesn't know. The release notes say so.
- A session checkpointed before the change keeps its tool rows as one line; content appears for tool calls made after it.
- A pre-change Brindle attached to a new core gets `tool_items` events it can't decode, so its zone says it can no longer show the session (the session stays, detached). This is a pre-release pairing only, as in `agents-log-paging`'s "What a pre-release build meets".
