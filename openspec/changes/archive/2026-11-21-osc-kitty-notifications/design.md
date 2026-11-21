# Design

## Context

Notifications today flow as `osc::Scanner` (`brindle-term-core`) → `Found::Notice` → `Terminal::handle_found` → `TerminalEvent::Notify` → `Workspace::on_attention` (focus rule, settings) → `Notifier::send` (a GPUI global: rate limits in `policy.rs`, then a sink command). The sinks are `notify/dbus.rs` (zbus worker) and `notify/toast.rs` (WinRT), plus the `BRINDLE_NOTIFY_LOG` writer. Sinks answer with `Reply::{Shown, Action, Token, Closed}`, and `Notifier` maps a sink id to a `Target { window, tab, source }` for clicks. Bell notifications already use `replaces`. Program replies to the PTY go through `Terminal::write`, as the OSC 7501 query reply does. The server core's `Emulator` ignores notices, and this change keeps it that way.

kitty's protocol reference: <https://sw.kovidgoyal.net/kitty/desktop-notifications/>.

### Checked against kitty's source

The brief for this design came from a summary of that page, so the implementation was checked against `kitty/notifications.py` and `docs/desktop-notifications.rst` (kitty master, November 2026). What it showed, and what changed here:
- **Reply bytes.** kitty writes `99;i=<id>:p=?;<caps>` and the other replies with no spaces (the spaces in the doc's forms are layout), and joins the query's keys in the order `a`, `o`, `u`, `p`, `c`, `w`, `s`. Brindle writes the same bytes in the same order. A query, click or close report for a notification without an id uses `i=0`; `p=alive` and `p=close` without an id are no-ops.
- **Buttons.** Labels are split on U+2028, as the brief said. kitty keeps up to 8 of 256 characters; Brindle keeps its tighter 4 × 64 (toasts allow 5 actions, and servers show few). A button press is an activation with the button's number, and kitty writes it **only with `a=report`** ("the terminal must not send a response unless report is requested"), so Brindle does too. kitty also focuses the window on a button press when `focus` is on; Brindle keeps its rule that a button never raises, since a button answers the notification in place.
- **`s`** is base64, as every free-form metadata value (`f`, `n`, `t`) is. A value that doesn't decode is ignored, leaving the default sound.
- **Base64 payloads** go through a streaming decoder per payload buffer, so a program may chunk before encoding (each chunk padded) or after (cut anywhere, the last chunk padded or not). Decoding therefore moves from `term-core` to the assembly, which carries an unfinished quad between chunks.
- **Metadata merges across chunks.** kitty folds each chunk's metadata into the pending notification, a later explicit value winning; Brindle does the same (each key is an `Option` on the chunk). kitty unions `a` across chunks; Brindle lets the latest chunk that names an action decide it, which is the same for every program that sends `a` once.
- **No title** means the body is the title; neither means nothing is shown.
- **Close reports.** kitty writes `p=close` for every close the server reports, its own `CloseNotification` included, so a program's `p=close` of a `c=1` notification gets a report. Brindle writes it too (from `Notifier`, at the `p=close`, so it doesn't depend on the server's signal), and the spec says so instead of excepting the program's own close. A replaced notification isn't closed.
- **An update before the first is shown.** kitty sends the update as a new notification and closes the old one once the server has created it. `Notifier` does the same: it has no server id to replace yet.
- **Capabilities** before the server has been asked: kitty assumes buttons until `GetCapabilities` says otherwise, and Brindle does too, since the query must be answered at once (before a following DA1's answer).
- **`o=invisible`** in kitty is "unfocused and not visible", visible being the active tab of a focused (or, on macOS, unoccluded) OS window. Brindle keeps its reading, "not its window's active tab", the quieter of the two where it can't know whether an unfocused window is on screen.
- **Payload size.** kitty limits the payload, not the whole OSC, to 4096 bytes, so the scanner collects up to 4096 bytes of payload plus 1024 of metadata for OSC 99.

## Goals / Non-Goals

**Goals:**
- Parsing and assembly as pure code with unit tests. Entities and sinks only route values (`fcis R1`).
- Every reply Brindle writes is built from validated ids and fixed tokens, so a program can't make Brindle echo arbitrary bytes.

**Non-Goals:**
- Icons, named sounds. Notification history.
- Keeping incomplete chunks across a reconnect. The server doesn't track notices, so a chunk split across a reconnect is lost, and a program resends on the next notice.

## Decisions

### Parse in `brindle-term-core`, assemble in `src/notify/kitty.rs`

`osc.rs` gains `(99, 2)` and `Found::Kitty(kitty::Chunk)`, parsed by a new pure `term-core/src/kitty.rs`. Parsing lives in `term-core` because the scanner returns it, but the server core drops it, as it drops `Notice`.

```rust
// term-core (pure)
pub struct KittyId(String);                 // [A-Za-z0-9_+.-]{1,64}
pub enum Payload { Title, Body, Buttons, Close, Query, Alive, Unsupported }
pub enum Data { Plain(Vec<u8>), Base64(Vec<u8>) }   // base64: its alphabet only
pub enum Sound { Default, Silent }          // `s` decoded; anything but `silent` is Default
pub enum Occasion { Always, Unfocused, Invisible }
pub enum Urgency { Low, Normal, Critical }
pub enum Expiry { Default, Never, After(Duration) }
pub struct Meta { focus, report, close_report: Option<bool>, occasion: Option<Occasion>,
                  urgency: Option<Urgency>, expiry: Option<Expiry>, sound: Option<Sound> }
pub struct Chunk { id: Option<KittyId>, payload: Payload, done: bool, data: Data, meta: Meta }
```

A bad id, a malformed metadata part or a payload outside the base64 alphabet under `e=1` is `None` (`railway R5`). `Meta`'s fields are `None` where the chunk doesn't name them, so a later chunk's value replaces an earlier one's.

`src/notify/kitty.rs` holds the per-terminal `Assembly` (at most 8 pending, oldest dropped, 2048-byte caps). It decodes base64 as a stream (an unfinished quad carried to the next chunk of the same payload), keeps the bytes until the notification completes, then reads them as UTF-8 (lossily) and removes controls with the existing `clean`. It turns chunks into an outcome, matched exhaustively by `Terminal`:

```rust
pub enum Step { Pending, Show(Notice99), Close(KittyId), Query(Option<KittyId>), Alive(KittyId), Ignored }
pub fn reply_query(id: Option<&KittyId>, caps: Caps) -> String;
pub fn reply_alive(id: &KittyId, open: &[KittyId]) -> String;
pub fn click_report(id: Option<&KittyId>, button: Option<Button>) -> String;
pub fn close_report(id: Option<&KittyId>) -> String;
```

`Caps` is a value, not a `cfg`: `Notifier` builds it from the platform's sink (`hints: bool` is true for dbus, false for toast; `buttons: bool` is the dbus server's `actions` capability, read in `dbus.rs` and sent back as `Reply::Capabilities`, true until then, and true for toast). The log sink stands in for its platform's sink and answers with that sink's caps. So the reply's platform difference sits in the sink seam, not in `kitty.rs` (`ports R4`). `Assembly` is GPUI-free and lives in the `Terminal`. Each terminal owns its pending notifications, and ids are per terminal.

Alternative considered: assembling in `term-core`. Rejected: the core would carry state it never uses, and the rate limits and focus rule that decide the outcome are client-side.

### Notifier tracks open OSC 99 notices

`Request` gains `program: Option<ProgramNotice { terminal: WeakEntity<Terminal>, id: Option<KittyId>, focus, report, close_report, urgency, expiry, buttons, sound }>`. A pure `kitty::Open<K>` table (`K` is the terminal's `EntityId`; generic so it is tested without GPUI) maps `(terminal, id)` to the notification's sink id, or to the key of the command still waiting for one, and remembers which sent keys are to be closed once shown. It answers replacement (`Replace::Id(n)`, `Replace::Nothing`, or "send as new and close the old key when shown"), `p=close` (close this sink id now, or close that key once shown), `p=alive` (the open ids, oldest first) and the server's closes. `Target` gains the program part, so:
- `Reply::Action` with `"default"` → focus (unless `focus` is off) and, with `report`, the click report; with `"1"`–`"4"` → with `report`, the button report, never a raise;
- `Reply::Closed` → drop from the table, and with `close_report`, the close report;
- the program's `p=close` → the sink's `Close`, the table and the target dropped (so the server's later signal finds nothing), and with `close_report`, the close report at once.

`Terminal` sends shows, closes and alives on as one event, `TerminalEvent::Notify99` (`Request99::{Show, Close, Alive}`), through the workspace (which applies the focus rule to shows) to `Notifier`. GPUI delivers emitted events after the update that emitted them, so a close or an alive called from `Terminal` directly would reach `Notifier` before a show in the same read; one event kind keeps them in stream order. Only the query is answered in `Terminal` at once, since it needs no table and its reply must come before the answer to a following request (kitty's DA1 check).

Reports are written through the `WeakEntity<Terminal>`. One that no longer upgrades is skipped: the terminal closed, so nothing is written (`railway R6`).

Sink changes:
- dbus: `Command::Notify` gains `urgency` and `expire_ms` (the hint and `expire_timeout`, still `1` and `-1` for other sources) and a new `Command::Close { id }` (`CloseNotification`). `connect` sends `Reply::Capabilities { buttons }`.
- toast: `Close` removes the toast by tag through `ToastNotificationManager::History`. Only a `UserCanceled` dismissal counts as a close, as today: a toast that timed out is still in Action Center and can still be clicked, and `ApplicationHidden` is Brindle's own close. An activated toast leaves Action Center, so the sink reports it closed after the click. Urgency and expiry are dropped.

### Buttons and silence

`Assembly` keeps `buttons: Vec<Label>` per pending notification (split on U+2028, empty labels dropped, at most 4, 64 characters each: a `Label` is built only by that check). A completed update carries its own buttons, so an update without `p=buttons` has none, as the spec says.

- **dbus:** `Command::Notify` gains `buttons: Vec<String>` and `silent: bool`. Actions become `["default", "Show", "1", label1, "2", label2, …]`, sent only when the server advertises `actions`; `silent` adds the `suppress-sound` hint. `Reply::Action { id, action }` already carries the key, so `Notifier` maps `"default"` to the click path and `"1"`–`"4"` to a button report.
- **toast:** the XML gains `<actions>` with one `<action content="…" arguments="button=N" activationType="foreground"/>` per button (toasts allow 5), and `<audio silent="true"/>` when silent. The `Activated` handler reads `ToastActivatedEventArgs::Arguments` (today it ignores its arguments), giving `Reply::Action` the key `"N"` for a button or `"default"` for the body. The toast XML builder becomes a pure function (`toast::xml(summary, body, buttons, silent)`), unit-tested on Linux for escaping labels: `toast.rs` is now built on every platform, with its worker behind `cfg(windows)`.
- **Button press:** `Notifier` writes `click_report(id, Some(n))` (`ESC ] 99;i=<id>;<n> ESC \`) through the `WeakEntity<Terminal>`, only with `report` (kitty's rule). A button press never raises the window; `focus` applies only to the body click.

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
- `dbus.rs` and `toast.rs` unit tests for the pure parts: the action list, the hints, the toast XML. The log writer records closes and replacements, so the cases can see them without a desktop: an OSC 99 line is `osc99<TAB>title<TAB>body`, then tab-separated flags in a fixed order (`i=<id>`, `replaces`, `buttons=<a>|<b>`, `silent`, `urgency=low|critical`, `expiry=never|<ms>`, each only when it applies), and a program's close is `close<TAB>osc99<TAB>i=<id>`.
- Manual (needs a real click): `a=report`, `c=1` and button reports on Linux (GNOME and KDE, whose servers show actions differently) and on Windows.

## Risks / Trade-offs

- [The log sink can't click] → clicks and dismissals stay manual checks; their routing is unit-tested on `Notifier`'s pure parts (target lookup, which report to write).
- [Some notification servers hide actions or collapse them into a menu (GNOME shows at most a few; some minimal servers none)] → the `actions` capability decides whether buttons are sent and claimed in the query reply; a server that advertises actions but hides them leaves the program waiting, which is why programs should keep the tab as a fallback, and README says so.
- [Program-chosen ids collide across terminals] → ids are keyed by `(terminal, id)`.
- [A program polls `p=alive` in a loop] → each reply is a small PTY write; the event channel's side-event bound drops floods.
