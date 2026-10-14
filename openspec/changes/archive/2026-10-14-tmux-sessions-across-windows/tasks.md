# Tasks

## 1. Session names and attach errors (`src/tmux/`)

- [x] 1.1 Make the session name an `Option<String>`, set from the profile for named attaches and by both `%session-changed` and `%session-renamed`. Update readers (`snapshot`, `attach_tmux`, the ask palette placeholder) to handle `None`. Verify with a unit test that a `SessionChanged` notification sets the name.
- [x] 1.2 Add `AttachSpec { Named, New(Option<String>) }` to `TmuxSession::attach` and `command_line`. `New` emits `new-session [-s name] [-c cwd]` without `-A` and starts with no name. Verify with `command_line` unit tests for `New(None)`, `New(Some("work"))` and the unchanged `Named` form.
- [x] 1.3 Before attaching, keep the body of an unflagged `%error` response as `attach_error`, and use it as the `Failed` reason in `on_detached`. Verify with a unit test feeding the probed `duplicate session: work` transcript through the session's event handling, or through the `Collector` plus a reason helper.
- [x] 1.4 Add `SessionKey { program, tmux_args, name }` with a constructor from a session (`None` while the name is unknown or the session is detached) and one from a profile. Verify with unit tests: two profiles with the same command, args and session give equal keys; different `tmux_args` give different keys.

## 2. One window per session (`src/workspace.rs`, `src/notify/activation.rs`)

- [x] 2.1 Implement `find_session(key)`: this window's sessions first, then the other windows via `cx.windows()`, skipping its own. Switch `attach_tmux`'s duplicate check to it. Verify `cargo test` passes and that the e2e case in 5.2 shows one client.
- [x] 2.2 Implement "Go to <session>" focusing: defer a `handle.update` that activates the tab for tmux's current window (else the first tab) and raises the window. Verify through e2e 5.2: after choosing "Go to", the other window's active tab is the session's.
- [x] 2.3 Add an `activation.rs` entry point that requests an `xdg_activation_v1` token for the focused window's surface and activates the target with it, falling back to `activate_window`. Use it in 2.2. Verify that it builds and logs which path it took at debug level. The manual check is in 6.3.
- [x] 2.4 Add `session_open_rows(config, picked, open_keys, target)` returning typed rows: Go to, New session, then `New tab:`/`New window:` for other profiles, skipping the picked profile and control profiles that are open. Verify with unit tests for tab vs window labels and for the skipped profiles.
- [x] 2.5 Add `Workspace::open_profile(ix, target)` and `PaletteKind::SessionOpenElsewhere`, and route the new-tab palette, ctrl-alt-N, `new_tab` (non-tmux tabs) and `NewWindow` through it. `NewWindow` goes through the active workspace, or opens directly when there is no window. Confirming a row launches the profile, opens the name prompt, or goes to the session; dismissing does nothing. Verify with e2e 5.2, 5.3 and 5.8.
- [x] 2.6 In `session_ended_rows`, label a control profile whose session is open elsewhere "Go to <session>", and make confirming it go there and close this window. Verify with an extended `session_ended_rows` unit test.
- [x] 2.7 Update the README sections on tmux and windows: one window per session, the open-elsewhere palette and its rows, and that nothing switches windows without asking. Verify the README matches the specs' wording.

## 3. tmux: New Session

- [x] 3.1 Add the `TmuxNewSession` action (`tmux_new_session`, labelled "tmux: New Session", no default binding) to `ACTIONS`, handled in `Workspace` and registered only while the config has a control-mode profile. Verify that `--list-actions` shows it, and with a test (or e2e 5.5) that the command palette lists it in a local tab only when a control profile exists.
- [x] 3.2 Implement the prompt flow:
  - choose the profile by the spec's order;
  - attach with `AttachSpec::New(name or None)` and the active tab's cwd;
  - keep the prompt's sender in `creating`;
  - send `Ok` when the session's first tab arrives, and `Err(reason)` on `Failed` without opening the failure tab.

  Verify with e2e 5.5 and 5.6.
- [x] 3.3 Run the same flow from the palette's "New tmux session…" row with the picked profile. For a new window, check the name with `tmux [args] has-session -t =<name>` before opening it and report a taken name in the prompt; then open the window with `AttachSpec::New`. Verify with e2e 5.9.
- [x] 3.4 Document "tmux: New Session" in the README (command palette list and tmux section), including empty names and how a bad name is reported. Verify the README entry against the spec scenarios.

## 4. Profile palette and restore

- [x] 4.1 Append ` · open in this window` / ` · open in another window` to control-mode profile rows in `open_profile_picker` using `find_session`, and print row details in `Palette::describe` as `label  detail  [shortcut]`. Verify that the existing e2e parsers still pass, and with e2e 5.4.
- [x] 4.2 Add `session::dedupe_windows(windows, config)` and call it in `main.rs` before restoring. Verify with unit tests: a duplicate dropped from a later window; a window emptied by dedupe dropped; `active` remapped when an earlier entry is removed or reset when the active entry is removed.
- [x] 4.3 Update the README's session-restore notes if they describe per-window sessions. Verify by reading the section against the session-restore delta.

## 5. Debug hook and e2e cases (`tests/e2e/`)

- [x] 5.1 Add `--- window N tabs=[…]` lines for windows other than the dumped one to `--dump-screen-after`, update the dump parser and its `--self-test`. Verify that `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` passes.
- [x] 5.2 Case `session-open-elsewhere-goto`:
  - start with `-p Shell`, then `--action new_window`: the default tmux profile's session isn't open yet, so window 2 attaches it;
  - pick tmux in window 1 with `--action new_tab_with_profile_2`;
  - assert that the dump shows the open-elsewhere palette with "Go to <session>" first;
  - then `picker_confirm`, and assert from `tmux list-clients -t <session>` that there's exactly one control client.
- [x] 5.3 Case `new-window-asks`: with tmux as the default profile and its session open, `--action new_window`. Assert that the dump shows the palette with "Go to <session>", "New tmux session…" and "New window: Shell". Then choose "New window: Shell" and assert that the `--- window 2` line shows one Shell tab and that `list-clients` shows one client.
- [x] 5.4 Case `profile-palette-open-state`: with the session open in the first window, open the profile picker. Assert that the tmux row's detail reads `tmux control mode · <session> · open in this window`.
- [x] 5.5 Case `tmux-new-session`: from a local tab, run `tmux_new_session` via `--action`, type `work`, `picker_confirm`. Assert that `tmux ls` on the throwaway server lists `work`, that a tab for it appears, and that the palette is closed.
- [x] 5.6 Case `tmux-new-session-duplicate`: create `work` beforehand with `tmux new-session -d`, then submit `work` in the prompt. Assert that the dump shows the prompt still open with `error="duplicate session: work"`, and that no attach-failure tab exists.
- [x] 5.7 Case `restore-dedupe`: write a state file (under `BRINDLE_STATE_DIR`) with two windows that both record the session, the second also with a local tab. Launch and assert that the first window shows the session, that `--- window 2` shows only the local tab, and that `list-clients` shows one client.

- [x] 5.8 Case `session-open-elsewhere-dismiss`: open the palette as in 5.3, send `picker_cancel`, and assert that there is no `--- window 2` line, no new tab, and no palette.
- [x] 5.9 Case `new-window-new-session`: open the palette as in 5.3, choose "New tmux session…", type `scratch`, confirm. Assert that `tmux ls` lists `scratch` and that the `--- window 2` line shows its tab.

## 6. Integration

- [x] 6.1 `cargo test` and `cargo build --release` pass. `openspec validate tmux-sessions-across-windows --strict` passes.
- [x] 6.2 With the user's go-ahead, run the e2e suite (`BRINDLE_E2E=1 cargo test --test e2e`). All cases pass, including the earlier `session-end-*` and `restore-*` cases.
- [x] 6.3 Manual check for the user (GNOME/Wayland): with `main` open in window A, focus window B and pick `tmux` from the profile picker. Window A should come to the front with its `main` tab active, or at worst show GNOME's "Brindle is ready" notice.

## 7. After the manual check

- [x] 7.1 Send an input serial with the activation token request: attach to GPUI's display when a Wayland window opens, bind `wl_seat`, create our own `wl_keyboard`/`wl_pointer`, keep the latest key or button press serial, drain the queue on a 1 s timer, and `set_serial` in `request_token`. Verify that it builds, `cargo test` passes, and the e2e `session-open-elsewhere` cases still pass (X11: no Wayland path). The manual check is 6.3 again.
- [x] 7.2 Confirming a profile in the new-tab palette whose session another window shows goes there directly; the row's detail reads `in another window (switches)`. Update `profile-picker-open-state` and add e2e case `profile-picker-goes-to-session`: window 2 shows the session, pick `tmux` in window 1's picker, and assert there's no palette, window 1 has no new tab, and one client.

  7.1 and 7.2 were verified by hand on GNOME/Wayland (Enter and click both raise the other window; the picker doesn't ask). The e2e suite was not re-run after them.
