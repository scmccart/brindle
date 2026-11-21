# Tasks

## 1. Shared types and parsing (`brindle-term-core`)

- [x] 1.1 Check against kitty's source: the reply separators, the U+2028 button separator, whether a button press needs `a=report`, and `s`'s base64 encoding; update this change's spec where kitty differs
- [x] 1.2 Add `Source::Osc99` (name `osc99`) to `notice.rs` and extend `each_source_has_its_own_name`
- [x] 1.3 Add the OSC 99 chunk parser (`KittyId`, `Payload` with `Buttons`, `Sound`, `Occasion`, `Urgency`, `Expiry`, `Chunk`) and `(99, 2)` to the scanner's wanted numbers, returning `Found::Kitty`; make `Emulator::feed` ignore it; update the fuzz target; verify unit tests for metadata, ids, `e=1`, payload types and split reads with `cargo test -p brindle-term-core`

## 2. Pure client models (`src/notify/`)

- [x] 2.1 Add `src/notify/kitty.rs` (`Assembly` with buttons, `Step`, `Caps` with `buttons`, the reply builders including the button report) and verify unit tests for assembly, button limits, an update dropping buttons, caps, eviction and byte-exact replies with `cargo test -p brindle notify::kitty`
- [x] 2.2 Extend `policy::wanted` with `Source::Osc99` and `Occasion`, and verify unit tests for each occasion under both `when` values
- [x] 2.3 Add `notifications.osc99` to `config.rs` and `assets/default-config.toml`, and verify the config tests cover its default and `false`

## 3. Shell

- [x] 3.1 Keep an `Assembly` per `Terminal`, handle `Found::Kitty` in `handle_found` (show, close, reply), and route `Step::Show` through `TerminalEvent::Notify` with the program fields; verify `cargo test -p brindle terminal`
- [x] 3.2 Extend `Request`, `Target` and `Notifier` with `ProgramNotice` and `open99`: replacement, `p=close`, `p=alive`, click and close reports through `WeakEntity<Terminal>`; unit-test the pure lookups
- [x] 3.3 dbus sink: urgency, expiry, button actions and `suppress-sound` on `Notify`, a `Close` command; toast sink: a pure `toast::xml` with `<actions>` and silent audio, button arguments read in `Activated`, `Close` by tag and dismissal reasons; `Notifier` maps action keys `1`–`4` to button reports without raising; the log writer records buttons, `silent`, `close` and replacement lines; unit-test the action list, hints and XML; verify `cargo clippy --locked --workspace --all-targets -- -D warnings` on Linux and the Windows CI job

## 4. End to end

- [x] 4.1 Add `notify-osc99-chunks`, `notify-osc99-update`, `notify-osc99-query`, `notify-osc99-alive`, `notify-osc99-off`, `notify-osc99-buttons` and `notify-osc99-silent` to `tests/e2e/cases/notify.rs`, and verify `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- notify-osc99`
- [ ] 4.2 Manual checks for the user (real clicks): `a=report` writes the click report, `a=-focus` doesn't raise the window, `c=1` reports a dismissal, and pressing a button reports its number without raising the window, on Linux (GNOME and KDE) and on Windows

## 5. Docs

- [x] 5.1 Update `docs/code-map.md` (`notify/`), README.md (supported OSC 99 subset, the `osc99` key), and CLAUDE.md's `BRINDLE_NOTIFY_LOG` note if its line format gains close records; verify `cargo test --workspace`

## Workflow follow-up

- Archive the change on the implementation PR once its checks pass.
