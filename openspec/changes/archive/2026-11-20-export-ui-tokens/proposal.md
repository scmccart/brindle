# Proposal

## Why

Brindle's website (a separate repository, built with shadcn/ui) should look like the app's chrome. The chrome's colours come from `Theme::ui_tokens`, which already uses shadcn/ui's token names. Copying them into the site by hand would drift with every theme change, so Brindle should print them, and each release should carry them.

## What Changes

- `brindle --export-ui-tokens` prints every built-in theme's UI tokens as CSS custom properties under shadcn/ui's names (`--background`, `--primary`, …, `--radius`), plus the theme's terminal colours (`--terminal-background`, `--terminal-ansi-0` … `-15`, …), and exits without opening a window. Each theme is a `[data-theme="NAME"]` block; `brindle-light` is also `:root` and `brindle-dark` also `.dark`, after it.
- The Linux build writes the output to `brindle-ui-tokens.css`, and the release attaches it beside the archives and lists it in `SHA256SUMS`.

## Impact

- `src/theme.rs` (`ui_tokens_css`, pure, with tests), `src/main.rs` (the option and its help line).
- `.github/workflows/build-linux.yml`, `.github/workflows/release.yml`.
- Spec: `command-line` (Informational options).
