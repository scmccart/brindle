# Clipboard checks by hand

These rows need real keys, a real pointer, other applications on the clipboard,
or Wayland. The e2e suite covers the same paths between Brindle and a program
in its own pane (the `clipboard-*` cases, which use `xclip` or `Get-Clipboard`),
but it sends no real keys and can't cross into another application's window,
so these stay manual.

Try a build without touching your config, saved session or terminal server:

- Linux: `scripts/run-throwaway.sh`, with `--config FILE`, `--keep` and
  `-- <brindle args>`.
- Windows: `scripts/run-throwaway.ps1`, with `-Config FILE`, `-Keep` and
  `-BrindleArgs '-d','C:/Temp'`.

Record each row in the Result column as Pass, Fail or Not run, with the app
you used. Rows that name a setting assume `right_click` and `copy_on_select`
are at their defaults unless the row says otherwise.

## Matrix

| # | Platform | Steps | Expected | Result |
|---|---|---|---|---|
| 1 | all | Select a word, ctrl-shift-c, paste in a browser's address bar | The word | |
| 2 | all | Copy a line in a browser, ctrl-shift-v at a prompt | The line, not run | |
| 3 | all | Copy rich text (bold link) in a browser, paste in a tab | Visible text only | |
| 4 | all | Copy two lines in a tab, paste in a plain-text editor | Two lines; on Windows Notepad and an Excel cell agree | |
| 5 | all | `copy_on_select = true`: drag-select, paste elsewhere with the app's own paste key | The selection | |
| 6 | all | `right_click = "paste"`: copy in another app, right-click in a tab | The text | |
| 7 | all | Copy in tab A, close tab A, paste in another app | Still there | |
| 8 | all | Copy in Brindle, quit Brindle, paste elsewhere | Windows: still there. Linux: still there only with a clipboard manager running; note which desktop | |
| 9 | all | tmux `set-clipboard on`, copy-mode yank, paste in another app | The yank (OSC 52) | |
| 10 | all | Inside `ssh` to a host in tmux, yank, paste locally | The yank | |
| 11 | all | Shift-insert and ctrl-insert in a tab | Paste and copy, as ctrl-shift-v/-c | |
| 12 | Linux | Select in a GTK app, middle-click in a tab | The selection (PRIMARY) | |
| 13 | Linux | Drag-select in a tab, middle-click in a GTK app | The selection | |
| 14 | Linux Wayland | Rows 1, 2, 12 and 13 in a native Wayland session (not Xwayland) | Same as X11 | |
| 15 | Linux Wayland | Copy from an Xwayland app, paste in Brindle; and the reverse | The text | |
| 16 | Windows | Middle-click in a tab after selecting in another app | Brindle's own primary, not the other app's (by design); note if this is the confusion | |
| 17 | Windows | Win+V history after copying in Brindle | The copy is listed | |
| 18 | Windows | Paste a path copied from Explorer (ctrl-c on a file) | Text paths, or nothing if Explorer offered only files; no error | |
| 19 | Windows | Copy in an elevated Brindle, paste in an unelevated app, and the reverse | Works both ways (the clipboard isn't integrity-bound) | |
| 20 | all | Paste a 1 MB text (copied from an editor) into `cat > /dev/null` or `$null = [Console]::In.ReadToEnd()` | Arrives whole, window stays responsive | |
| 21 | Linux and Windows | Select text with the mouse, then watch the copy label beside the pointer; then click on the label | The label shows a copy icon beside the pointer, fades in about a second, and the click reaches the terminal under it, not the label | |
| 22 | Linux and Windows | `right_click = "paste"`, select text, copy other text in another app, right-click over the selection | The selection clears, the clipboard text is pasted, and a paste icon shows beside the pointer | |
| 23 | Linux and Windows | `right_click = "paste"`, select text, leave the clipboard empty (or an image only), right-click over the selection | Nothing is pasted, the selection stays, and a ban icon shows beside the pointer | |
| 24 | Windows | Set sudo to inline mode, then `sudo pwsh` in a tab; accept the prompt; exit that pwsh | The tab shows the Administrator badge (a warning icon, tooltip "Running as Administrator") within about a second of sudo starting; it drops within about a second of the pwsh exiting | |
| 25 | Windows | On a machine with no Brindle (no earlier install, no `PATH` entry), unpack the release zip, run `brindle.exe`, open a local tab | The window opens and the tab shows a prompt; no error notice | |
| 26 | Windows | Same machine: open an SSH profile to a host you can reach with `ssh` | The tab connects and shows the remote shell; no error line | |
| 27 | Windows | Same machine, Docker Desktop running: open a container profile | The tab opens a shell inside the container; no error line | |

Rows 21 to 23 are about the mouse. Row 21's label is an icon only, with no text.

## Findings

The user's own testing found that copy and paste between Brindle and other
applications works at a shell prompt. The differences that remain come from
full-screen programs that report the mouse, such as htop or vim with the mouse
on. Brindle hands those programs the clicks, and each one copies and pastes in
its own way. Shift gives the mouse back to Brindle, so shift-drag selects and
shift-right-click pastes in such a program.

When a row fails inside such a program, first check the same steps at a shell
prompt. If they work there, the difference is the program's, not Brindle's
clipboard.

## Follow-up

Each failing row gets its own follow-up change, with the failing case as its
test. Row 16 also answers the open question in
`openspec/changes/polish-elevation-install-and-clipboard/design.md`: whether
Windows middle click should paste the system clipboard when Brindle's own
primary selection is empty.
