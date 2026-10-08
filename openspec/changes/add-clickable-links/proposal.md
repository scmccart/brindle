# Proposal

## Why

URLs in terminal output (build logs, `git push` hints, compiler errors, `ls --hyperlink`) can't be opened from Brindle: the user has to select the text, copy it and paste it into a browser. Programs that emit OSC 8 hyperlinks get nothing for it either. Every mainstream terminal makes links clickable, and Brindle should too, in local tabs and tmux control-mode panes alike.

## What Changes

- **Link detection**: URLs with a known scheme (`http`, `https`, `file`, `mailto`, `ftp`, `ssh`, `git` and a few more) are found in the text under the mouse. Trailing sentence punctuation and unbalanced closing brackets or quotes are left out, balanced brackets inside a URL are kept, and a URL that the terminal wrapped across rows is found as one link.
- **OSC 8 hyperlinks**: text a program marks with OSC 8 is a link to its URI, and takes precedence over a detected URL in the same cell. Every visible cell of the same hyperlink belongs to it.
- **Hover feedback**: while ctrl is held, the link under the mouse is underlined and the mouse pointer becomes a pointing hand.
- **Opening**: ctrl-click opens the link with `xdg-open`, spawned in the background so the UI never waits for it. The opener can be replaced with the `BRINDLE_LINK_OPENER` environment variable (used by tests).
- **Mouse reporting**: a ctrl-click on a link opens it even when the program has turned on mouse reporting, and nothing is sent to the program. A ctrl-click elsewhere behaves as before. Ctrl-drag starting on a link no longer starts a block selection; alt-drag still does.
- **Copy link**: a `copy_link` action ("Copy Link Under Mouse" in the command palette, no default key) copies the URL of the link under the mouse.
- **tmux control mode**: links work in panes. OSC 8 reaches panes because tmux forwards pane output verbatim, and pane snapshots on attach keep it.
- **Debug hooks**: `--hover` and `--click` steps move or click the mouse over a cell of the active terminal (in-process, not OS input), and the screen dump reports the hovered link, so the end-to-end suite can check all this.

Out of scope: opening links without a modifier, showing the URL in a tooltip or status line, always-on underlining of OSC 8 text, user-defined link patterns, and keyboard-driven link hints.

## Capabilities

### New Capabilities
- `links`: finding links (detected URLs and OSC 8 hyperlinks) in a terminal, the ctrl-hover feedback, opening and copying them, and how they behave in tmux panes.

### Modified Capabilities
- `terminal-emulation`: mouse reporting gains the ctrl-click-on-a-link exception, and ctrl-drag block selection no longer starts on a link.
- `command-line`: the debug hooks gain `--hover` and `--click` steps and a hovered-link line in the screen dump, and the opener override is documented.

## Impact

- **Code:**
  - new `src/terminal/links.rs`: URL scanning (pure, unit-tested) and the opener;
  - `src/terminal/mod.rs`: finding the link at a grid point (OSC 8 or detected);
  - `src/terminal_view.rs`: hover state, ctrl-click handling, `copy_link`;
  - `src/terminal_element.rs`: underline and pointer for the hovered link, modifier tracking;
  - `src/actions.rs`, `src/main.rs`, `src/workspace.rs`: the action and the debug hooks.
- **Tests:** unit tests for detection; new e2e cases for local tabs, mouse-reporting programs and tmux panes.
- **Docs:** README usage notes.
- **Dependencies:** none.
