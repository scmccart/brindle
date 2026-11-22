# Proposal

## Why

Five rough edges, collected into one change. On Windows nothing sends OSC 3008, so a tab running elevated (an elevated Brindle, or `sudo` in inline mode) shows no privileged badge, though that is the context a user most needs to see. The command palette lists Select and Copy Last Command Output in a terminal without shell integration, where they can only do nothing; Copy Last Command Line is already hidden there. Installing a release takes five commands, where most tools today take one (`curl … | sh`, `irm … | iex`). And copy and paste between Brindle and other applications feels inconsistent, but nothing tests it across the process boundary: every clipboard e2e case sets and reads the clipboard from inside Brindle. Testing showed the system clipboard itself works; the differences come from TUI programs that take the mouse and handle copy and paste their own way. And with `right_click = "paste"`, a right-click over a selection does nothing, which reads as broken, while mouse copies and pastes give no sign that they happened.

## What Changes

- **Brindle detects elevation on Windows.** A local Windows tab whose foreground program (the process its title names) runs elevated shows the privileged badge, labelled `Administrator`, with the tooltip "Running as Administrator", on the tab face and zone header, as an OSC 3008 `elevate` context does. An OSC 3008 context a program announces can't replace it, since a program could use one to hide the warning; it decides only in unelevated tabs. It needs nothing from the program, so it covers an elevated Brindle's tabs, `sudo` in inline mode and a shell started elevated. The lookup runs off the UI thread and is cached per process.
- **Output commands hidden without integration.** Select Last Command Output and Copy Last Command Output join Copy Last Command Line: the palette doesn't list them, and the actions do nothing, until the terminal has received a prompt mark.
- **One-command installs.** Two new scripts, published as release assets `install.sh` and `install.ps1`:
  - `curl -fsSL https://github.com/scmccart/brindle/releases/latest/download/install.sh | sh`
  - `irm https://github.com/scmccart/brindle/releases/latest/download/install.ps1 | iex`

  Each downloads its platform's archive from the same release, checks it against `SHA256SUMS`, and installs it as the from-source scripts do: the same directories, PATH entry, launcher entry or Start Menu shortcut, and helper beside the binary. So `scripts/uninstall.sh` and `scripts/uninstall.ps1` remove either install, and each script also takes `--uninstall` (`-Uninstall`). Without a version given, a script installs the release it was published with.
- **Clipboard checks across applications.** New e2e cases copy in Brindle and read the result from another process (the pane's own shell running `xclip`, or `Get-Clipboard` on Windows), and the other way round, for the keys, OSC 52, the primary selection and multi-line text. A manual matrix covers what only real keys and other desktop applications can show, including Wayland. `terminal-emulation` gains a requirement that states what is shared with other applications.

- **Right-click over a selection clears it and pastes.** With `right_click = "paste"`, a right-click pastes the clipboard whether or not text is selected, and the paste clears the selection, as every paste does. With `copy_on_select` on too, that is PuTTY's select-then-right-click flow. This reverses the rule from `polish-settings-and-clipboard` that a right-click over a selection does nothing. The default, `right_click = "none"`, is unchanged. An empty clipboard still pastes nothing and leaves the selection.
- **Feedback for mouse copy and paste.** A small "Copied", "Pasted" or "Nothing to paste" label shows at the pointer for about a second, then fades. It shows only for `copy_on_select` copies and right-click pastes, never for keys. It takes no focus, lets clicks through and doesn't move the grid.

- **Archives checked out of the box.** Every release archive, unpacked alone, must run local tabs and SSH and container profiles: the Windows zip with `brindle-remote.exe` (ConPTY embedded) and the Linux helper, the Linux archive with its static helper. Packaging already does this on `main` (v0.3.0 predates it). What's new is a CI check that runs the e2e local and remote cases against the installed archive instead of the build directory.

No breaking changes: both new release assets are additive, and the right-click change is a behaviour change of an opt-in value, not a changed key or default.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `program-contexts`: a local Windows terminal whose foreground program runs elevated has a privileged notable context of its own, which no announced context can replace.
- `shell-integration`: a new requirement hides Select and Copy Last Command Output, and makes them do nothing, until a prompt mark has arrived.
- `desktop-integration`: one-command installs and uninstalls from a release, on Linux and Windows.
- `release-process`: archives must work out of the box, locally and for SSH and container profiles; and a new requirement adds `install.sh` and `install.ps1` to every release, listed in `SHA256SUMS`.
- `terminal-emulation`: the clipboard is the system's, shared with other applications both ways, with the line endings each side sees; right-click paste clears a selection and pastes, shift-right-click included over mouse reporting; mouse copies and pastes show a label at the pointer.
- `configuration`: `right_click = "paste"` is described as clearing any selection and pasting.

## Impact

- `brindle-term-core` `context.rs`: a pure function choosing the badge from the announced context and the detected elevation.
- `src/terminal/process.rs` (the process seam): a Windows elevation query for a pid. `src/terminal/mod.rs` and `workspace.rs`: a per-pid elevation cache fed off the UI thread on the title tick, and the badge read through it.
- `src/actions.rs` `palette_lists`, `src/terminal_view.rs`: the two output commands gated on prompt marks.
- `scripts/get.sh`, `scripts/get.ps1` (new), `.github/workflows/release.yml` (stamp the version, upload, add to `SHA256SUMS`), `build-linux.yml` and `build-windows.yml` (run each script against the built archive), README.
- `tests/e2e/cases/clipboard.rs` and `win.rs`: cross-application cases. CI's e2e job and README's Xvfb packages gain `xclip`.
- `src/terminal/mouse.rs` `right_press`, `src/terminal_view.rs` (the feedback label, as an overlay child like the find bar), `tests/e2e/cases/clipboard.rs`'s right-click cases, README and the settings dialog's Clipboard text.
- `docs/code-map.md`, CLAUDE.md's manual checks.
