# Design

## Context

`brindle-term-core`'s `osc::Scanner` already collects every `OSC 9` payload and hands it to `parse_osc9`, which returns `None` for the ConEmu sub-commands `9;1`–`9;12`. `shell_integration`'s own scanner takes `9;9` as a working-directory report. What the `osc` scanner finds reaches two places in stream order: the client's `Terminal::handle_found` (through `SessionSink` and `ChannelEvent::Osc`) and the server core's `Emulator::feed`, which keeps `Records` so `replay::render` can send OSC 7501 reports after the screen. Status records already have exactly the lifetime this change wants (`end_activity` on OSC 133 `A` and exit, `reset` on `ESC c`), and the tab strip already aggregates a per-terminal value across zones (`Tab::status`).

## Goals / Non-Goals

**Goals:**
- One pure progress model in `brindle-term-core`, used unchanged by the client and the server core, so replays never drift (`fcis R6`).
- A bar on the tab face and the zone header, drawn from values computed outside `render`.

**Non-Goals:**
- Taskbar or launcher progress. The per-terminal value is the input such a seam would take later; nothing here blocks it.
- Animating the indeterminate bar. A repaint timer per tab would cost a full-window render each frame (GPUI 0.2.2 doesn't cache views), so it is a static faint bar.
- Merging `9;4` into OSC 7501 records. They are separate protocols with separate senders; folding one into the other would make a `9;4;0` clear a 7501 record a program still owns.

## Decisions

### A closed `Progress` enum in `term-core/src/progress.rs` (pure)

```rust
pub struct Percent(u8);            // 0..=100, built only by Percent::clamped
pub enum Progress {
    Normal(Percent),
    Error(Option<Percent>),
    Indeterminate,
    Paused(Option<Percent>),
}
pub enum Report { Clear, Set(ReportedState) } // what one sequence says
pub fn parse(payload: &[u8]) -> Option<Report>;            // the body after "9;"
pub fn apply(current: Option<Progress>, report: Report) -> Option<Progress>;
pub fn most_pressing(it: impl Iterator<Item = Progress>) -> Option<Progress>;
```

`None` is "no progress", so there is no `Clear` stored state to represent. `Indeterminate` carries no percent, and a percent can't exceed 100 (`types R1`, `R4`). `apply` carries the previous percent into `Error`/`Paused` when the report gives none, per the spec. `most_pressing` is the tab and zone aggregation (error, paused, normal by lowest percent, indeterminate), tested once and used by the strip. The module joins `PURE_MODULES` in `tests/architecture.rs`.

Alternative considered: storing the raw state number and percent. Rejected: it lets `Indeterminate` carry a percent and leaves each reader to clamp.

### The scanner returns `Found::Progress(Report)`

In `osc::Scanner::finish`, the `9` arm tries `progress::parse` first and falls back to `parse_osc9`, which keeps ignoring the other ConEmu commands. `Found` is matched without a wildcard everywhere (`railway R3`), so the compiler lists every place that must handle the new variant: `Terminal::handle_found`, `Emulator::feed` and the fuzz target. Malformed payloads return `None` (`railway R5`). The fuzz generator already emits `9;4` payloads.

### Lifetime lives beside `Records`, applied in the same order

Both `Terminal` and `Emulator` hold `progress: Option<Progress>`. `Found::PromptStart` and `Found::Reset` set it to `None` where they already call `end_activity` and `reset`; process exit does the same where `end_activity` runs on exit. Typing does not clear it (unlike `done` records), since a running download is still running when the user types.

### Replay: one `9;4` report at the end of the stream

`replay::render` takes the progress (a new argument beside `&Records`) and writes `ESC ] 9 ; 4 ; <state> ; <percent> ESC \` after the records. It must come after the replayed OSC 133 marks, because a replayed `A` would clear it on the client. Replays are bytes, so no protocol version changes (`process R4`):
- old client, new core: its scanner treats `9;4` as a ConEmu command and ignores it;
- new client, old core: no progress in the replay; the next report shows it (the spec says so).

Core restart: the checkpoint is a replay render, so progress survives a core restart and a step-down upgrade (`process R1`–`R3`, `R12`). A Brindle crash loses nothing server-side; the reattach replay restores it.

### Shell: events and drawing

`Terminal` emits the existing `TerminalEvent::Status(None)` when its progress changes, so `Workspace` refreshes `Tab::status` and the new `Tab::progress` (`progress::most_pressing` over the tab's terminals) in the same handler, outside `render` (`process R14`). `render_tab_bar` and the zone header draw the bar from that cached value with the face's theme colours. The tab face's 2-pixel bar sits inside the face's bottom padding so the pill doesn't change height.

The status side events are dropped under event-channel overload (`EVENT_MARKS`); when that happens the task already resets `Records`, and it resets progress too, so a dropped `9;4;0` can't leave a stale bar.

### Testing (by `testing-architecture.md` layers)

- Unit tests in `progress.rs`: every spec scenario of states, clamping, kept percent, aggregation.
- Unit tests in `osc.rs`: split reads, cancellation, `9;4` vs `9;9` vs a message.
- `replay.rs`/`emulator.rs` unit tests: a rendered emulator rebuilds the same progress; a replayed `A` before the progress doesn't clear it.
- `--dump-screen-after` prints `--- progress state=normal|error|paused|indeterminate[ percent=N]` per terminal (active first), and a `progress` e2e case set covers the tab bar's presence by dump, the prompt clearing it, and attach in `local_server.rs`. The bar's pixels are sampled in one capture case (colour by state).

## Risks / Trade-offs

- [A program that sets progress and exits without clearing, inside a shell without OSC 133] → the bar stays until exit of the shell or the next report. Brindle's shipped scripts emit `A`, and the same limitation applies to 7501 records today.
- [Two-pixel bars are easy to miss] → the tab's status dot stays the attention signal; the bar is ambient information.
- [cargo and others emit `9;4` only when they detect a supporting terminal] → Brindle may need to be detected (e.g. by `TERM_PROGRAM`); that's the program's choice and outside this change. README lists the support, and the implementer checks which detection the common emitters (cargo, winget) use and whether they can be forced on, and documents that.
