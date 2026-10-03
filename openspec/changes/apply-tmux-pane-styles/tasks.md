# Tasks

## 1. Style resolution

- [ ] 1.1 In `src/tmux/style.rs`, add parsing of a bare style string (`fg=blue,bg=default`, without `#[ ]`) into foreground and background `TmuxColor`s, reusing the directive parser from `show-tmux-pane-titles`. Add `resolve_pane_defaults` (design.md decision 3) and a border-style helper that treats tmux's built-in raw values as unset (decision 2). Unit-test:
  - `fg=blue`, `bg=#102030`, `default`, and an empty value;
  - active and inactive panes, where the active style overrides only the colors it sets;
  - raw default versus a custom value for both border options.

  Verify with `cargo test tmux::style`.

## 2. Pane default colors

- [ ] 2.1 Add `default_override` to `Terminal`, and have `default_color` use it for foreground, background and dim foreground. Unit-test `default_color` with and without an override. Verify with `cargo test terminal`.
- [ ] 2.2 Send the `brindle-pane-style` subscription and store both style strings per pane. Re-resolve and push the override when they change or the window's active pane changes, with indexed colors resolved against the pane theme. Verify with a clean `cargo build`.
- [ ] 2.3 Re-send a pane's `refresh-client -r` color reports with its effective colors whenever its override changes (design.md decision 4). Verify with `cargo test` and a clean `cargo build`.

## 3. Border colors

- [ ] 3.1 Send the `brindle-border-style` subscription and store each window's resolved border foregrounds. Use them in `paint_dividers` and as the title base colors (design.md decision 5). Verify with a clean `cargo build`.
- [ ] 3.2 Update `CLAUDE.md`'s tmux notes: pane default colors and divider colors follow tmux styles, the built-in border defaults count as unset, and color reports carry the effective colors. Verify the note matches the code.

## 4. End-to-end verification

Use a throwaway server (`-L brindle-e2e -f /dev/null`) and kill it afterwards. Colors need window captures under Xwayland (x11rb `GetImage`). The color reports can be checked with the query script used for `report-pane-colors-to-tmux`.

- [ ] 4.1 Split a window and set `window-style fg=blue` on one pane. Attach, print plain text and SGR 31 text in both panes, capture, and confirm the plain text is blue only in the styled pane and the red text stays red.
- [ ] 4.2 Set `window-style bg=#102030` on a pane, run the OSC 11 query in it, and confirm the reply is `rgb:1010/2020/3030`. Confirm the other pane still replies with the theme background.
- [ ] 4.3 Set `window-active-style bg=#202020`, switch the active pane, capture, and confirm the background follows the active pane.
- [ ] 4.4 Set `pane-border-style fg=magenta` and `pane-active-border-style fg=yellow`, capture, and confirm the divider colors. Unset both, and confirm the theme colors return, with no green active border.
- [ ] 4.5 Run `cargo test`, `cargo build --release` and `openspec validate apply-tmux-pane-styles --strict`.
