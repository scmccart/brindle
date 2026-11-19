# Proposal

## Why

Five small rough edges, collected into one change. The settings dialog is reachable only by a key or the command palette, though the strip's `⌄` menu is the obvious place to look for it. The Theme list doesn't say which themes are built in, so the disabled Delete (and an Edit that quietly makes a copy) seems arbitrary. The dialog's close `X` only steps back one screen, so in an editor it takes two presses to close what looks like a window's close button. Users coming from PuTTY or Windows Terminal want select-to-copy with right-click paste; Brindle has half of it (`copy_on_select`), and that half doesn't follow a config reload. And the e2e case `local-server-sink-panic` fails under `cargo test --release` because its fault switch exists only in debug builds: the harness can't tell a missing switch from a broken feature, and three other cases share the trap.

## What Changes

- **Profile menu → Settings.** The strip's `⌄` menu ends with a separator and a "Settings" item (with its key), which opens the settings dialog as ctrl-shift-, does.
- **Built-in themes are marked and read-only.** Each built-in row in the Theme list shows a lock icon, whose tooltip reads "Built in: New makes an editable copy", where custom rows show "custom". `brindle-ui` gains `Icon::Lock`, lucide's `lock`. With a built-in selected, Edit and Delete are unavailable, and `e` or `d` says why, pointing at New, which still copies the selected theme. Today Edit on a built-in silently opens a copy, which is what New does.
- **The close `X` closes the dialog.** From any view it closes the whole dialog as ctrl-shift-, already does: previews end, an editor's unsaved edits are dropped, nothing is written. Escape still steps back one screen (discard in an editor, keep in a delete question, cancel in a list).
- **Right-click paste.** A new config key `right_click` (`"none"`, the default, or `"paste"`): with `"paste"`, a right press in a terminal with no text selected pastes the clipboard. While text is selected, a right press does nothing (neither paste nor copy), so a stray right-click can't paste over a selection. With `copy_on_select` on, the selection is already in the clipboard. A program that reports the mouse still gets the right press. Shift-right-click goes to Brindle instead, as shift-drag does, and follows the same selection rule. `copy_on_select` and `right_click` are read when they're used, so a reload or the settings dialog changes open tabs too.
- **Clipboard settings section.** The settings dialog gains a fourth section, "Clipboard", with two switches, "Copy on select" and "Right-click pastes", applied like the Fonts section's fields.
- **Debug hook.** `--right-click [MODS-]COL,ROW` presses and releases the right button on a cell, in-process like `--click`.
- **Fault switches are listed, and cases that need one say so.** `brindle --list-fault-switches` and `brindle-remote --list-fault-switches` print the `BRINDLE_TEST_*` fault switches the build honours (none in a release build) and exit. Each e2e case declares the switches it needs. The harness asks each binary once, and a case whose binary lacks a switch is skipped with the reason, before any window opens. `BRINDLE_E2E_SWITCHES_REQUIRED=1` (set in CI) makes that a failure. The real-process tests that use the helper's switches skip through the existing `skip(Capability, …)` with a new `fault-switches` capability.

No breaking changes: `right_click` defaults to today's behaviour, and the new flags are additive.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `profiles`: the profile menu ends with a separator and a Settings item.
- `settings-dialog`: the profile menu opens the dialog; built-in themes are marked, and Edit and Delete are unavailable on them; the close button closes the dialog from any view; a Clipboard section edits `copy_on_select` and `right_click`.
- `terminal-emulation`: right-click paste, how it works alongside mouse reporting and shift, and `copy_on_select` applying to open tabs after a reload.
- `configuration`: `right_click` joins the behaviour settings, and a reload applies both clipboard settings to open tabs.
- `command-line`: `--list-fault-switches` (informational) and the `--right-click` debug step, checked before the run.

## Impact

- `src/workspace.rs`: the profile menu's entries and its `MenuEvent` handler, through a closed choice decoded from the item key.
- `ui/src/icon.rs`, `ui/README.md`: `Icon::Lock`.
- `src/settings_dialog.rs`, `settings_dialog/themes.rs`, a new `settings_dialog/clipboard.rs`, `src/config_edit.rs`: the built-in lock and its tooltip, the button and key rules, the close button's step, and the Clipboard section's model and edit.
- `src/config.rs`, `assets/default-config.toml`: `right_click`.
- `src/terminal/mouse.rs`, `src/terminal_view.rs`, `src/terminal/mod.rs`, `src/main.rs`: the pure press decision, the right-press handling, `copy_on_select` read live, `--right-click`, `--list-fault-switches`.
- A new `src/test_switches.rs`, plus `src/terminal/server.rs` and `src/main.rs` naming their switches from it. `remote-agent/src/server/fault.rs` and `remote-agent/src/main.rs`: the helper's list and flag.
- `tests/e2e/` (`main.rs`, a new `switches.rs`, `brindle.rs`, `cases/*`), `remote-agent/tests/support/`, `remote-agent/tests/{holder,core}.rs`: needs, the probe, skips and the guard.
- `.github/workflows/ci.yml` and `build-windows.yml`: the new require variables.
- Docs: `README.md`, `CLAUDE.md`, `docs/architecture/testing-architecture.md` (a new rule) and `process-architecture.md` (`R20`).
- No wire, protocol, state-file or terminal-server change.
