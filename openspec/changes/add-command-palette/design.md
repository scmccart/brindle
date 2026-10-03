# Design

## Context

- **Profile picker.** `src/picker.rs` holds `ProfilePicker`, a filterable list hard-wired to `config.profiles`. It uses the key context `ProfilePicker` and the `Picker*` navigation actions. `Workspace` owns it, draws it as an overlay below the tab strip, and subscribes to `PickerEvent`.
- **Action handlers by context.** Handlers live at three levels: `Workspace`, `TmuxWindow` (only in control-mode tabs) and `Terminal`. `Quit`, `ReloadConfig` and `NewWindow` are global (`cx.on_action` in `main.rs`). `TmuxDetach` is currently handled by `Workspace`, so it is reachable from every tab.
- **The `ACTIONS` table.** `src/actions.rs` maps config names to constructors. It drives config lookup and `--list-actions`.
- **tmux replies.** `TmuxSession` sends commands through `Io::send`, which queues one `Pending` per command. Replies are matched first in, first out. Every command Brindle sends today expects exactly one `%begin…%end/%error` block. tmux emits one block **per command**, so a user line like `a ; b` produces two blocks.
- **Unhandled keybindings fall through.** In GPUI 0.2.2, a key whose bound action nobody handles falls through to `on_key_down`. `TerminalView` would encode ctrl-shift-d as `^D`.

## Goals / Non-Goals

**Goals:**
- One list widget behind both palettes, with a prompt mode.
- The commands listed come from what is reachable from the focused tab, not from hand-kept rules per tab type.
- A prompted command behaves the same whether it starts from the palette or from a keybinding.
- A free-form tmux line can never desynchronize reply matching.

**Non-Goals:**
- Showing output from tmux commands.
- Ranking by recent use, or fuzzy scoring beyond the existing subsequence match.
- Listing actions that take a parameter (`activate_tab_N`, `new_tab_with_profile_N`) as one entry per value.

## Decisions

### 1. A shared `Palette` widget replaces `ProfilePicker`

`picker.rs` becomes a generic palette with two modes:
- **List:** rows of `label`, optional `detail` and optional `shortcut`. It filters with the existing `fuzzy_match` and emits `Confirmed(row index)` or `Dismissed`.
- **Prompt:** a label, an editable single-line value and an optional error line. Submitting returns a future of `Result<(), String>`. The palette shows a busy state while it waits, closes on `Ok`, shows the message on `Err`, and closes if the future is cancelled.

`Workspace` keeps a single `palette: Option<(Entity<Palette>, PaletteKind, Subscription)>`, where `PaletteKind` is `NewTab`, `Commands` or `Prompt` (a prompt opened by a command, from the list or a keybinding). Opening either kind replaces whatever is open. Opening the kind that is already open closes it.

The key context is renamed from `ProfilePicker` to `Palette`. User bindings always go into the `Terminal` context, so the rename doesn't affect users. Backspace and character input stay in `key_down`. Pasting (ctrl-shift-v, while the palette has focus) is added for the prompt, with line breaks replaced by spaces.

*Alternative:* keep two separate widgets. That was rejected because the list, filter, navigation and overlay code would be duplicated almost line for line.

### 2. Command entries come from `ACTIONS` plus a label column

`ACTIONS` becomes `(name, label: Option<&str>, constructor)`. Entries with a label can appear in the palette. The following have no label and are never listed:
- `none`;
- the `Picker*`/`Palette*` navigation actions, which aren't in the table anyway;
- `open_command_palette` itself.

Labels for tmux actions start with `tmux: `. `--list-actions` and config lookup are unchanged. A unit test asserts that every labelled action resolves and that labels are unique.

*Alternative:* a separate table for the palette. That was rejected because the two tables would drift apart.

### 3. Availability is snapshotted before the palette takes focus

`open_command_palette` runs while the tab still has focus. At that moment it:
1. builds each labelled `ACTIONS` entry;
2. keeps an entry when `window.is_action_available(&*action, cx)` is true, or when its `TypeId` is among those returned by `window.available_actions(cx)`.

   Both checks are needed. `is_action_available` walks only the dispatch tree, so it misses global listeners such as `Quit` and `ReloadConfig`. `available_actions` includes those, but it constructs actions through the registry, which can't build `no_json` actions, so it silently drops `ActivateTab` (`last_tab`). `App::is_action_available` can't be used from inside a window update, because it re-enters the window;
3. looks up each entry's shortcut with `window.highest_precedence_binding_for_action_in(action, &tab_focus)`. The tab's context stack includes `Terminal`, so user overrides win.

It stores the rows together with the tab's `FocusHandle`. Rows are built once per open and never during `render`.

Control-mode tabs get tmux commands with no tab-type checks, because their handlers exist only on the `TmuxWindow` path.

### 4. Running a command dispatches it back to the tab

On `Confirmed`, `Workspace`:
1. drops the palette;
2. calls `window.focus(&tab_focus)`;
3. calls `window.dispatch_action(action.boxed_clone(), cx)`.

`dispatch_action` is deferred and starts from the element that has focus when it is called, which is the tab. The action therefore takes exactly the path its keybinding would.

### 5. Prompted commands are ordinary actions that ask `Workspace` for a prompt

`TmuxRenameWindow` and `TmuxRunCommand` are unit actions handled in `TmuxWindowView`. Their handlers emit a `PromptRequested` event carrying:
- the label;
- the initial value (the window's current name, or empty);
- a submit closure from `String` to a future of `Result<(), String>`.

`Workspace` subscribes to each `TmuxWindowView` it creates, and opens the palette in prompt mode on that event.

From the palette, this means the list closes, the action dispatches, and the prompt opens. All three happen in the same effect flush, before the next frame, so there's no visible flicker. A keybinding goes straight to the prompt. Either way it is one code path, as the spec requires.

*Alternative:* actions carrying a `String` argument. That was rejected because a keybinding would have no text to supply, and the palette would need to know which actions take arguments.

### 6. Free-form tmux lines are framed with a sentinel

`TmuxSession::run_user_command(line) -> oneshot::Receiver<Result<(), String>>` works like this:
1. It replaces any line breaks with spaces and rejects empty lines.
2. It writes `line`, then `display-message -p brindle-sync-<n>`, using a session counter `n`. Both writes happen under one `io` borrow, so nothing else can be queued between them.
3. It pushes a single `Pending::UserCommand { token, first_error, reply }`.

In `on_response`, while that entry is at the front of the queue:
- an `%error` block records its body as `first_error`, if none is recorded yet;
- an `%end` block whose body is exactly the token pops the entry and sends `Err(first_error)` or `Ok(())`;
- any other `%end` block is consumed.

Every other `Pending` kind still pops on its first block. If the session ends, the sender is dropped and the palette closes.

The submitted line is not given a `-t` target. Brindle already keeps the control client's current window and pane in sync with the focused tab through `select-window` and `select-pane`, so tmux's default target is the tab's active pane.

Before framing, the line goes through the directory rewrite in §6a.

Checked against tmux 3.6 (`-C`, throwaway server):
- `rename-window a ; split-window` replies with two blocks before the sentinel's.
- `bogus-command` replies with one `%error` block: `parse error: unknown command: bogus-command`.
- `if-shell true 'split-window'` replies with one block before the sentinel's. The inner command runs, but in the session directory: it isn't top level, so it isn't rewritten.
- `new-window -c "#{pane_current_path}" -c /tmp` starts in `/tmp`, so the last `-c` wins.

*Alternatives:*
- Parse the line to count commands. That was rejected because tmux's quoting and `{}` blocks make this fragile, and the count has to be exact: a miscount corrupts reply matching.
- Run the line with `if-shell`/`source-file` wrappers. That was rejected as more indirection, still one block per command.

### 6a. Run Command lines start in the current directory by rewriting

Without `-c`, tmux starts new windows and panes in the *session's* working directory, not the pane's. Only `new-session`/`attach-session -c` changes that directory, and the change persists, so it would affect every later window, including windows created from other clients. Brindle instead rewrites the line before sending it, with a pure function `with_start_dir(line) -> String`:

1. **Tokenise the top level only.** Single quotes, double quotes and backslash escapes are kept intact. `{ … }` blocks are tracked by depth and copied verbatim. A `;` that is unquoted, unescaped and at depth 0 ends a command, whether it stands alone or ends a word.
2. **Insert after the command name.** If the first word of a top-level command is `new-window`, `neww`, `split-window` or `splitw`, insert ` -c "#{pane_current_path}"` right after that word. The format is expanded by tmux against the client's current pane, which is the tab's active pane.
3. **Copy everything else byte for byte.** No re-quoting, so lines the tokeniser doesn't recognise pass through unchanged.

A user-supplied `-c` wins because it comes later in the arguments: tmux's `args_get` returns the last value given for a flag. A task verifies this on tmux 3.6. If it doesn't hold, step 2 instead skips commands whose flag words, before the first non-flag word, already contain `c`.

This parse is best-effort, unlike counting commands for reply framing (§6). A missed case only means tmux's default directory is used; it never affects correctness. Brace-nested commands and `command-alias` aliases are left alone on purpose, as the spec says.

*Alternative:* temporarily set the session directory with `attach-session -c`, then restore it. That was rejected because it needs a round trip to read the old path first, briefly changes behaviour for other clients, and leaves the session changed if anything fails in between.

### 7. tmux command mapping

| Action | tmux command |
|---|---|
| `tmux_new_window` | the existing `new_window()` |
| `tmux_layout_even_horizontal` / `_even_vertical` / `_main_vertical` / `_tiled` | `select-layout -t @W <name>` |
| `tmux_next_layout` | `next-layout -t @W` |
| `tmux_rotate_panes` | `rotate-window -Z -t @W` |
| `tmux_swap_pane_prev` / `_next` | `swap-pane -U` / `-D -t %P` (no `-d`, so the active pane follows; no `-s`, so a marked pane is ignored) |
| `tmux_break_pane` | `break-pane -s %P` (the new window becomes current, and `%window-add` → `list-windows` makes the new tab) |
| `tmux_rename_window` | prompt, then `rename-window -t @W <quote(name)>`; empty text does nothing |
| `tmux_command` | prompt, then `run_user_command` |

`@W` is the tab's window and `%P` is its active pane, through the existing `send_to_active_pane` helper. None of these actions gets a default key.

### 8. Detach moves to `TmuxWindow`, with a guard binding in `Workspace`

- The `TmuxDetach` handler moves to `TmuxWindowView` and its default binding moves to the `TmuxWindow` context.
- A hidden internal action, `Swallow`, is not in `ACTIONS` and is handled by `Workspace` as a no-op. ctrl-shift-d is bound to it in the `Workspace` context.

In control-mode tabs the deeper `TmuxWindow` binding wins. Elsewhere `Swallow` consumes the key, so `^D` never reaches a shell. Because `Swallow` has no label, it never shows in the palette.

### 9. Default bindings

| Key | Action | Context |
|---|---|---|
| ctrl-shift-p | `OpenCommandPalette` (new, config name `open_command_palette`) | Workspace |
| ctrl-shift-space | `OpenProfilePicker` (config name unchanged) | Workspace |

The strip's `⌄` button keeps opening the new-tab palette.

## Risks / Trade-offs

- **[The rewrite tokeniser disagrees with tmux's parser on an unusual line]** → Mitigation: the rewrite only inserts after a recognised command name and otherwise copies the line, so the worst case is a missing or misplaced `-c`. A misplaced `-c` would make tmux reject the line, and the prompt shows tmux's error. Unit tests cover quotes, escapes, braces, `;` attached to a word, and an explicit `-c`.

- **[tmux's per-command block rule doesn't hold for some commands]** For example, commands inserted by `if-shell`, or `run-shell -b` output arriving later. → Mitigation: the sentinel only requires every block for the line to come before the sentinel's block. A task checks `a ; b`, an `if-shell` line and an unknown command against tmux 3.6 on a throwaway server. A late stray block would fall through to the existing unmatched-reply handling, which is logged and ignored.
- **[A user command blocks the queue]** For example, `run-shell 'sleep 10'`. → Mitigation: the prompt shows a busy state, and escape closes the palette while the reply is consumed and dropped later. Brindle's own commands queue behind it, as they would in tmux.
- **[Commands that change the session's identity]** For example, `switch-client`, `kill-session` or `detach-client` in the run prompt. → Mitigation: they are allowed, and the spec makes no promise about what happens afterwards. Detach and exit go through the existing paths.
- **[The shortcut snapshot goes stale while the palette is open]** For example, if the config reloads. → Mitigation: this is accepted. The next open is correct.
- **[ctrl-shift-space is now consumed in every tab]** It is also some users' input-method toggle. → Mitigation: it is rebindable, and binding it to `none` frees it.
- **[BREAKING: ctrl-shift-p muscle memory]** → Mitigation: README and keybinding tables updated. The `⌄` button still opens profiles.

## Migration Plan

No data or config migration. Existing `[keybindings]` entries keep their meaning. Roll back by reverting the change.
