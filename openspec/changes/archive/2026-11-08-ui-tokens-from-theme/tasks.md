## 1. Adapter

- [x] 1.1 Add `Theme::ui_tokens` in `src/theme.rs` per the design's mapping, with a luminance-based foreground picker
- [x] 1.2 Unit tests: accent → primary/ring, `dark` hint, destructive from `ansi[1]`
- [x] 1.3 Contrast test over `BUILTIN_THEMES` for the pairs in the spec; adjust the mapping until all pass

## 2. Keeping the tokens current

- [x] 2.1 Add `apply_ui_theme` and call it at both startup sites in `src/main.rs`
- [x] 2.2 Call it on config reload (where `apply_config` runs)
- [x] 2.3 Call it from `Workspace::set_theme_preview`, restoring the config theme on `None`; per-tab profile themes must not affect it

## 3. Verify

- [x] 3.1 `cargo build` and `cargo test --workspace` pass with no new warnings
- [x] 3.2 Unit test (headless GPUI app) that preview then cancel restores the tokens
