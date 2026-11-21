# Design

## Context

Notifications today flow as `osc::Scanner` (`brindle-term-core`) → `Found::Notice` → `Terminal::handle_found` → `TerminalEvent::Notify` → `Workspace::on_attention` (focus rule, settings) → `Notifier::send` (a GPUI global: rate limits in `policy.rs`, then a sink command). The sinks are `notify/dbus.rs` (zbus worker) and `notify/toast.rs` (WinRT), plus the `BRINDLE_NOTIFY_LOG` writer. Sinks answer with `Reply::{Shown, Action, Token, Closed}`, and `Notifier` maps a sink id to a `Target { window, tab, source }` for clicks. Bell notifications already use `replaces`. Program replies to the PTY go through `Terminal::write`, as the OSC 7501 query reply does. The server core's `Emulator` ignores notices, and this change keeps it that way.

kitty's protocol reference: <https://sw.kovidgoyal.net/kitty/desktop-notifications/>. Verify the reply separators against kitty's source while implementing, since the brief for this design was taken from a summary of that page.

## Goals / Non-Goals

**Goals:**
- Parsing and assembly as pure code with unit tests. Entities and sinks only route values (`fcis R1`).
- Every reply Brindle writes is built from validated ids and fixed tokens, so a program can't make Brindle echo arbitrary bytes.

**Non-Goals:**
- Icons, named sounds. Notification history.
- Keeping incomplete chunks across a reconnect. The server doesn't track notices, so a chunk split across a reconnect is lost, and a program resends on the next notice.

## Decisions

### Parse in `brindle-term-core`, assemble in `src/notify/kitty.rs`

`osc.rs` gains `(99, 2)` and `Found::Kitty(kitty::Chunk)`. Parsing lives beside `parse_osc9` because the scanner returns it, but the server core drops it, as it drops `Notice`.

```rust
// term-core (pure)
pub struct KittyId(String);                 // [A-Za-z0-9_+.-]{1,64}
pub enum Payload { Title, Body, Buttons, Close, Query, Alive, Unsupported }
pub enum Sound { Default, Silent }          // `s` decoded; anything but `silent` is Default
pub enum Occasion { Always, Unfocused, Invisible }
pub enum Urgency { Low, Normal, Critical }
pub enum Expiry { Default, Never, After(Duration) }
pub struct Chunk { id: Option<KittyId>, payload: Payload, done: bool, text: String,
                   focus: bool, report: bool, close_report: bool,
                   occasion: Occasion, urgency: Urgency, expiry: Expiry, sound: Sound }
```

`e=1` is decoded during parsing, so `Chunk::text` is always checked UTF-8 with controls removed by the existing `clean`. A bad id or bad base64 is `None` (`railway R5`).

`src/notify/kitty.rs` holds the per-terminal `Assembly` (at most 8 pending, oldest dropped, 2048-byte caps) and turns chunks into an outcome, matched exhaustively by `Terminal`:

```rust
pub enum Step { Pending, Show(Notice99), Close(KittyId), Reply(String), Ignored }
pub fn reply_query(id: &KittyId, caps: Caps) -> String;
pub fn reply_alive(id: &KittyId, open: &[KittyId]) -> String;
pub fn click_report(id: &KittyId) -> String;  pub fn close_report(id: &KittyId) -> String;
```

`Caps` is a value, not a `cfg`: `Notifier` builds it from the platform's sink (`hints: bool` is true for dbus, false for toast; `buttons: bool` is the dbus server's `actions` capability, already read in `dbus.rs`, and true for toast). So the reply's platform difference sits in the sink seam, not in `kitty.rs` (`ports R4`). `Assembly` is GPUI-free and lives in the `Terminal`. Each terminal owns its pending notifications, and ids are per terminal.

Alternative considered: assembling in `term-core`. Rejected: the core would carry state it never uses, and the rate limits and focus rule that decide the outcome are client-side.

### Notifier tracks open OSC 99 notices

`Request` gains `program: Option<ProgramNotice { terminal: WeakEntity<Terminal>, id: KittyId, focus, report, close_report, urgency, expiry }>`. `Notifier` keeps `open99: HashMap<(EntityId, KittyId), sink id>` for replacement, `p=close` and `p=alive`. `Target` gains the same `program` field, so:
- `Reply::Action` → focus (unless `focus` is off) and, with `report`, `terminal.write(click_report)`;
- `Reply::Closed` → drop from `open99`, and with `close_report`, unless the close was the program's own `p=close` (tracked as a pending-close set), `terminal.write(close_report)`.

A `WeakEntity` that no longer upgrades is skipped: the terminal closed, so nothing is written (`railway R6`).

Sink changes:
- dbus: `Command::Notify` gains `urgency` and `expire_ms` (the hint and `expire_timeout`, still `1` and `-1` for other sources) and a new `Command::Close { id }` (`CloseNotification`).
- toast: `Close` removes the toast by tag through `ToastNotificationManager::History`. `Dismissed` reasons other than `ApplicationHidden` count as closes. Urgency and expiry are dropped.

### Buttons and silence

`Assembly` keeps `buttons: Vec<Label>` per pending notification (split on U+2028, empty labels dropped, at most 4, 64 characters each: a `Label` is built only by that check). A completed update carries its own buttons, so an update without `p=buttons` has none, as the spec says. `ProgramNotice` gains `buttons` and `sound`.

- **dbus:** `Command::Notify` gains `buttons: Vec<String>` and `silent: bool`. Actions become `["default", "Show", "1", label1, "2", label2, …]`, sent only when the server advertises `actions`; `silent` adds the `suppress-sound` hint. `Reply::Action { id, action }` already carries the key, so `Notifier` maps `"default"` to the click path and `"1"`–`"4"` to a button report.
- **toast:** the XML gains `<actions>` with one `<action content="…" arguments="button=N" activationType="foreground"/>` per button (toasts allow 5), and `<audio silent="true"/>` when silent. The `Activated` handler reads `ToastActivatedEventArgs::Arguments` (today it ignores its arguments), giving `Reply::Action` the key `"N"` for a button or `"default"` for the body. The toast XML builder becomes a pure function (`toast::xml(summary, body, buttons, silent)`), unit-tested for escaping labels.
- **Button press:** `Notifier` writes `click_report_button(id, n)` (`ESC ] 99 ; i=<id> ; <n> ESC \`) through the `WeakEntity<Terminal>`. A button press never raises the window; `focus` applies only to the body click. The report is written whatever `a` says, since a button with no report would be useless. The implementer checks this, and the button separator and `s`'s base64 encoding, against kitty's source, and adjusts the spec in this change if kitty differs.

A button can't be pressed without a pointer, so the e2e suite checks the sink commands (through `BRINDLE_NOTIFY_LOG`, which records the buttons and `silent`), and the press itself is a manual check.

### Policy

`policy::wanted` gains `Source::Osc99` (the `osc99` key) and takes the chunk's `Occasion`. It returns `true` only when the user's `when` allows it and the occasion holds (`Invisible` means the tab isn't its window's active tab, which is all Brindle can know of visibility). It stays pure and takes `visible` and `active` as arguments. Rate limits count every shown or replacing notification as a program notification. A query or alive reply isn't a notification, so it isn't limited beyond the event channel's existing side-event bound (`process R9`). Replies to a disabled source are not written, so `osc99 = false` makes Brindle look like a terminal without OSC 99.

### Config

`notifications.osc99: bool`, default `true`, in `config.rs` and `assets/default-config.toml`. The settings dialog has no notifications section, so nothing there changes. Old configs without the key get the default, and an old Brindle ignores the key. Both directions are additive.

### Testing

- `term-core` unit tests: metadata parsing (unknown keys, `-focus`, bad ids, `e=1`), every payload type.
- `kitty.rs` unit tests: assembly across chunks, the caps, the 8-pending eviction, reply texts byte-for-byte (query on both `Caps`, alive, click, close).
- `policy.rs` unit tests: occasion against `when`.
- e2e (`cases/notify.rs`, through `BRINDLE_NOTIFY_LOG`): `notify-osc99-chunks`, `notify-osc99-update` (the log shows a replacement), `notify-osc99-query` (the reply read back by the shell, as the 7501 query case does), `notify-osc99-alive`, `notify-osc99-off`, `notify-osc99-buttons` (the log records the labels, the fifth dropped) and `notify-osc99-silent`.
- `dbus.rs` and `toast.rs` unit tests for the pure parts: the action list, the hints, the toast XML. The log writer records closes and replacements, so the cases can see them without a desktop.
- Manual (needs a real click): `a=report`, `c=1` and button reports on Linux (GNOME and KDE, whose servers show actions differently) and on Windows.

## Risks / Trade-offs

- [The log sink can't click] → clicks and dismissals stay manual checks; their routing is unit-tested on `Notifier`'s pure parts (target lookup, which report to write).
- [Some notification servers hide actions or collapse them into a menu (GNOME shows at most a few; some minimal servers none)] → the `actions` capability decides whether buttons are sent and claimed in the query reply; a server that advertises actions but hides them leaves the program waiting, which is why programs should keep the tab as a fallback, and README says so.
- [Program-chosen ids collide across terminals] → ids are keyed by `(terminal, id)`.
- [A program polls `p=alive` in a loop] → each reply is a small PTY write; the event channel's side-event bound drops floods.
