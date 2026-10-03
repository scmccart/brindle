# Tasks

> Backfilled: every task here was completed in the 2026-10-03 session
> (commits `a87eb34`, `00bdaf1`, `c227f0b`, `488ca95`).

## 1. Build environment

- [x] 1.1 Rootless toolchain: `.toolchain/bootstrap.sh` unpacks gcc, pkgconf, `-dev` headers and tmux into `.toolchain/root`, and `env.sh` exports the paths. Verified: C and C++ test programs link against xkbcommon, and `tmux -V` prints 3.6.
- [x] 1.2 Crate setup on gpui 0.2.2 and alacritty_terminal 0.26, with `Cargo.lock` seeded from gpui's lockfile. Verified: `cargo build` succeeds; without the seed, `xattr` fails to compile.

## 2. Terminal core (terminal-emulation)

- [x] 2.1 PTY terminal model: Brindle environment, an event channel into GPUI, resize, scrollback, selection, and an error message tab when launch fails. Verified: `--send 'echo hello-$((6*7))\r' --dump-screen-after` shows `hello-42`.
- [x] 2.2 xterm key encoding (`terminal/keys.rs`), with plain text routed through the IME path. Verified: unit tests for control codes, alt-as-meta, application cursor mode, modified keys and function keys.
- [x] 2.3 Mouse reporting in SGR, UTF-8 and X10 encodings; alternate scroll; focus reports (`terminal/mouse.rs`). Verified: unit tests.
- [x] 2.4 Bracketed paste and OSC 52 clipboard. Verified: unit tests for paste handling; alacritty gates OSC 52 reads by config.

## 3. Rendering (terminal-rendering)

- [x] 3.1 GPUI element with batched text shaping, merged backgrounds, cursor shapes, selection and IME pre-edit. Verified: screenshot shows correct colors, bold, underline, backgrounds, CJK and emoji.
- [x] 3.2 Procedural box-drawing and block elements. Verified: unit tests, and a screenshot showing joined `┌─┬─┐`, `│` and `▄▀`.
- [x] 3.3 Font family resolution, glyph fallbacks, ligatures off, font zoom. Verified: the log shows the resolved family; zoom works.

## 4. Window, tabs and profiles (tabs, profiles)

- [x] 4.1 Workspace with tabs, a client-side-decorated tab strip as the title bar, window controls and resize edges. Verified: screenshot; resize-edge unit test.
- [x] 4.2 Tab actions: new, close, cycle, jump, move, drag-to-reorder. Bell indicator. Cached titles from OSC title, foreground process or profile. New tabs inherit the working directory. Verified: run manually and confirmed by the user.
- [x] 4.3 Profiles, profile picker with fuzzy filter, ctrl-alt-N shortcuts, classic (`plain`) tmux profiles. Verified: fuzzy-match unit test; classic tmux screenshot with prefix splits and joined borders.

## 5. Configuration (configuration)

- [x] 5.1 TOML config at the XDG path or `$BRINDLE_CONFIG`, with a commented default written on first run and clamping. Invalid config falls back to defaults and shows a banner. Verified: config unit tests, including that the default file parses.
- [x] 5.2 Built-in and custom themes with inheritance. Verified: theme unit tests.
- [x] 5.3 Single action table, default bindings, user overrides and `none`. Verified: action-name unit test.
- [x] 5.4 Live reload on file change, applied to tabs, tmux panes and sessions. Verified: an e2e run switched the theme to solarized-dark live while attached.

## 6. tmux control mode (tmux-control-mode)

- [x] 6.1 Protocol spike against tmux 3.6 over pipes; transcript captured. Verified: transcript shows framing, octal-escaped `%output`, layouts and `%exit`.
- [x] 6.2 Line parser, response collector and layout parser. Verified: unit tests with fixtures from the spike, including an `%end` line inside a capture and nested layouts.
- [x] 6.3 Session model: attach, `list-windows` reconciliation, panes as remote terminals, `send-keys -H` input, `refresh-client -C` sizing, `Failed` and `Detached` events. Verified: e2e run where the window appears as a tab and input round-trips.
- [x] 6.4 Pane restore from `display-message` and `capture-pane`, including mode flags (`mouse_all_flag` for 1003). Verified: `restore_bytes` unit tests; e2e reattach restored vi on the alternate screen with app-cursor and keypad modes, and a 1002+1006 pane came back as exactly that.
- [x] 6.5 Window view with native panes, dividers, active-pane highlight, zoom indicator, and focus following the active pane. Verified: divider-segment unit test; screenshots of the split and zoomed states.
- [x] 6.6 Native bindings for new window, split, close pane, zoom, pane focus and detach. When detaching from the only tabs, open a shell tab first. Verified: e2e run with `--action` steps; the detach fallback leaves a shell tab and the session survives.

## 7. CLI, docs and quality

- [x] 7.1 CLI options (`-e`, `-p`, `-d`, `--config`, `--list-actions`, `--list-themes`) and debug hooks (`--send`, `--action`, `--dump-screen-after`). Verified: used by every e2e run.
- [x] 7.2 README (build, rootless toolchain, keys, control-mode prefix limitation), LICENSE-MIT and LICENSE-APACHE, release profile (stripped, thin LTO). Verified: release binary is 21 MB.
- [x] 7.3 `/simplify` review pass: reuse, simplification, efficiency and altitude cleanups. Verified: clean build with no warnings, 49 tests passing, e2e scenarios re-run.

## 8. Integration checks

- [x] 8.1 Control-mode e2e on a throwaway server (`-L brindle-e2e`): split, zoom, new window, rename, detach, reattach and restore, theme reload, failure tab. Verified with screenshots and screen dumps.
- [x] 8.2 Manual keyboard, wheel and classic tmux checks on Wayland. Verified: confirmed by the user ("Everything looks to be working beautifully").
