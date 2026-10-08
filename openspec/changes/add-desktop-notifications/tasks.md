# Tasks

## 1. Dependencies

- [ ] 1.1 Add `zbus` (5.12, no default features, `async-io`), `raw-window-handle` 0.6, `wayland-client` 0.31, `wayland-backend` 0.3 (`client_system`, `dlopen`) and `wayland-protocols` 0.31.2 (`client`, `staging`) to `Cargo.toml`; verify `cargo build` succeeds and `git diff Cargo.lock` only adds lines to Brindle's own dependency list

## 2. OSC scanner and policy (pure)

- [ ] 2.1 Add `src/notify/osc.rs`: the streaming scanner and OSC 9/777 payload parsing with text clean-up; verify unit tests for BEL/ST terminators, split reads at every byte, CAN/SUB/ESC cancellation, other OSCs ignored, oversize payloads, ConEmu sub-commands 1–12 vs `9;42`, OSC 777 `notify` with `;` in the body, other 777 sub-commands, control-character stripping and truncation
- [ ] 2.2 Add `src/notify/policy.rs`: the focus rule, the notification title/body layout, and the token-bucket rate limiter with an injected clock; verify unit tests for visible/background/unfocused × `unfocused`/`always`, per-source switches, bell storm (one per 5 s), OSC burst of 3 then 1 per 2 s, the global cap, and no charge when denied

## 3. Configuration

- [ ] 3.1 Add the `[notifications]` table (`bell`, `osc9`, `osc777`, `when`) to `Config` and a commented section to `assets/default-config.toml`; verify `default_config_file_parses` and a new parse test for the table, its defaults and an invalid `when`

## 4. Detection in terminals

- [ ] 4.1 Carry notices on the terminal's event channel and add `TerminalEvent::Notify`; wrap the PTY in `ScanPty` so alacritty's reader scans output; verify `cargo build` and a unit test that `ScanReader` reports a notice split across two reads from a pipe
- [ ] 4.2 Scan bytes in `Terminal::feed` for remote terminals and emit `TerminalEvent::Notify`; verify with a unit test of the remote scan path (pure helper)

## 5. Notification service

- [ ] 5.1 Add `src/notify/mod.rs`: the `Notifier` global (rate limits, click map, bell replacement ids) with the file sink for `BRINDLE_NOTIFY_LOG` (synchronous line append) and a command channel to a background worker; install it in `main.rs`; verify `cargo build` and a unit test of the log line format
- [ ] 5.2 Add `src/notify/dbus.rs`: the zbus worker (lazy connect with retry back-off, `GetCapabilities`, `Notify` with actions/markup escaping/hints/`replaces_id`, `CloseNotification`, and `ActionInvoked`/`ActivationToken`/`NotificationClosed` signals back to the UI thread); verify by building and a short manual run that sends one real notification to the session bus
- [ ] 5.3 Add `src/notify/activation.rs`: raise a window with an xdg-activation token on Wayland, falling back to `activate_window()`; verify it builds and that the X11 path falls back (logged)

## 6. Workspace and tmux wiring

- [ ] 6.1 Make `TmuxWindowView` forward `Bell` and `Notify` from its pane terminals, and have `Workspace` handle both events for local and tmux tabs in one place (bell dot, focus rule, settings, title layout, send); verify by building and the e2e cases in 7
- [ ] 6.2 Handle a notification click: activate the tab by id in its window and raise the window with the token; verify by hand on GNOME (morning checklist) and by building

## 7. End-to-end cases

- [ ] 7.1 Add a headless `tmux-osc-in-output` case: a raw control client sees OSC 777, OSC 9 and BEL from a pane in a non-current window in `%output`; verify `BRINDLE_E2E=1 cargo test --test e2e -- tmux-osc` passes
- [ ] 7.2 Let `Run` pick the profile and append config text; add `notify-osc-local` (Shell profile, `when = "always"`: OSC 777, OSC 9 notify, `9;4` and `9;9` don't) and `notify-tmux-background` (two tmux windows: a bell storm and an OSC 777 in the background window give one bell line and one osc777 line naming the window); verify `BRINDLE_E2E=1 cargo test --test e2e -- notify` passes under `flock /tmp/brindle-gui.lock`

## 8. Documentation and integration

- [ ] 8.1 Document notifications in README.md (config, examples, tmux behaviour) and CLAUDE.md (architecture notes, `BRINDLE_NOTIFY_LOG`); verify the README commands match the default config
- [ ] 8.2 Run `cargo test`, `cargo build --release`, the e2e `notify`/`tmux-` cases and the full e2e suite; verify everything passes
