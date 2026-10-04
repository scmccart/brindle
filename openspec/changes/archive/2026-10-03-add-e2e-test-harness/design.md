# Design

## Context

- Brindle is a binary-only crate, so integration tests can't call its code. They can run the built binary, which Cargo exposes to integration tests as `CARGO_BIN_EXE_brindle`.
- **Debug hooks:**
  - `--send` and `--action` steps run one second apart;
  - `--dump-screen-after N` prints the tab titles, a `--- screen COLSxROWS mode …` header and the active terminal's text, then quits;
  - `BRINDLE_CONFIG` selects the config file.
- **Rules from `CLAUDE.md`:**
  - never inject synthetic input;
  - capture pixels under Xwayland (`env -u WAYLAND_DISPLAY DISPLAY=:0`);
  - use a throwaway tmux server, never the user's;
  - ask before opening windows.
- x11rb 0.13.2 is already in `Cargo.lock` through GPUI.
- The checks being ported are listed in the archived changes for pane geometry, titles, styles and color reports.

## Goals / Non-Goals

**Goals:**
- Every check from this round of tmux work is one named, repeatable case with a pass/fail result.
- `cargo test` without opting in opens no windows and needs no display.
- On failure, a case leaves its evidence behind: the dump, `capture-pane` output, a window capture and a tmux transcript.

**Non-Goals:**
- CI. It would need a virtual X server and software Vulkan; this can be done later.
- Porting earlier changes' checks. That is a follow-up.
- A general GUI testing framework. The helpers serve these cases.

## Decisions

1. **One integration test target with its own runner.** `[[test]] name = "e2e"`, `path = "tests/e2e/main.rs"`, `harness = false`. `main` returns at once, printing that the suite was skipped, unless `BRINDLE_E2E=1` is set.
   - Arguments are case-name filters (substring match), plus `--list`.
   - Cases run one at a time, because they share the display and focus.
   - It prints `ok`/`FAILED` per case and exits non-zero on any failure.

   Run it with `BRINDLE_E2E=1 cargo test --test e2e -- titles`.
   - Rejected alternative: a shell script. Pixel and protocol checks need real parsing, and the capture tool is Rust anyway. One language keeps cases typed and shareable.
   - Rejected alternative: libtest with `#[ignore]`. `--ignored` would also run any other ignored tests, and libtest's parallelism and output capture get in the way of a suite that opens windows.

2. **Binary under test.** The binary under test defaults to `CARGO_BIN_EXE_brindle`, the dev-profile build. Dependencies are built at opt-level 2, so it renders fine. `BRINDLE_E2E_BIN` can point at another binary, such as a release build or the installed one.

3. **Layout of `tests/e2e/`:**
   - `main.rs`: the runner, filtering, reporting, and the case table.
   - `tmux.rs`:
     - `TmuxServer`, a throwaway server on socket `brindle-e2e-<pid>` with `-f /dev/null`, killed on `Drop`, with `run(args)`, `capture(pane)` and `geometry(pane)` helpers;
     - `ControlClient`, a raw `tmux -C` connection that records its transcript, used by the tmux-behaviour cases.
   - `brindle.rs`:
     - `Run`, a builder for one Brindle run: profile, `--send`/`--action` steps, dump time, and a timeline of tmux commands at offsets, run from a thread;
     - writes the test config into a temp dir;
     - returns the parsed `Dump`, which holds the tabs, the size, the modes, the grid geometry and the screen lines.
   - `capture.rs`: finds the window by `_NET_WM_PID` and takes it with `GetImage`. It returns an in-memory RGB image with region and color helpers, and on failure writes PPM files to `target/tmp/e2e/<case>/` (`CARGO_TARGET_TMPDIR`), so no image crate is needed.
   - `query.rs`: writes the OSC 10/11 query script into the temp dir and parses its output from `capture-pane`.
   - `cases/{geometry,colors,titles,styles,tmux}.rs`: the cases.

4. **Expected colors come from the test config, not from Brindle's themes.** The test config defines its own `[themes.e2e]` with distinctive, well-separated colors, and its profiles use it. Cases assert against those constants, so they don't break when a built-in theme is retuned.
   - The config also fixes `padding`, the font size and the cursor (no blink), so captures are stable.

5. **Pixel positions come from the dump.** The dump gains one line for the active terminal, `--- grid origin=X,Y cell=WxH scale=S`, taken from the terminal view's last `GridLayout` and the window's scale factor (the spec delta in this change). For a tmux tab, the active pane's origin minus its `pane_left`/`pane_top` (from tmux) gives the window's cell grid. Pixel helpers can then address cells and rows (title rows, divider columns) instead of fractions of the window.
   - The capture is taken shortly before the dump time. Pixel cases dump late (about 6 s) and capture at about 4.5 s, after all timeline steps have run. Geometry from the dump stays valid because the cases don't resize the window.

6. **Kinds of assertion:**
   - **Text:** the dump's screen equals `capture-pane -p` for the active pane, after trimming trailing blanks, and the dump's size equals tmux's `pane_width`x`pane_height`.
   - **Query:** the parsed `bg=`/`fg=` replies equal the expected `rgb:` strings.
   - **Pixel:** a color is present, or dominant, within a cell region; for example, the title row's text cells contain the title color, and a divider column is the border color.
   - **tmux behaviour:** expected notification lines appear, or don't, in a `ControlClient` transcript within a timeout.

7. **Safety.**
   - The harness only uses Brindle's debug hooks and tmux commands, never synthetic input.
   - It sets `DISPLAY` from `BRINDLE_E2E_DISPLAY` (default `:0`), unsets `WAYLAND_DISPLAY` for Brindle, and refuses to run pixel cases if no X display answers. Text and tmux-behaviour cases still run.
   - `CLAUDE.md` gains the suite's command and a reminder that it opens windows, so the user is asked first.

## Risks / Trade-offs

- **[Timing-based steps (1 s apart) and subscription delays (up to about 1 s) make cases slow, and possibly flaky on a loaded machine]** → Cases wait generously and allow `BRINDLE_E2E_SLOW=<factor>` to stretch every delay. The whole suite should still take a few minutes.
- **[Window captures depend on the compositor letting Xwayland read the window, and on the window not being covered]** → `GetImage` on the window itself reads its own contents under Xwayland, which worked in the manual checks. A pixel case that sees only the background color reports that, rather than a color mismatch.
- **[The dev-profile binary is slower than release]** → The dump times leave headroom, and `BRINDLE_E2E_BIN` can select a release build.
- **[Cases need `less`, `seq` and `bash` on the machine]** → These are standard on any Linux this runs on. A case checks for its tools and is skipped, not failed, when one is missing.
