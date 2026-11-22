# Design

## Context

- **Badges.** `brindle-term-core`'s `context.rs` keeps the OSC 3008 stack (`Contexts`) and derives `Option<Badge>` (`BadgeKind::{Privileged, Elsewhere}`, a label of at most 24 characters). `Terminal` caches it in `self.badge` when a context message changes the stack, and `ZoneView`/`Workspace` copy it to the zone header and tab face (`refresh_badge`). On Windows nothing sends OSC 3008, so no badge ever shows there, even in an elevated Brindle.
- **Process info.** `src/terminal/process.rs` is the process seam. On Windows a tab's title is the newest descendant of its program in a Toolhelp `Snapshot`, taken once per 1 s title tick inside `Workspace`'s `update_in`, so on the UI thread. Brindle already reads its own elevation (`win::elevated`, through `brindle_remote_protocol::win::is_elevated`) to pick its state and server directories.
- **Palette gating.** `actions::palette_lists(action, PaletteFacts { prompt_marks })` hides only `CopyLastCommand` until a prompt mark; `Terminal::last_command_line_text` refuses without one. `select_last_output` and `copy_last_output` read the marks with no such check.
- **Installs.** `scripts/install.sh` and `install.ps1` build from a checkout. Releases (`release.yml`) upload the Linux tarball, the Windows zip, `brindle-ui-tokens.css` and `SHA256SUMS`. `scripts/uninstall.sh` removes `brindle` and `brindle-remote`, not `brindle-remote-linux-x86_64`.
- **Clipboard.** All copies go through GPUI's `write_to_clipboard` as plain `ClipboardItem::new_string`; paste reads `read_from_clipboard().text()` and `paste_bytes` turns `\r\n` and `\n` into `\r`. Selections come from alacritty's `selection_to_string`, which joins rows with `\n` on every platform. The e2e clipboard cases (`cases/clipboard.rs`) set the clipboard with OSC 52 and paste it back inside Brindle, so no case crosses into another process, except `windows-pwsh-integration`'s `Get-Clipboard` read.

- **Right-click.** `terminal/mouse.rs`'s pure `right_press(reports, has_selection, setting) -> RightPress::{Report, Paste, Nothing}` returns `Nothing` for `(Paste, true)`, so a stray right-click can't paste over a selection (a decision from `polish-settings-and-clipboard`). `TerminalView` acts on it; `Terminal::paste` already clears the selection. Mouse copies and pastes show nothing.
- **Findings from the user's testing.** Copy and paste between Brindle and other applications works at a shell prompt. The differences show inside TUI programs that report the mouse, since Brindle hands them the clicks (shift gives them back to Brindle) and many copy and paste in their own way.

## Goals / Non-Goals

**Goals:**
- A privileged badge on any local Windows tab whose foreground program is elevated, with no UI-thread work added.
- The two output commands hidden and inert without shell integration, like Copy Last Command Line.
- `curl … | sh` and `irm … | iex` installs and uninstalls from a release, checked against `SHA256SUMS`, matching the from-source layout.
- Automated cross-process clipboard checks on both platforms, a manual matrix for what needs real keys and other applications, and Windows line ends on copy.

**Non-Goals:**
- Detecting `sudo` on Linux. Brindle could read the foreground process's effective uid from `/proc`, but systemd's `run0` already announces itself, and that is a separate decision.
- Elevation inside remote sessions. Their pids belong to another machine.
- macOS or aarch64 installers. No release archive exists for them.
- Fixing clipboard bugs the new cases may find. Each one is fixed as its own change, with the failing case as its test.
- Signing the scripts or checking attestations in them. `gh attestation verify` stays the documented check for those who want it.

## Decisions

### 1. Elevation: a value from the seam, a choice in term-core

- **Pure (term-core, `context.rs`):** `pub enum Elevation { Standard, Elevated, Unknown }`, and `pub fn badge(notable: Option<Badge>, elevation: Elevation) -> Option<Badge>`. It returns `notable` when it is `Some`, a `Badge { Privileged, "Administrator" }` for `Elevated`, and `None` otherwise. `Unknown` and `Standard` both mean no badge, and they stay apart so the dump and logs can tell an unreadable token from a standard one (`types R1`, matched without a wildcard, `railway R3`). Unit tests sit beside the function (`fcis R1`).
- **Seam (`terminal/process.rs`, `cfg(windows)`):** `newest_descendant` gains a pid twin, so the title and the elevation name the same process, and `fn elevation(pid) -> Elevation` calls `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`, `OpenProcessToken(TOKEN_QUERY)` and `GetTokenInformation(TokenElevation)`. Non-Windows builds have an `elevation` stub returning `Unknown`, so no new `cfg` leaves the seam (`ports R4`).
  - Only a token actually read decides. Any failure to open the process or its token, `ERROR_ACCESS_DENIED` included, is `Unknown`, never inferred as `Elevated`: a denial can have other causes (a protected process, another user's, a policy), and a wrong privileged badge is worse than none. Task 1.1 checks whether an unelevated process can read an elevated process's token. If it can't, an unelevated Brindle shows no badge for `sudo` inline, and design.md records that limit; an elevated Brindle still reads every token.
- **Shell (`Workspace` title tick):** the tick gathers, on the UI thread, each local Windows terminal's root pid (cheap reads). On the background executor it takes the `Snapshot`, resolves each root's newest descendant (name and pid), and looks up the elevation of each pid not yet in an `ElevationCache` (`HashMap<u32, Elevation>`, kept by the workspace, pruned to the pids in the latest snapshot so a reused pid is read again). Then it applies titles and elevations back on the UI thread. This moves the existing Toolhelp walk off the UI thread too (`process R13`), and keeps `render` untouched (`process R14`).
- `Terminal` stores `elevation` beside `contexts` and recomputes `self.badge = context::badge(self.contexts.notable(), self.elevation)` when either changes. `refresh_badge` already carries it to the face and header, and the dump's `--- badge` line already prints it.
- An elevated Brindle needs no special case: its server runs elevated, so every program it starts reads `Elevated`.

Alternatives considered:
- *Badge every tab when Brindle itself is elevated, from `win::elevated()` alone.* Rejected: it misses `sudo` inline, the case a user most often forgets about.
- *Compute it in the terminal server's core and send it with `PROCESS`.* Rejected for now: it would need a new negotiated message (`process R4`, `R5`) for something the client can read itself, since local sessions share its machine.
- *Ask the console for its attached processes (`GetConsoleProcessList`).* That would be more exact, since it excludes a `Start-Process -Verb RunAs` window, but it needs `AttachConsole` into the pseudoconsole from a process that owns none. See Risks.

### 2. Output commands gated like Copy Last Command Line

`palette_lists` becomes `facts.prompt_marks || !needs_prompt_marks(action)`, where `needs_prompt_marks` matches `CopyLastCommand`, `SelectLastOutput` and `CopyLastOutput`. The jump-to-prompt actions stay out of it: without marks they scroll a line, as specified. `Terminal::last_output_text` (and the selection path) return `None` until `marks.prompt_seen()`, as `last_command_line_text` does, so `--action` and user bindings are inert too. These are pure checks with unit tests in `actions.rs` and `terminal/mod.rs`.

### 3. One-command installers

- **Files:** `scripts/get.sh` (POSIX `sh`, shellcheck-clean, run by `lint-workflows`) and `scripts/get.ps1` (PowerShell 5.1 compatible), published as the release assets `install.sh` and `install.ps1`. The release asset names are what users type. The repository names keep the from-source `install.*` as they are.
- **Version:** each file holds `@VERSION@`, which `release.yml` replaces with the tag's version before upload and before `SHA256SUMS` is written. `BRINDLE_VERSION` (`$env:BRINDLE_VERSION`) overrides it. An unstamped copy, run from a checkout, asks for `releases/latest`.
- **Source:** `BRINDLE_DOWNLOAD_BASE` (default `https://github.com/scmccart/brindle/releases/download/v<version>`) lets CI point a script at a local directory of built assets. It is documented as a testing hook.
- **Truncation safety:** each script's body is one function (`main "$@"` on the last line, `& { … }` in PowerShell), so a partial download defines nothing it runs.
- **Linux:** checks `uname -s`/`-m`, needs `curl` or `wget` and `sha256sum` or `shasum -a 256`, downloads into `mktemp -d` (removed by `trap`), checks the archive's line in `SHA256SUMS`, then installs `brindle` and `brindle-remote-linux-x86_64` with `install -Dm755`. It reuses install.sh's `writable`/`sudo` rule, runs `--install-desktop --prefix`, then runs `brindle --version` to catch a glibc that is too old, naming the requirement. `--uninstall` runs the same steps as `scripts/uninstall.sh`, which also learns to remove `brindle-remote-linux-x86_64`.
- **Windows:** never calls `exit`, which would close the user's shell under `iex`. It `throw`s with `$ErrorActionPreference = 'Stop'`, sets TLS 1.2 for 5.1, downloads with `Invoke-WebRequest -UseBasicParsing`, checks `Get-FileHash`, expands into a temp directory, and installs the three files with install.ps1's rename-aside `Install-File`, then adds the `PATH` entry and runs `--install-desktop`. Parameters come from the environment (`BRINDLE_VERSION`, `BRINDLE_PREFIX`, `BRINDLE_UNINSTALL`) because `iex` can't pass them. `-Version`/`-Prefix`/`-Uninstall` also work through `& ([scriptblock]::Create((irm …)))`.
- **Shared steps stay in one place:** the from-source scripts keep their own copies of the install and uninstall steps rather than sourcing the get scripts, because the get scripts must run standalone. A CI step installs and uninstalls with both, so the two copies can't drift unnoticed.
- **CI:** `build-linux.yml` runs `get.sh` against the just-built tarball (`BRINDLE_DOWNLOAD_BASE=file://…`, a stamped copy, a private `PREFIX` and `XDG_DATA_HOME`), checks `--version`, a checksum-mismatch run that must fail and install nothing, and `--uninstall`. `build-windows.yml` does the same with `get.ps1`, invoked through `iex`, in a child `pwsh` so a stray `exit` would show as a failure. The `-Prefix` is private and the Start Menu shortcut is removed again, as the existing `--install-desktop` check does.

### 4. Clipboard: cases that cross the process boundary

- **New e2e cases** (`cases/clipboard.rs`, Linux under Xvfb; `cases/win.rs`, Windows). The "other application" is the pane's own shell running a clipboard client, which is a separate process and a separate X client from Brindle. That is not synthetic input, so it is safe on the desktop rule.
  - `clipboard-copy-to-app`: print a marker, select it with `--press`/`--hover`/`--release`, `--action copy`, then `xclip -o -selection clipboard` (`Get-Clipboard -Raw`) prints it back.
  - `clipboard-paste-from-app`: `printf 'echo from-$((6*7))' | xclip -selection clipboard` (`Set-Clipboard`), then `--action paste` and Enter, and the screen shows `from-42`.
  - `clipboard-osc52-to-app`: OSC 52 `c`, then `xclip -o` reads it.
  - `clipboard-primary-both-ways` (Linux): a drag-selection read with `xclip -o -selection primary`, and `xclip -selection primary` set by the shell pasted with `--action paste_primary` (the debug hooks have no middle button).
  - `clipboard-multiline-line-ends`: two lines copied; Linux reads `one\ntwo` (`xclip -o | od -c`), Windows reads `one\r\ntwo` (`[int[]][char[]](Get-Clipboard -Raw)`). A clipboard set to `a\r\nb` by the shell pastes into `od -c` (or `[Console]::In`) as `a\rb`.
  - `clipboard-image-pastes-nothing`: `xclip -t image/png` (`Set-Clipboard` with a bitmap through `System.Windows.Forms.Clipboard`), paste, and a marker typed after it shows nothing arrived before it.
  - These need `xclip` on CI's e2e runner and in README's Xvfb package list. Without it, the Linux cases skip with a reason, as the `remote-*` cases do without Docker; CI sets `BRINDLE_E2E_CLIPBOARD_REQUIRED=1`, which turns the skip into a failure.
- **Windows line ends:** `Terminal::selection_text` (and the Copy Last paths) pass through a pure `copied_text(text, LineEnd)` in `terminal/mod.rs`, with `LineEnd::{Lf, CrLf}` chosen by a constant in the process seam (`LOCAL_LINE_END`, beside `LOCAL_PATH_STYLE`), so no new `cfg` appears outside it. OSC 52 text is written untouched. Windows Terminal, conhost and PuTTY all copy CRLF, and some Windows applications (older edit controls, Excel cells) treat a bare `\n` as nothing.
- **Manual matrix:** what needs real keys, real applications or Wayland becomes `docs/manual-checks/clipboard.md`, linked from CLAUDE.md's manual checks. Its rows:

| # | Platform | Steps | Expected |
|---|---|---|---|
| 1 | all | Select a word, ctrl-shift-c, paste in a browser's address bar | The word |
| 2 | all | Copy a line in a browser, ctrl-shift-v at a prompt | The line, not run |
| 3 | all | Copy rich text (bold link) in a browser, paste in a tab | Visible text only |
| 4 | all | Copy two lines in a tab, paste in a plain-text editor | Two lines; on Windows Notepad and an Excel cell agree |
| 5 | all | `copy_on_select = true`: drag-select, paste elsewhere with the app's own paste key | The selection |
| 6 | all | `right_click = "paste"`: copy in another app, right-click in a tab | The text |
| 7 | all | Copy in tab A, close tab A, paste in another app | Still there |
| 8 | all | Copy in Brindle, quit Brindle, paste elsewhere | Windows: still there. Linux: still there only with a clipboard manager running; note which desktop |
| 9 | all | tmux `set-clipboard on`, copy-mode yank, paste in another app | The yank (OSC 52) |
| 10 | all | Inside `ssh` to a host in tmux, yank, paste locally | The yank |
| 11 | all | Shift-insert and ctrl-insert in a tab | Paste and copy, as ctrl-shift-v/-c |
| 12 | Linux | Select in a GTK app, middle-click in a tab | The selection (PRIMARY) |
| 13 | Linux | Drag-select in a tab, middle-click in a GTK app | The selection |
| 14 | Linux Wayland | Rows 1, 2, 12 and 13 in a native Wayland session (not Xwayland) | Same as X11 |
| 15 | Linux Wayland | Copy from an Xwayland app, paste in Brindle; and the reverse | The text |
| 16 | Windows | Middle-click in a tab after selecting in another app | Brindle's own primary, not the other app's (by design); note if this is the confusion |
| 17 | Windows | Win+V history after copying in Brindle | The copy is listed |
| 18 | Windows | Paste a path copied from Explorer (ctrl-c on a file) | Text paths, or nothing if Explorer offered only files; no error |
| 19 | Windows | Copy in an elevated Brindle, paste in an unelevated app, and the reverse | Works both ways (the clipboard isn't integrity-bound) |
| 20 | all | Paste a 1 MB text (copied from an editor) into `cat > /dev/null` or `$null = [Console]::In.ReadToEnd()` | Arrives whole, window stays responsive |

### 5. Right-click pastes over a selection

`right_press` drops its `has_selection` parameter: with `RightClick::Paste` it returns `Paste` whenever the press isn't reported, and `RightPress` keeps its three variants. The selection is cleared by the paste itself (the `Paste` requirement already says a paste clears it), so with an empty clipboard nothing is pasted and the selection stays, which is the safe outcome. A reported press (a TUI using the mouse) is unchanged; shift-right-click comes to Brindle as before. The unit tests in `mouse.rs` change with it, and the e2e cases that asserted "nothing over a selection" in `cases/clipboard.rs` assert a clear and paste instead. The old rule protected against a stray click pasting over a selection. The user asked for the PuTTY flow, and `right_click` is opt-in, so whoever turns it on wants it.

### 6. Mouse clipboard feedback

- **Pure model (`src/terminal/feedback.rs`, joins `PURE_MODULES`):** `enum FeedbackKind { Copied, Pasted, NothingToPaste }`, `struct Feedback { kind, at: Point<Pixels>, shown: Instant }` and `fn phase(&self, now) -> Phase` with `enum Phase { Shown, Fading(f32), Gone }` (about 800 ms shown, then 200 ms fade). It takes `now` as an argument, so the tests need no clock (`fcis R2`). A new feedback replaces the old, so two can never stack.
- **What sets it:** `TerminalView`'s mouse-up, when `finish_selection` copies to the clipboard (`Copied`), and the right press when `right_press` says `Paste` (`Pasted`, or `NothingToPaste` when the clipboard has no text). `finish_selection` returns whether it wrote the clipboard, so the view doesn't decide it again. The key and palette actions never set it.
- **Drawing:** an absolutely positioned child of the view, as the find bar is, offset from the pointer by 12 px down and right, kept inside the grid's bounds. It uses brindle-ui's tooltip surface (`TooltipStyle`, so it matches the hover tooltips and every theme's tokens) with `Icon::Copy`, or the new `Icon::ClipboardPaste` and `Icon::Ban` (lucide's `clipboard-paste` and `ban`, added to brindle-ui) for pasted and nothing to paste. It doesn't take a focus handle and has no mouse handlers, so it catches neither keys nor clicks. While a feedback isn't `Gone`, one timer task, about 60 Hz and paced as `terminal/pace.rs` describes, notifies the view for the fade and ends at `Gone`. Nothing wakes the view at other times (`process R13`). On Windows, a timer that keeps waking is what starves `WM_PAINT`, so the task ends itself.
- **Dump:** `--- feedback kind=copied|pasted|nothing-to-paste` while one isn't `Gone`, after the badge lines, so the e2e cases can check it within its second. `Run::dump_after_last_step` names the 1 s limit, as the testing rules ask for an unscaled offset.
- Alternatives: flashing the selection (no use for paste, and it hides the text being copied) and a toast on the config line (too far from the pointer, and it would compete with notices). Rejected.

## Risks / Trade-offs

- **Token reads across integrity levels.** If an unelevated process can't read an elevated one's token, detection reads `Unknown` there and shows nothing, which is today's behaviour, so `sudo` inline in an unelevated Brindle goes unbadged. Task 1.1 settles it before the rest is built on it, and a better source (such as the holder reading its own console's processes) would be a follow-up change.
- **Descendant, not console.** A program the shell starts elevated in its own window (`Start-Process -Verb RunAs`) is the tab's newest descendant while it runs, so the badge shows though the tab itself isn't elevated. The tab title already names that process, so the two agree. `GetConsoleProcessList` from the holder would be exact, and is the follow-up if this confuses.
- **Polling delay.** The badge appears up to a second after `sudo` starts, as titles do.
- **`curl | sh` trust.** The script is only as trustworthy as the release. It checks the archive against `SHA256SUMS` from the same release, which catches corruption but not a compromised release. The README keeps `gh attestation verify` for provenance.
- **CRLF on Windows copies** changes what a script reading `Get-Clipboard` sees. The PowerShell integration case that reads a single line is unaffected, since one line has no line end.
- **Right-click over a selection now pastes.** Someone who right-clicks out of habit while text is selected pastes. The setting is opt-in, the label says what happened, and the paste is bracketed where the program asks.
- **New dependency for e2e:** `xclip` on the runner. It is a small apt package with no daemon.

## Migration Plan

Nothing to migrate. The installers first appear on the next release. Until then, the README shows them as coming with that release.

## Open Questions

- Row 16: should Windows' middle click paste the system clipboard when Brindle's own primary is empty, or always? Today it pastes nothing. The user's answer to the matrix decides whether a follow-up changes it.
