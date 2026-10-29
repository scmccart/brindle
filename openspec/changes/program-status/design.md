# Design

## Context

Brindle already scans output beside alacritty's parser for the OSCs it drops: `notify::osc::Scanner` finds OSC 9 and 777 on the PTY reader thread (`ScanReader`, before parsing) and in `Terminal::feed` for tmux panes, and the shell-integration `Parser` finds OSC 133 marks under the term lock. Bells and notices reach the `Workspace` as `TerminalEvent`s, which `TmuxWindowView` re-emits for its panes, and `Workspace::on_attention` turns them into the bell dot and desktop notifications.

The protocol defines its records per terminal (window, tab or pane), so Brindle's unit is the `Terminal` entity: one per local tab and one per tmux pane. tmux copies pane output into `%output` unchanged, OSC 7501 and OSC 133 included (checked with a raw `tmux -C` client on tmux 3.6), so panes get the same treatment as local tabs; the query reply goes back through the pane's input (`send-keys -H`), as palette replies do.

## Goals / Non-Goals

**Goals:**
- Parse reports exactly as the specification says, including every limit and discard rule, with the parser and record set pure and unit-tested.
- Show state where the user looks when they come back: the tab strip, and the desktop when the tab isn't in front of them.
- Make the protocol detectable, so programs that query first start reporting.

**Non-Goals:**
- A separate inbox or list of all records across windows. The tab dot and notifications are the presentation for now.
- Showing `progress`, `title` or `msg` in the tab strip. They are kept in the record (and in the dump) for a later tooltip or status line.
- Mapping OSC 9;4 progress onto the root record. The specification allows it, but ConEmu's progress sequence is widely sent with different semantics, and Brindle already ignores it.
- Terminfo. Brindle ships no terminfo entry; the query is the authoritative detection and is what Claude Code uses.

## Decisions

### One scanner for the side-channel OSCs

`notify::osc::Scanner` grows from "OSC 9 and 777" to the OSCs Brindle acts on beside the parser, and returns `Found` values: `Notice` (as now), `Status(Message)` for OSC 7501, `PromptStart` for OSC 133 `A`, and `Reset` for `ESC c`. It runs on both paths already (the reader thread and `feed`), keeps its state across reads, and never buffers anything but the payload of a wanted OSC, so this adds no new scanning pass.

OSC 133 `A` is taken from this scanner rather than from the shell-integration marks because ordering matters: a report and the prompt that follows it (or precedes it) must apply in stream order. The marks are recorded under the term lock on another thread and reach the entity only as a `Wakeup`, which could reorder a `working` report sent right after a prompt. The scanner sees both in sequence.

Alternative: parsing OSC 7501 in the shell-integration `Parser`. It would have to split the stream at each report for no reason (reports move no marks), and its output is positional, not event-shaped.

### `terminal/status.rs`: a pure parser and record set

`parse(body)` takes the OSC body after `7501;` and returns `Message::Query`, `Message::Clear { id }` or `Message::Report { id, record }`, or `None` for anything the specification says to ignore or discard. It checks every limit before touching a record: key length, `msg` and `title` encoded then decoded sizes, `app` and `id` grammar, base64 (standard alphabet, padding optional, through `base64` 0.22's `Indifferent` padding mode), control characters in decoded text, and the `state` value. Unknown keys are skipped, a malformed pair is skipped, the last of a repeated key wins, and whitespace around keys and values is removed.

Decoded text also has bidirectional overrides and isolates (U+202A–U+202E, U+2066–U+2069, U+200E, U+200F, U+061C) removed, since Brindle shows it only outside the grid, in notifications.

`Records` holds a `BTreeMap<String, Entry>` keyed by id (the root is the empty string) with an update stamp per entry for the 256-record cap (the least recently updated record goes). `apply` replaces the record whole; `clear` removes the id and everything under it (prefix `id/`), or everything with no id. `prompt_start` and `process_exit` drop `working` and `blocked` records and keep `idle`; `seen` drops `done` and `error`; `reset` drops everything. Each returns whether anything changed. `summary()` is the highest state across records, ordered working < done < error < blocked, with `idle` counting as nothing: it is what the tab shows. `effective_app(id)` walks up the path for `app`, as section 6 asks.

### The `Terminal` owns the records and answers the query

`Terminal` keeps `status: Records` and handles what the scanner found on the GPUI side, in order: a notice is emitted as before; a query writes `ESC ] 7501 ; ? ESC \` back to the program (`write`, so a pane's reply goes through tmux `send-keys -H`; tmux doesn't answer this query itself); a report or clear updates the records; a prompt start or reset applies the lifetime rule; `mark_exited` applies the process-exit rule; and `input` (user keys) marks `done` and `error` as seen. Whenever the records change, the terminal emits `TerminalEvent::Status(Option<Notice>)`. The `Notice` is present when a report left a record `blocked`, `done` or `error` with a different `(state, kind, title, msg)` than before, so a second permission prompt notifies again and a repeated identical report doesn't. Its `title` is the record's `title`, else its effective `app`; its `body` is `msg`, else a phrase for the state: "Waiting for permission", "Waiting for an answer", "Waiting for a login", "Waiting for you", "Done", "Failed".

`Source::Osc7501` joins the notification sources, so `BRINDLE_NOTIFY_LOG` lines read `osc7501<TAB><title> — <tab><TAB><body>` and the per-tab OSC rate limit applies.

### The tab shows a summary; tmux tabs summarise their panes

`Tab` gains `status: Option<State>`, refreshed on every `Status` event from its terminal (local) or from any pane (`TmuxWindowView` re-emits `Status` as it does `Bell` and `Notify`, and exposes `status_summary`, the maximum over its panes). It renders as a 6 px dot beside the bell dot, coloured from the theme's palette: `blocked` yellow (ANSI 3), `error` red (1), `done` green (2), `working` the muted foreground. It clears only when the records do (the lifetime rules, a `clear`, or the user typing), not on activation: a `done` the user hasn't looked at should stay marked, and the specification leaves the "seen" moment to the terminal, which here is a key typed into that terminal.

### The dump prints records

`--dump-screen-after` prints one `--- status …` line per record of the active terminal, in id order with the root first: `state=` always, then `id=`, `kind=`, `progress=`, `app=`, `title=` and `msg=` when set, strings Rust-debug quoted. The e2e `Dump` keeps the raw lines in `status`.

## Risks / Trade-offs

- [A program sends reports inside tmux's passthrough or to a classic tmux client] → Classic tmux swallows unknown OSCs as it does OSC 9/777 (README says so); nothing changes there.
- [tmux learns OSC 7501 and answers the query itself] → the program would get two replies, which it ignores after the first. Nothing else is sent back, ever.
- [Shells without integration never send OSC 133 `A`] → a `working` record from a program that died without reporting stays until the tab's process exits or the user types. The specification accepts this; it ties lifetimes to the events the terminal sees.
- [A storm of reports] → each report is bounded (4096 bytes, 256 records) and the notifications go through the existing per-tab and global limits.
