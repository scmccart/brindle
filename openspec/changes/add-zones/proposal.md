# Proposal

## Why

Local tabs can hold only one terminal today, so working across shells side by side requires opening separate tabs or running tmux. Giving local tabs a native binary split tree of zones lets users arrange multiple terminals (and future non-terminal zone types) with native GPUI chrome and controls around each zone rather than cell-grid rendering.

## What Changes

- Replace a local tab's single terminal view with a binary split tree of zones (`ZoneTreeView`), where each leaf holds a `ZoneContent` (starting with `Terminal`, structured to add other zone types later).
- Support 50/50 horizontal (`Split Zone Right`, `ctrl-shift-e`) and vertical (`Split Zone Down`, `ctrl-shift-o`) splits of the active zone, spawning a new terminal with the tab's profile and the active zone's working directory and focusing the new zone immediately.
- Reject a new split when the active zone's bounds cannot fit two minimum terminal grids plus the divider and zone header chrome.
- Render native GPUI zone chrome (an always-on header bar per zone with status dot, automatic title, active-zone styling, and Split Right, Split Down, and Close Zone controls) plus 1px dividers only when a tab contains two or more zones; a single-zone tab renders with no zone header bar, identical to today.
- Add `Close Zone` (`ctrl-shift-x` and the header `×` button) and close a zone when its terminal exits, promoting the sibling subtree in the binary tree or closing the tab when the last zone closes (`ctrl-shift-w` continues to close the entire tab).
- Add directional focus actions (`Focus Zone Left/Right/Up/Down`, `alt-left/right/up/down`) resolved against normalized `[0, 1] x [0, 1]` tree geometry.
- Persist and restore the zone split tree, per-zone working directories, and the active zone in `session.toml` while keeping backward-compatible top-level `cwd` fallback.
- Add zone metadata lines (`--- zones` and `--- zone`) to `--dump-screen-after` when a local tab has multiple zones, keeping existing active-terminal dump fields unchanged.

Out of scope: zones inside tmux control-mode tabs, per-zone zoom (`ctrl-shift-z` remains tmux-only), draggable or non-50/50 split ratios, configurable header bar styles, non-terminal zone content types, and moving or breaking zones between tabs.

## Capabilities

### New Capabilities
- `zones`: splitting local tabs into a binary tree of zones, zone header bars and controls, directional focus navigation, minimum split size guards, and zone lifecycle.

### Modified Capabilities
- `tabs`: a local tab's title, working directory inheritance, bell indicator, and program status indicator derive from its zone tree (active zone for automatic title and working directory; all zones for bells and most pressing status).
- `session-restore`: local tab state records and restores the binary zone tree, per-zone working directories, and active zone alongside existing tab metadata.
- `command-palette`: zone commands (`Split Zone Right`, `Split Zone Down`, `Close Zone`, `Focus Zone Left/Right/Up/Down`) are listed when a local tab has focus and omitted in tmux control-mode tabs.
- `command-line`: `--list-actions` and `--action` include the zone actions, and `--dump-screen-after` emits additive zone layout lines when a local tab has two or more zones.

## Impact

- **UI and workspace:** `src/workspace.rs` (`TabContent` and local tab event routing), new `src/zone_view.rs` (or `src/zones.rs`) owning the binary tree, normalized geometry, native GPUI headers, dividers, and focus tracking.
- **Actions and bindings:** `src/actions.rs` adds `SplitZoneRight`, `SplitZoneDown`, `CloseZone`, `FocusZoneLeft`, `FocusZoneRight`, `FocusZoneUp`, and `FocusZoneDown` in the `ZoneTree` key context.
- **Session persistence:** `src/session.rs` extends `TabState::Local` / `SnapshotTab::Local` / `RestoredTab::Local` with optional zone tree serialization compatible with format version 1.
- **Verification and docs:** `tests/e2e/`, unit tests for tree mutations, normalized directional focus, and session round-tripping, plus `README.md` and `CLAUDE.md`.
