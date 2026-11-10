## 1. Card component (`brindle-ui`)

- [ ] 1.1 Spike: paint four corner cutouts above a child with a square background in the gallery and check the corners and the border by eye (and the fallback in design 2 if not)
- [ ] 1.2 `card_style` in `ui/src/style.rs` (border, radius, header height and padding, ring from `ring_geometry`) with unit tests
- [ ] 1.3 `ui/src/components/card.rs`: `Card`, `CardHeader`, `CardColors`; export from `components/mod.rs` and `lib.rs`
- [ ] 1.4 Gallery section for `Card` (default tokens, owner colours, with and without header, ring) and a `--check` scenario for each spec scenario that can run in-app
- [ ] 1.5 `ui/README.md`: list `Card`, and every difference from shadcn's Card (the rounded-corner cutouts, no `CardDescription`, the ring as a prop)

## 2. Theme helpers

- [ ] 2.1 `Theme::card_border` (the `chrome_border` mix) and the muted foreground for a zone header, with unit tests

## 3. Zones

- [ ] 3.1 `ZONE_GAP` and `CARD_BORDER`; drop `ZONE_DIVIDER_SIZE`; `render_tree` uses the gap; root padding
- [ ] 3.2 `render_leaf`: every zone a `Card` (headerless for one zone), header on the terminal's colours, the active primary line removed
- [ ] 3.3 The ring on the active zone: 2+ zones, keyboard modality, focus inside
- [ ] 3.4 `zone_outer_size` and `can_split_zone` count the card; update their unit tests and add the "card chrome counts" case
- [ ] 3.5 Check the header menu anchor, tooltips and header presses still work through the card

## 4. tmux

- [ ] 4.1 Wrap `TmuxWindowView` in a headerless `Card` with the window theme's colours
- [ ] 4.2 Confirm the size sent by `refresh-client -C` and each pane's `Inset` are computed from the card's inner bounds (e2e `tmux-*` geometry cases)

## 5. Specs, docs, tests

- [ ] 5.1 Update the e2e cases whose geometry or pixel samples change (`zones`, `zone_menu`, `styles`, the tmux window cases); add card cases: border colour from the zone theme, the gap, the ring on keyboard focus only, tmux's one card
- [ ] 5.2 `ui-theme-tokens` e2e/theme checks: zone header on the terminal's colours, the ring on the global tokens
- [ ] 5.3 README and CLAUDE.md lines that describe zone headers and dividers (`zone_view.rs`, `tmux_view.rs`, `theme.rs` entries)
- [ ] 5.4 By eye on every built-in theme: ghost button hover, header contrast, corners, ring (hand to the user: needs a window)
- [ ] 5.5 `cargo build`, `cargo test --workspace`, then `openspec validate zones-shadcn --strict`
