# Proposal

## Why

Long-running programs in a terminal (builds, deployments, coding agents) alternate between working, waiting on the user and finishing, while the user is looking elsewhere. Today Brindle learns of that only through bells and one-shot OSC 9/777 notifications, which say nothing about what is true now: a tab can't show that the program in it is waiting for permission, and a notification that went by can't be told apart from one whose program has since recovered.

The Program Status Protocol (OSC 7501, revision 0.1, [specification](https://gist.github.com/mitchellh/7acae3abd8355c1c00287d67e96c913a)) lets the program report its state (`idle`, `working`, `done`, `blocked`, `error`) with a kind, a progress percentage, a program name and a message. Claude Code, Codex, Terraform, Homebrew and libghostty have begun to send it. tmux passes the sequence through raw in `%output` (checked on tmux 3.6), so control-mode panes can report too.

## What Changes

- Brindle follows OSC 7501 reports in local tabs and in control-mode panes: it keeps the records the protocol defines per terminal (one per id, replaced whole, cleared with their children, capped at 256), applies the lifetime rules (`working` and `blocked` end at a new shell prompt or when the process exits; `done` and `error` survive until the user types into that terminal), and answers the feature query `OSC 7501 ; ?` so programs know they are heard.
- Each tab shows a dot coloured by the most pressing state of its terminal (or of any pane of its tmux window): blocked, error, done or working. It stays until the record goes away, unlike the bell dot it sits beside.
- A record that becomes `blocked`, `done` or `error` raises a desktop notification under the existing focus rule and rate limits, with the record's `title` or `app` and its `msg`. A new `notifications.osc7501` setting (default `true`) turns these off.
- `--dump-screen-after` prints the active terminal's records as `--- status` lines, and the notification log names the source `osc7501`, so the e2e suite can check both.

## Capabilities

### New Capabilities

- `program-status`: the OSC 7501 reports Brindle recognises, the records it keeps, their lifetimes, and the feature-detection reply.

### Modified Capabilities

- `desktop-notifications`: OSC 7501 state changes notify, and the `osc7501` setting.
- `tabs`: the program status indicator beside the bell dot.
- `command-line`: `--- status` lines in the dump; `osc7501` as a notification-log source.

## Impact

- `src/terminal/status.rs` (new): the report parser and the record set, pure and unit-tested.
- `src/notify/osc.rs`: the scanner also collects OSC 7501, OSC 133 `A` (for the lifetime rule) and `ESC c`.
- `src/terminal/mod.rs`: the `Terminal` holds its records, answers the query, and emits a `Status` event; `scan.rs` and `feed()` forward what the scanner finds.
- `src/tmux_view.rs`, `src/workspace.rs`: the indicator, the notification, and the dump lines.
- `src/config.rs`, `src/notify/policy.rs`, `README.md`, `CLAUDE.md`: the setting and its docs.
- `tests/e2e/`: the `Dump` parser gains `status`, and new cases cover a local tab and a tmux pane.
- Dependencies: none new; `base64` 0.22 is already a dependency.
