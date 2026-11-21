# Proposal

## Why

OSC 9 and OSC 777 notifications are fire-and-forget. A program can't update a notice it already raised (a build going from "running" to "failed"), close one that no longer applies, learn that the user clicked it, or ask whether the terminal supports any of this. kitty's OSC 99 protocol does all of that. kitty, Ghostty, foot and WezTerm support some or all of it, and a growing set of tools emit it, among them notify-send replacements, editors and agent CLIs. Brindle already has the delivery side (freedesktop and toasts, click to raise, rate limits), so the protocol is mostly parsing and bookkeeping on top of it.

## What Changes

- Recognise `ESC ] 99 ; <metadata> ; <payload> ST` in every terminal: title and body payloads, chunked with `d=0`, plain or base64 (`e=1`), identified by `i=`.
- Show a complete notification through the existing sinks. A notification with an id that is still open replaces it.
- Support `p=close` (close by id), `p=alive` (list open ids), `p=?` (the capability query and its reply), `a=focus`/`-focus`/`report` (what a click does, and a click report written back to the program), `c=1` (a close report), `o=always|unfocused|invisible` (which can only make a notice quieter than the user's `when`), and on Linux `u=` (urgency) and `w=` (timeout).
- Support buttons (`p=buttons`, up to 4): shown as freedesktop actions where the server shows them, and as toast buttons on Windows. A press reports the button's number back to the program without raising the window, so a program can ask "Allow / Deny?" and take the answer from the notification.
- Support `s=silent`. Every other sound plays the platform's default.
- Ignore icons, types and application names (`p=icon`, `g`, `n`, `t`, `f`). The query reply doesn't claim them.
- Add `notifications.osc99` (default `true`), apply the existing program rate limits to OSC 99, and log it as `osc99` in `BRINDLE_NOTIFY_LOG`.

Out of scope: icons, and named sounds.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `desktop-notifications`: adds OSC 99 recognition, assembly, updates, closing, queries, click and close reports, buttons, silence, and the `osc99` setting.

## Impact

- `term-core/src/osc.rs` (`99` joins the wanted numbers; a `Found::Kitty` with the parsed metadata and payload), `term-core/src/notice.rs` (`Source::Osc99`).
- A new pure `src/notify/kitty.rs` (per-terminal assembly and the reply texts), `src/notify/mod.rs` (`Request` gains an optional program id and flags; `Notifier` tracks open OSC 99 notices per terminal and routes clicks and closes back), `src/notify/dbus.rs` and `src/notify/toast.rs` (close by id, urgency and timeout hints, dismissal reasons), `src/terminal/mod.rs`, `src/config.rs`, `assets/default-config.toml`.
- `tests/e2e/cases/notify.rs`, `docs/code-map.md`, README.md. No wire change, no new dependency.
