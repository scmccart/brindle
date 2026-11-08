# Proposal

## Why

Each of Brindle's overlays draws its own chrome. The settings dialog has its own `button()`, `field_box()`, `checkbox()` and choice rows, the palette and the find bar draw theirs again, and all of them pick colours from the terminal theme at each call site. Focus handling, dismissal and keyboard navigation are reimplemented per overlay. A shared component crate, modelled on shadcn/ui's tokens, variants and component set, lets later changes rebuild those surfaces on one tested set of primitives. Every maintained GPUI component library has moved to gpui-pre or a fork and left crates.io gpui 0.2.2, and Zed's `ui` crate is GPL, so the crate is Brindle's own.

## What Changes

- Add a workspace crate `brindle-ui` (`ui/`), depending only on `gpui`. It never depends on `brindle`, `brindle-term-core` or `alacritty_terminal`.
- **Tokens:** the shadcn token set as data (`Tokens`: background/foreground, card, popover, primary, secondary, muted, accent, destructive, border, input, ring, radius), held in a `UiTheme` GPUI global. The crate ships neutral light and dark token sets for its gallery. Mapping a terminal theme to tokens is left to the app (change `ui-tokens-from-theme`).
- **Primitives:**
  - layer stack: Escape and outside presses go to the topmost overlay only;
  - focus scope: Tab cycles inside an overlay, and focus returns to where it was on close;
  - anchored placement: side and align, flipping at the window edge;
  - roving list navigation with typeahead;
  - focus-visible tracking (keyboard vs pointer);
  - variant/size styling as Rust enums, the role `cva` plays in shadcn.
- **Components:**
  - Button: variants default, destructive, outline, secondary, ghost, link; sizes default, sm, lg, icon.
  - Input: a styled field around `TextInput`.
  - Checkbox and Switch.
  - Dialog.
  - Popover.
  - Select.
  - Command: a filterable list with groups, usable inline, in a Dialog or in a Popover.
- Move `TextInput` (`src/text_input.rs`) into `brindle-ui` unchanged in behaviour, together with its action types and default bindings. Brindle imports it from the crate, and its `ACTIONS` table keeps the config names (`text_input_left`, …).
- Add a gallery example (`cargo run -p brindle-ui --example gallery`) that shows every component in every variant, in light and dark.
- Out of scope:
  - migrating any Brindle surface to the components (changes `ui-settings-dialog`, `ui-command-palette`, `ui-chrome`);
  - the terminal-theme adapter (`ui-tokens-from-theme`);
  - exit animations;
  - Tabs, Tooltip, DropdownMenu and the rest of shadcn's inventory;
  - accessibility (GPUI 0.2.2 has no accessibility layer).

## Capabilities

### New Capabilities
- `ui-components`: Brindle's shared UI component set. Covers its tokens and theming, the behaviour of its overlay primitives (layering, dismissal, focus containment and return, placement, list navigation, focus rings), and the behaviour of each component.

### Modified Capabilities
None. `text-input` describes behaviour only, and moving `TextInput` into the crate changes none of it.

## Impact

- **New crate:** `ui/` (`brindle-ui`, version equal to the other workspace crates). It joins workspace `members` and `default-members`, and `Cargo.lock` gains only its own entry. No new third-party dependencies.
- **Brindle:**
  - `src/text_input.rs` moves to `ui/src/text_input.rs`.
  - `src/actions.rs` takes the TextInput action types and their default bindings from the crate.
  - Imports change in `picker.rs`, `find_bar.rs`, `settings_dialog.rs`, `settings_dialog/profiles.rs`, `workspace.rs` and `tmux/mod.rs`.
  - No visible change.
- **Action names:** the GPUI names of the moved actions change from `brindle::TextInput*` to `brindle_ui::TextInput*`. Config, `--action` and `--list-actions` use the `ACTIONS` table's names, which are unchanged.
- **CI and release:** the "workspace crates share one version" check covers the new crate. release-please's `rust` release type bumps it with the rest.
- **Docs:** CLAUDE.md (architecture section, crate count) and README.md (build section, if it lists crates).
