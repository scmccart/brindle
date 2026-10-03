# Design

## Context

`TmuxSession` creates a remote `Terminal` for each pane in its pane-creation loop (`src/tmux/mod.rs`), using `config.profile_theme(&profile)`, and sends the three restore commands there. A config reload reaches the session through `set_config`, which updates only `profile` and `config`. The pane terminals' themes are updated separately, through `TmuxWindowView::set_profile`.

tmux's `refresh-client -r %<pane>:<report>` takes a raw OSC 10/11 report, for example `ESC ] 11 ; rgb:1a1a/1b1b/2626 ESC \`. It was checked on tmux 3.6 with the escape written as `\033` inside a double-quoted tmux argument: tmux's parser turns `\033` into ESC, and a pane's OSC 11 query then returned the reported value.

## Goals / Non-Goals

**Goals:**
- Panes report the same colors a local tab's `ColorRequest` handler would give for an unmodified palette, which is the theme's foreground and background.

**Non-Goals:**
- Following colors a program changes with OSC 10/11 *set* sequences. tmux tracks those for the pane itself.
- Doing anything for classic (`tmux = "plain"`) tabs. There, tmux asks Brindle's PTY terminal, which already answers.

## Decisions

- **Report the theme colors, not the pane terminal's dynamic `colors` table.** That table only diverges when a pane's program sets colors, and tmux already handles that case for the pane. Using the theme keeps the report a pure function of the config, so it can be sent before the pane terminal exists and re-sent from `set_config` without reading any terminal.
- **Build the command with an explicit `\033` escape, not `protocol::quote`.** `quote` escapes backslashes, so it would turn `\033` into a literal `\\033`. A small helper (`color_report_commands(pane, fg, bg) -> [String; 2]`) formats `refresh-client -r "%N:\033]1X;rgb:RRRR/GGGG/BBBB\033\\"`, with each 8-bit channel doubled into 16 bits (`1a` → `1a1a`), the same form tmux and xterm use. The argument contains only hex digits, `%`, `:`, `;` and `/`, so it needs no other quoting. The helper is unit-tested against the exact string checked on tmux 3.6.
- **Send the reports before the restore commands for a new pane.** They don't depend on the snapshot. They are matched as `Pending::Ignore`, so their place in the FIFO doesn't matter, but sending them first narrows the window in which a program that just started in the pane queries before the report arrives.
- **Re-send for every live pane in `set_config`.** Sending is cheap: two short commands per pane, and only on reload. Comparing old and new themes isn't worth the code.

## Risks / Trade-offs

- [A program queries before the report arrives, for example the shell of a brand-new pane at startup] → It gets tmux's black answer, as today. Sending the reports in the same burst as the pane's first commands keeps the window small, and programs that re-query (such as editors on focus or startup) recover.
- [tmux older than the `-r` option] → The command fails with `%error`. The `Pending::Ignore` path logs a warning and nothing else changes, as the "tmux without color reports" scenario requires.
- [Several control clients attached to one session report different themes] → The last report wins: tmux keeps one report per pane. This is acceptable because Brindle can't control other clients.
- [A report outlives Brindle's client] → tmux stores the report on the pane (`wp->control_fg`/`control_bg`, set in `cmd-refresh-client.c`). It answers with that report whenever *any* control client is connected to the server, and otherwise falls back to the colors of attached tty clients (`window_pane_get_bg_control_client` in `window.c`, tmux 3.6). So after Brindle detaches, a normal terminal client gets its own colors back. Another control client that doesn't report gets Brindle's last colors, which is better than black. The next Brindle attach reports again. The end-to-end check confirmed both cases: no client connected gives no answer, and a non-reporting control client gets the last reported theme.
