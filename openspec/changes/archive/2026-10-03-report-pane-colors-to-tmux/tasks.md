# Tasks

## 1. Report pane colors

- [x] 1.1 Add a pure helper in `src/tmux/mod.rs` that builds the two `refresh-client -r` commands for a pane from a foreground and a background `Color` (design.md, second decision). Unit-test it against the exact string checked on tmux 3.6, `refresh-client -r "%0:\033]11;rgb:1a1a/1b1b/2626\033\\"`, plus the OSC 10 form and a channel at `00`/`ff`. Verify with `cargo test tmux`.
- [x] 1.2 In the pane-creation loop, send both reports for each new pane with `Pending::Ignore`, before the restore commands, using the pane's theme (`config.profile_theme(&profile)`). Verify with `cargo test` and a clean `cargo build`.
- [x] 1.3 In `TmuxSession::set_config`, re-send the reports for every pane in `self.panes` with the reloaded theme. Verify with a clean `cargo build`. Behaviour is checked in group 2.
- [x] 1.4 Add a line to `CLAUDE.md`'s tmux notes: control clients have no tty, so tmux answers OSC 10/11 as black unless Brindle reports per-pane colors with `refresh-client -r`, which it does on pane creation and on reload. Verify the note matches the code.

## 2. End-to-end verification

Warn the user and wait for their go-ahead before launching Brindle windows. Use a throwaway server (profile `tmux_args = ["-L", "brindle-e2e", "-f", "/dev/null"]`) and a throwaway `BRINDLE_CONFIG`, and kill the server afterwards. The query command used below is `printf '\e]11;?\a'; read -t1 -rd x r; printf '%q\n' "$r"`. Use the OSC 10 form where noted.

- [x] 2.1 Attach with `--send` running the OSC 11 query, then the OSC 10 query, and `--dump-screen-after 5`. Confirm the dump shows the theme's background and foreground as `rgb:` values, not `rgb:0000/0000/0000`.
- [x] 2.2 Attach with `--action tmux_split_right`, then `--send` the OSC 11 query into the new pane, and `--dump-screen-after 5`. Confirm the new pane gets the theme background.
- [x] 2.3 Attach with `--dump-screen-after 7`. About 2 s in, have a second command change the profile's `theme` in the throwaway config (for example to `gruvbox-dark`). After the reload, `--send` the OSC 11 query. Confirm the reply is the new theme's background.
- [x] 2.4 After Brindle detaches, run the OSC 11 query in the same pane through a second tmux client or `send-keys` plus `capture-pane`. Record whether the report survives the detach, which answers design.md's open question. No code change is expected either way. Result: the report survives the detach and is used while any control client is connected. With none connected, tmux uses its tty clients' colors (recorded in design.md, Risks).
- [x] 2.5 Run `cargo test`, `cargo build --release` and `openspec validate report-pane-colors-to-tmux --strict`.
