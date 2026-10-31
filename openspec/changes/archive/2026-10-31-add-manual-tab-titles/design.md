# Design

## Context

See proposal.md for motivation.

Current tab titles are cached on `Tab.title` in `src/workspace.rs`. `Tab::refresh_title` recomputes the display title from automatic sources and is called from:
- tab creation;
- `TerminalEvent::TitleChanged` for local tabs;
- a one-second workspace poll that also records session state;
- tmux window observation when tmux window names change.

The command palette already runs actions exactly as their keybindings would and supports one-line prompts through `PromptRequest`. Session restore records local tabs in `session::TabState::Local { profile, cwd }` and collapses control-mode tmux tabs to one session entry, not one entry per tmux window.

## Goals / Non-Goals

**Goals:**
- Add a manual-title layer above the existing automatic title calculation.
- Keep automatic title behavior unchanged when no manual title is set.
- Reuse the existing command action and prompt infrastructure.
- Persist manual titles for local restored tabs without changing tmux restore semantics.
- Keep Brindle-local tmux tab titles separate from tmux-owned window names.

**Non-Goals:**
- No new default keybinding.
- No tmux `rename-window` call from `Rename Tab`.
- No persisted per-tmux-window Brindle title state in session restore.
- No changes to config format or settings dialog.

## Decisions

### Store automatic and manual title state separately

Add a manual title field to `Tab`, and keep `Tab.title` as the current displayed title cache. Change title refresh so it computes the automatic title but displays `manual_title` when present.

A small shape keeps the invariant obvious:

```text
Tab
  manual_title: Option<String>
  title: SharedString        # displayed title cache

refresh_title():
  automatic = current OSC/process/profile/tmux title
  displayed = manual_title.unwrap_or(automatic)
```

This preserves the render, dump and window-title callers that already consume `Tab.title`.

### Implement `Rename Tab` at the workspace level

Add a `RenameTab` action to `actions.rs` with config name `rename_tab` and palette label `Rename Tab`. Handle it in `Workspace`, because the command applies to any active tab type and must update the tab wrapper rather than terminal or tmux internals.

The handler opens a `PromptRequest`:
- label: `Rename tab`;
- initial text: current displayed tab title;
- submit: trim the submitted string;
  - if empty, clear `manual_title`;
  - otherwise set `manual_title` to the trimmed string;
  - refresh the displayed title and notify.

No async failure path is needed; return an immediately ready success like existing simple prompt commands.

### Avoid overwriting manual titles during automatic refresh

All existing title refresh callers can continue to call `Tab::refresh_title`. The method should return `false` if the displayed title remains the manual title, even when the automatic title changed behind it. Clearing the manual title then recomputes from the latest automatic source.

This avoids special casing the one-second foreground-process poll, OSC title events, and tmux window-name observer.

### Persist only local manual titles in session restore

Extend local session state with an optional title field:

```toml
[[windows.tabs]]
kind = "local"
profile = "Shell"
cwd = "/srv/www"
title = "server"
```

Data model changes:
- `session::TabState::Local` gains `title: Option<String>` with default and skip-serializing-if-none.
- `session::SnapshotTab::Local` gains the same optional title.
- `RestoredTab::Local` carries the optional title so `Workspace::restore` can apply it to the created tab after launch.

Keep `session::VERSION` unchanged unless existing project convention requires bumping for backward-compatible optional fields. Serde defaults make old state files readable, and older Brindle builds already ignore neither unknown fields nor newer versions reliably enough to justify a version bump for an optional additive field.

### Do not persist tmux manual titles

Control-mode tmux restore records sessions, not individual tmux-window tab entries. Persisting Brindle-local titles for tmux tabs would need a mapping by tmux window id/name/index and new conflict behavior when tmux windows are reordered, renamed, created, or removed while Brindle is closed.

For this change, tmux manual titles are runtime-only. `tmux: Rename Window` remains the durable tmux-owned rename path.

### Verification strategy

Use unit tests where state serialization and title refresh can be exercised without a GUI. Use e2e debug hooks for observable command behavior because prompt flows, command palette rows, `--action`, `--send`, and `--dump-screen-after` already expose tab titles without synthetic desktop input.

## Risks / Trade-offs

- `Tab::refresh_title` currently overwrites `Tab.title`; missing one callsite could make manual titles flicker or get lost. Keeping the override inside `refresh_title` centralizes the rule.
- Persisting local titles changes the session file shape. Making the field optional preserves existing files and keeps files concise when no manual title is set.
- Tmux runtime-only titles may surprise users who expect every manual title to restore. The separate existing `tmux: Rename Window` command gives a durable tmux-owned path without expanding restore semantics now.
- Trimming all submitted input means users cannot intentionally create leading or trailing spaces in a title. This is deliberate and matches the requested behavior.
