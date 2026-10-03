# Design

## Context

See proposal.md for the problem. The relevant code paths today:

- **Window list.** `TmuxSession::list_windows` asks for `window_id, window_index, window_active, window_zoomed_flag, window_visible_layout, window_name`. It doesn't ask for the active pane, so a new `TmuxWindow` starts with `active_pane: None`. Only `%window-pane-changed` sets it later.
- **Guessed active pane.** `active_pane_of` falls back to the layout's first pane. `TmuxWindowView::sync` makes that pane's view the active one, and its `on_focus_in` handler calls `TmuxSession::select_pane`. Because `active_pane` is still `None`, that sends `select-pane`.
- **Window activation order.** `on_window_list` emits `WindowAdded` for every new window, and only then sets `active_window` and emits `WindowActivated`. `Workspace::on_tmux_event(WindowAdded)` inserts the tab through `insert_tab`, which calls `activate`. For a tmux tab, `activate` calls `select_window`, and that sends `select-window` while `active_window` is still `None` or stale.

## Goals / Non-Goals

**Goals:**
- Brindle adopts tmux's current window and active panes on attach, and never writes them back as a side effect.
- Only actions the user takes in Brindle (clicking a tab or pane, keybindings, commands) select windows and panes in tmux.

**Non-Goals:**
- Preserving which Brindle tab was active before a tmux tab was added, beyond what tmux's current window implies.
- Changing how `%window-pane-changed` and `%session-window-changed` are mirrored. They already follow tmux.

## Decisions

### 1. The window list carries the active pane

The `list-windows` format becomes:

```
#{window_id}\t#{window_index}\t#{window_active}\t#{window_zoomed_flag}\t#{pane_id}\t#{window_visible_layout}\t#{window_name}
```

In a `list-windows` format, pane variables refer to each window's active pane. This was checked on tmux 3.6: with `%1` selected in a three-pane window, `#{pane_id}` gives `%1`. The name stays last, so `splitn(7, '\t')` keeps a name that contains tabs intact.

Line parsing moves into a pure `parse_window_line(&str) -> Option<WindowLine>` so it can be unit-tested.

### 2. The listed active pane only fills in what Brindle doesn't know

`on_window_list` sets `active_pane` from the list only when it is `None`, which in practice means the window is new.

`%window-pane-changed` stays the authority after that. A window list can be in flight while the user clicks a pane. If its reply overwrote `active_pane`, focus would briefly jump back to the old pane until the `%window-pane-changed` for the click arrived.

*Alternative:* always overwrite from the list. That was rejected because of the race above. tmux reports every pane change in the attached session as `%window-pane-changed`, so nothing is lost.

### 3. `select_pane` only sends when it changes tmux's known state

`TmuxSession::select_pane(pane)` behaves as follows:
- **Known active pane, same as `pane`:** nothing to do. This is the attach case once decision 1 is in place: focusing tmux's active pane is a no-op.
- **Known active pane, different from `pane`:** set it and send `select-pane`, as today. This is a user action.
- **Active pane unknown (`None`):** adopt `pane` locally without sending. This only happens if the list line lacked a pane id, and it keeps the first-pane guess from being written back to tmux.

`TmuxWindowView` needs no change: its focus-in handler keeps calling `select_pane`.

### 4. The current window is recorded before tabs are announced

`on_window_list`:
1. works out the reported current window;
2. notes whether it differs from `active_window`;
3. sets `active_window`;
4. emits `WindowClosed` and `WindowAdded`, then `WindowActivated` if the current window changed.

Because `active_window` is already right while tabs are being added, the `select_window` that runs when the current window's tab is activated is a no-op.

### 5. `Workspace` adds tmux tabs without activating them

`on_tmux_event(WindowAdded)` inserts the tab with a new `insert_tab_inactive(tab, index)`. That function inserts the tab, shifts `self.active` when the insertion lands at or before it, and doesn't call `activate`. `WindowActivated` activates the right tab, as it does today.

- **First attach in a new Brindle window:** there are no tabs yet. The first inserted tab is index 0 and `self.active` is 0, so it shows until `WindowActivated`, which arrives from the same `on_window_list` call, activates and focuses the current window's tab.
- **Foreground new window** (ctrl-shift-t, break pane): tmux makes the window current, so the list reports it and `WindowActivated` follows, or `%session-window-changed` does. The tab comes to the front.
- **Background new window** (`new-window -d` from another client): there is no `WindowActivated`, so the tab stays in the background.

*Alternative:* keep activating on add and suppress `select_window` with a flag while a list is being applied. That was rejected because it still flashes tabs and pulls background windows to the front.

## Risks / Trade-offs

- **[A list line without a pane id]** For example, an unexpected format from an older tmux. → Mitigation: parsing treats the pane field as optional, and decision 3 keeps the first-pane guess local.
- **[A tmux tab is added while the user is in a local tab of the same Brindle window, and tmux's current window changes]** The workspace switches to the tmux tab, as `WindowActivated` already does today. → Accepted: that's existing behaviour, not something this change introduces.
- **[The first tab of a new window is briefly shown before `WindowActivated`]** → Mitigation: both events come from one `on_window_list` call within one update, so no frame is drawn in between.

## Migration Plan

None. This is behaviour-only and needs no config changes.
