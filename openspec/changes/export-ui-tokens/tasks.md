## 1. Export

- [x] 1.1 `theme::ui_tokens_css`: every built-in theme's tokens and terminal colours as CSS, `:root` before `.dark`, with unit tests
- [x] 1.2 `--export-ui-tokens` in `main.rs` and its help line

## 2. Release

- [x] 2.1 `build-linux.yml` writes `brindle-ui-tokens.css` into the Linux artifact
- [x] 2.2 `release.yml` attaches it and lists it in `SHA256SUMS`
