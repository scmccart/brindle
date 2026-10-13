# Design

## Context

- **Where the fallback lives:** `TmuxSession::on_detached` marks every pane's terminal exited and emits `TmuxEvent::Detached(reason)`, or `Failed` when the session never attached. `Workspace`'s `Detached` handler (`workspace.rs`) opens a fallback tab when the session's tabs were all the window had, then removes those tabs. When the last tab goes, `remove_tab` calls `SessionStore::forget` and `window.remove_window()`, and closing the last window quits.
- **What tmux tells us:** tmux 3.6 sends a bare `%exit` to a control client, with no reason, whether the client detached, its session was destroyed by its last shell exiting, or the session was killed. When the session ended, a `%sessions-changed` comes first. This was checked against a throwaway server. Brindle sets `TmuxSession::released` when it asks to detach itself (ctrl-shift-d, or quitting). Since the setting applies to detach and to the session ending alike, none of this needs to be told apart, except quitting.
- **Quit:** `TmuxSession` registers an `on_app_quit` hook that calls `release()` and waits up to 50 ms for the write to flush. Session restore's own quit hook records the open windows. If `%exit` is processed during that wait, today's handler mutates the window while it is being recorded.
- **Palette:** `Palette::list(rows, placeholder, theme, cx)` emits `Confirmed(row)` or `Dismissed`. It emits `Dismissed` on escape and on a mouse-down outside it. `Workspace` keeps a single `overlay` and a `PaletteKind` (`NewTab`, `Commands`, `Prompt`) that decides what a confirmation does. Opening a palette closes the other overlay.
- **Settings dialog:**
  - `Section { Theme, Profiles }`, `SECTIONS`, and `section: Section::Theme` at construction.
  - `select_section` guards dirty editors and reloads profiles on entry.
  - The choice field (`choice_box`, the `SettingsChoice` key context, the `SettingsChoiceNext`/`Prev` actions) is implemented on the profiles view for its `Field`s.
  - `SettingsDialog::save` wraps `config_edit::save`; profile saves then reload the config directly through `reload_config_renaming`.
  - `describe()` produces the `--- settings section=…` dump line that the e2e cases match.
- **Config:** a `serde(default, deny_unknown_fields)` struct. Unknown enum values already produce the error banner. `config_edit::set_value` sets a top-level key in place.

## Goals / Non-Goals

**Goals:**
- One decision point, in `Workspace`, for "a session left and the window has nothing else". It is driven by the setting and is identical for detach, exit, kill and crash.
- The ask prompt reuses the palette rather than adding a new overlay type.
- Quitting never changes what session restore records.

**Non-Goals:**
- Telling detach apart from the session ending, by `%sessions-changed` or a `has-session` probe. The user chose one rule for both.
- A General section framework beyond this one setting.

## Decisions

### 1. Config: `tmux_session_end` with `snake_case` values

`TmuxSessionEnd { CloseWindow (default), NewTab, Ask }` uses `#[serde(rename_all = "snake_case")]`, so the values are `close_window`, `new_tab` and `ask`. Existing config enums use `lowercase` and `kebab-case`, but their values are all single words. The multi-word values a user already writes are action names in `[keybindings]`, which are snake_case, and snake_case reads as a sibling of the key. The default config gets a commented line beside `restore_session`, and the tmux profile's comment ("Detaching (ctrl-shift-d) keeps the session running") gains a pointer to the setting.

*Alternative:* a boolean `tmux_fallback_tab`. It was rejected because it has no room for `ask`.

### 2. Decision point: the `Detached` handler, reading the setting at event time

The handler first drops the session's restore bookkeeping (as now). When the window has tabs other than this session's, it removes the session's tabs, as now. Otherwise it reads `tmux_session_end` from `Settings` and acts on it:

- `CloseWindow`: remove the session's tabs. The last removal closes the window through `remove_tab`, which also forgets it in the session store, exactly as a local tab whose shell exits.
- `NewTab`: today's profile rule, then remove the tabs. The launch passes `cwd: None` rather than letting `launch` inherit `active_cwd`. That keeps today's behaviour, and an ended tmux tab's directory isn't meaningful anyway.
- `Ask`: keep the tabs and open the palette (decision 3).

`Failed` is unchanged.

### 3. The ask prompt: a `PaletteKind::SessionEnded`

- **Rows:** "Close window" first, then one row per profile in config order, labelled "New tab: <name>", with the same detail as the new-tab palette's rows. The placeholder names the session: `tmux session "<name>" ended`. `PaletteKind::SessionEnded { session }` keeps the `Entity<TmuxSession>`, so the confirm handler can find that session's tabs with `shows_tmux`.
- **Confirm:**
  - Row 0 removes the session's tabs, which closes the window.
  - Row n launches profile n−1 with `cwd: None`, then removes the session's tabs.
- **Dismiss:** `Dismissed` today means "close the overlay". For this kind it means close the window.
- **Click outside:** it must not dismiss, because closing the window on a stray click would be hostile. The palette gets a flag at construction, set only for this kind, that skips the `on_mouse_down_out` dismissal.
- **Overlays:** opening the prompt goes through the usual overlay path, so an open settings dialog or palette is cancelled first, which the "one overlay at a time" rule already requires.
- **Choosing a control-mode profile:** its session attaches asynchronously, so its tabs don't exist yet when the choice is made. Removing the ended tabs at once would empty the window and close it. Instead, `Workspace::replacing` maps the new session to the ended one, and the ended tabs are removed when the new session adds its first tab, fails, or detaches.
- **Closing an ended tab:** `close_tab_at` normally sends `kill-window` and waits for tmux. For a session that has already detached (`TmuxSession::is_detached`), it removes the tab directly. Otherwise an ended tab left behind, after another overlay replaced the prompt, could never be closed.
- **Other events while the prompt is open:** the panes are ended and the session sends nothing more. If the user closes the ended tab while the prompt is open, normal tab closing runs and the window closes. The palette goes with it. No extra handling is needed.

*Alternative:* removing the tabs at once and showing the prompt over an empty window. It was rejected because `remove_tab` closes a window with no tabs, and an empty tab strip would need a new state.

### 4. Quitting: suppress the event in `TmuxSession`

The `on_app_quit` hook sets a `quitting` flag before `release()`. `on_detached` still marks the panes exited but emits nothing while `quitting` is set. Doing this in the session keeps `Workspace` free of quit state, and covers `%exit`, EOF and errors alike.

*Alternative:* a global "app is quitting" check in `Workspace`. It was rejected because GPUI has no such query, and the session already owns the quit hook.

### 5. Settings dialog: a General section with one choice field

- **Sections:**
  - `Section::General` is added, and `SECTIONS` becomes `[General, Profiles, Theme]`.
  - The dialog is constructed with `section: General`. The theme list's selection is set to the active theme when the dialog is constructed, and `select_section` resets it whenever Theme is left, so it is the active theme whenever Theme is shown.
  - The `match self.section` arms gain `General`. It is never dirty, and `confirm` and `cancel` both close the dialog there.
- **Model:** `SettingsDialog` keeps the value it read when opened (`Settings` global at open, the same source as `active` for the theme). `step_session_end(forward)` writes it through `self.save(|doc| config_edit::set_tmux_session_end(doc, value))`. On success it updates the model and returns an outcome that makes the view call `reload_config`, as a profile save does. On error the value is not changed and `error` is shown, through the existing error path.
- **View:** a General pane renders one labelled choice field, with a short explanation under it. It uses the same `SettingsChoice` key context and `SettingsChoiceNext`/`Prev` actions as the profile editor, and gets focus when the section is shown, so left and right work at once.
- **Shared choice-field rendering:** the box styling (`field_box`) and the context wiring are hoisted from the profiles view into a small shared helper in `settings_dialog.rs`. The profiles view then calls it too, which avoids two copies drifting. The profiles' `Field`-keyed focus handles stay where they are.
- **Dump:** `describe()` for General prints `--- settings section=General tmux_session_end="close_window" error=None`.

### 6. `config_edit::set_tmux_session_end`

It is a thin `set_value` call like `set_global_theme`, writing the snake_case string. A unit test checks that the key is set in `DEFAULT_CONFIG` text with every other line unchanged. `toml_edit` renders a document's top-level values before its tables, so a new key can't land inside `[font]`. The test pins that.

### 7. Verification

- **Unit tests:**
  - config parsing of each value, the default, and an unknown value;
  - `set_tmux_session_end` on the default config and on a malformed one;
  - the dialog model: the section order, opening on General, stepping writes the value, a failed write keeps it.
- **e2e (`tmux` profile on a throwaway server; the trigger is `tmux kill-session` from the test or `--send 'exit\r'`):**
  - `close_window`: there is no dump to read, because the only window closes and Brindle quits. The case asserts that the process exits on its own, with status 0, well before its `--dump-screen-after` deadline.
  - `new_tab`: the dump shows a single local tab, and `tmux ls` on the throwaway server shows the session gone.
  - `ask`: the dump shows the `--- palette` line with "Close window" selected and the tmux tab still listed. A second run sends `picker_confirm` and expects the process to exit.
  - detach with `close_window`: `--action tmux_detach`, the process exits, and `has-session` succeeds afterwards.
  - settings: `open_settings`, `settings_choice_next`, then the dump shows `section=General tmux_session_end="new_tab"`, and the config file contains the key.
- **Existing settings cases:**
  - Theme cases add one `settings_section_prev` after `open_settings`, because Theme is now last and `next_section` wraps around.
  - Profile cases keep their single `settings_section_next`, because Profiles is second.

## Risks / Trade-offs

- [The default closes the window on ctrl-shift-d, and quits Brindle when it is the last window] → It's what the user chose. The proposal marks it, the README's ctrl-shift-d row says it, and `new_tab` gives the old behaviour.
- [tmux 3.6 may crash when a session with subscriptions ends from inside, which is the `exit` path] → It's pre-existing. The tabs still close, because EOF is handled like `%exit`. Only the server's other sessions are at risk, as before.
- [`%exit` processed during the 50 ms quit wait] → Decision 4.
- [tmux sends `%window-close` for the last window before `%exit`, and the `WindowClosed` handler empties the window before the setting is consulted] → No `%window-close` appeared before `%exit` when the last shell exited in the transcript, and ctrl-shift-w already waits for tmux. To be safe, `WindowClosed` leaves the session's last tab in place when nothing else is in the window, and the `Detached` that follows handles it. The `session-end-close-tab` e2e case pins this.
- [A non-closing palette is a special case] → It's limited to one construction flag, and escape still always works.
- [Existing e2e settings cases break on the new section order] → They are updated in the same change, and the whole settings filter is run.
