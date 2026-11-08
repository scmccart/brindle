# Design

## Context

`src/theme.rs` derives chrome colours ad hoc (`chrome_background`, `chrome_border`, `muted_foreground`, `hover_background`, `accent()`), used in about 47 places across `workspace.rs`, `settings_dialog*`, `picker.rs`, `find_bar.rs`, `zone_view.rs`, `tmux_view.rs`. `brindle-ui` reads `Tokens` from the `UiTheme` global and must not depend on `brindle` (see proposal.md). Previews reach the window through `SettingsEvent::Preview` → `Workspace::set_theme_preview`; reloads through `Workspace::apply_config`. Nothing reads `UiTheme` yet.

## Goals / Non-Goals

**Goals:** one function from `Theme` to `Tokens`; the global kept current at startup, reload and preview; contrast guaranteed for built-ins.

**Non-Goals:** migrating surfaces to components (later changes); removing the old derivations while callers remain.

## Decisions

- **Adapter lives in `src/theme.rs`** as `Theme::ui_tokens(&self) -> Tokens`, using `Color::mix`/`luminance`. Alternative: put it in `brindle-ui`; rejected, it must not know `Theme`.
- **Fixed mapping, no `[themes.<name>.ui]` overrides.** Keeps `configuration` unchanged; overrides can be added later additively.
- **Chrome follows the global theme only**, as it does today (tab themes affect terminals). Preview replaces the global theme while active.
- **UI zoom stays fixed**; terminal zoom does not touch `rem_size`. Revisit in `ui-chrome`.
- **Mapping:** background ← `chrome_background`; foreground ← theme fg; card/popover ← background mixed 0.04 toward fg; primary/ring ← accent; primary_foreground ← black or white by accent luminance; secondary/muted/accent ← background mixed toward fg (0.08/0.08/0.12); *_foreground ← fg (muted_foreground ← `muted_foreground()`); destructive ← `ansi[1]` with foreground picked by luminance; border/input ← `chrome_border`; radius ← default; `dark` ← `is_dark()`.
- **Update points:** one helper `settings::apply_ui_theme(theme, cx)` calling `UiTheme::set`. Called after `Settings` is set in `main.rs` (both sites), after reload, and from `set_theme_preview` (with `None` restoring the config theme).
- **Contrast test:** pure unit test over `BUILTIN_THEMES` using WCAG relative luminance. Where a pair fails, the adapter nudges the foreground toward black/white rather than the test being loosened.

## Risks / Trade-offs

- [Derived accents may lack contrast against some themes] → foreground picked by luminance; test covers all built-ins.
- [`refresh_windows` per preview step] → already a full re-render per `cx.notify()`; acceptable.
- [Two colour sources until migrations land] → old derivations stay and are built on the same primitives, so they cannot drift visibly.
