# Tasks

## 1. General section

- [ ] 1.1 In `render_general`, give the choice frame `flex_basis(Length::Auto)` after `flex_none()`, so its outline encloses the options. Verify: in the `settings-general` case, capture the window after `open_settings` and find the focused frame's accent outline (as `search.rs`'s `find_bar` finds the find bar): it is wider than the three labels (at least 250 logical pixels), and its right edge is past the "Ask" label.
- [ ] 1.2 Add a footer under the section with a Close button on the right (`self.button("general-close", "Close", …)` emitting `SettingsEvent::Close`), laid out like the Profiles list's footer. Verify: a headless capture under `scripts/with-xvfb` shows the button in the dialog's bottom-right corner, matching the Profiles section's Close button position; hand the click itself to the user as a manual check, since `--click` only targets terminal cells.
- [ ] 1.3 Run `cargo test` and the `settings-` e2e cases under `scripts/with-xvfb`. Verify: all pass.
