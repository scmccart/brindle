# Proposal

> **Skeleton.** This holds the context from the exploration that produced `add-brindle-ui`. Flesh it out (specs, design, tasks) before implementing. **Depends on:** `add-brindle-ui`.

## Why

`brindle-ui` draws only from its `UiTheme` tokens, and ships only shadcn's neutral sets. For Brindle's chrome to match the terminal, as it does today through `theme.rs`'s derived colours, Brindle has to produce tokens from the active terminal theme and keep them current. The migrations (`ui-settings-dialog`, `ui-command-palette`, `ui-chrome`) all need this first.

## What Changes

- Add a Brindle-side adapter: terminal `Theme` → `brindle_ui::Tokens`. It replaces the ad hoc derivations (`chrome_background`, `chrome_border`, `muted_foreground`, `hover_background`, `accent()` → `ansi[4]`) as the single place chrome colours come from. Starting mapping, to refine:
  - `background` ← the chrome background;
  - `foreground` ← the theme foreground;
  - `primary` ← the accent;
  - `destructive` ← `ansi[1]`;
  - `muted`/`accent` ← background mixes;
  - `border`/`input` ← `chrome_border`;
  - `ring` ← the accent;
  - `popover`/`card` ← the chrome background, slightly lifted.
- Set `UiTheme` at startup, on config reload, and during the settings dialog's live theme preview, so components follow the same theme changes the terminal does.
- Check contrast: foreground/background pairs must stay legible for light and dark built-in themes. A unit test runs every built-in theme through the adapter and checks minimum contrast.

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `configuration` (tentative): only if themes gain optional UI token overrides (see open questions).
- `settings-dialog` (tentative): live preview also re-themes the UI chrome. It may already be covered by "Live theme preview".

## Impact

- `src/theme.rs` (the adapter, and removal of the derivations once nothing uses them), `src/settings.rs` (setting `UiTheme` on build and reload), `src/settings_dialog.rs` (preview).
- No visible change until a surface is migrated, unless the adapter replaces the derivations' callers directly. Decide which when fleshing out.

## Open questions (resolve when fleshing out)

- Should themes get optional `[themes.<name>.ui]` token overrides, or is the mapping fixed?
- Should each tab's profile theme re-theme the chrome when its tab is active, or is it always the window default theme? It is the default today.
- Does UI zoom follow terminal zoom (`rem_size`), or stay fixed? This is also relevant to `ui-chrome`.
