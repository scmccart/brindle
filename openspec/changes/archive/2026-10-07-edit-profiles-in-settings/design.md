# Design

## Context

- **Profiles in config:** `Config.profiles: Vec<Profile>`, written as `[[profiles]]` (or, less often, an inline `profiles = [...]`). `normalized()` adds an implicit "Shell" profile when there are none and names unnamed ones `Profile N`. Order drives ctrl-alt-N and the first-profile default fallback. `default_profile` names a profile, matched case-insensitively.
- **Tabs and profiles:** each `Tab` (and each tmux session's view) holds a cloned `Profile`. `Workspace::apply_config` finds it again by `config.profile(&tab.profile.name)` and falls back to the stale clone. `tab_theme(profile, config)` resolves a tab's theme, with `theme_preview` applied only to tabs whose profile has no theme.
- **Settings dialog** (after `add-text-input`): a pure `Model` with modes `List`, `Editor` and `ConfirmDelete` for themes, plus a view that owns `TextInput` entities, pushes their text into the model, and moves focus with GPUI tab stops. `Outcome::Written` reloads and keeps the dialog open. `config_edit::save` re-reads and re-parses the file at save time.
- **`config_edit`** already walks profile tables in both forms (`for_each_reference_slot`) to rewrite theme references.
- **`shlex` 1.3.0** is in `Cargo.lock`. It provides `split`, `try_quote` and `try_join`.

## Goals / Non-Goals

**Goals:**
- Reuse the theme section's shape: pure model, `config_edit` operations, `Outcome`-driven reloads.
- Exactly one preview mechanism for profiles, going through the same code path as a real reload.

**Non-Goals:**
- Validating that a command or directory exists. A path may live on an unmounted disk.
- Editing a profile's keybindings, or adding profile settings that don't exist yet.
- Live-applying command, cwd, env or tmux changes to running tabs.
- Preview in other windows, the same limit as the theme preview.

## Decisions

### 1. Sections become a real enum in the model

`Model` gains `section: Section { Theme, Profiles }`, with `ThemeState` and `ProfilesState` holding each section's list, selection and mode. The existing theme fields move into `ThemeState` unchanged. `select_section` is refused while the current section is in an editor mode with unsaved changes. Leaving Theme resets its selection to the active theme, so `preview()` returns that theme and an unapplied preview is dropped.
- *Alternative:* a second dialog. Rejected: the original design planned sections for exactly this.

### 2. Profiles are read from the document one table at a time

`config_edit::profiles(doc) -> Vec<Result<Profile, String>>` deserializes each `[[profiles]]` (or inline) entry on its own, as `custom_themes` does, so one bad profile or an error elsewhere doesn't hide the rest. The dialog then applies the same naming rules as `normalized()` (`Profile N`, implicit Shell). If the file can't be parsed, the list falls back to `Settings.config.profiles`, read-only, with the parse error shown, the same fallback the theme list uses.

### 3. Profiles are addressed by position plus expected name

A profile has no stable id. Each save sends `ProfileRef { index, expected_name: Option<String> }`, where `None` stands for an unnamed or implicit profile. Because `save` re-reads the file, an op first checks that position `index` still holds that name. If not, it fails with "the config file changed; reopen the Profiles section", and the dialog reloads its list. This catches a hand edit in the `ctrl-,` tab between opening and saving without inventing ids.

### 4. `config_edit` profile operations

All of these work on both `ArrayOfTables` and inline arrays through one small accessor that yields `&mut dyn TableLike` per entry:
- `upsert_profile(doc, at: Option<ProfileRef>, &ProfileFields)`: sets or removes each key (blank means remove), keeping the table's decor and position. `None` appends. `args`, `tmux_args` and `env` are written as an inline array and an inline table.
- `rename_profile(doc, at, new)`: sets `name` and rewrites `default_profile` when it equals the old name, ignoring case.
- `delete_profile(doc, at)`: refuses the last profile, and removes `default_profile` when it names the deleted profile.
- `move_profile(doc, at, up)`: swaps adjacent entries. For `ArrayOfTables` the tables are taken out and pushed back in the new order, so each table's leading comments (its decor prefix) move with it.
- `set_default_profile(doc, name)`.
- `materialize_implicit(doc)`: when there are no profiles, writes `[[profiles]] name = "Shell"` first. Every profile op calls it, so adding `Work` to an empty config keeps Shell.

Unit tests cover both array forms, comment preservation, and the stale-reference refusal.

### 5. Editor model and field kinds

`ProfileEditor` holds the original `ProfileRef`, the field strings and the parsed result. The kinds:
- **Text** (`TextInput`): name, command, args, cwd, env, font size, session, tmux args.
- **Choice:** theme ("follow global", then `BUILTIN_THEMES`, then custom themes sorted; an unknown name in the file is kept as an extra "name (missing)" entry so opening and saving doesn't change it) and tmux mode. left/right cycle, and a click opens a small dropdown list.
- **Toggle:** default. Space or click toggles it. Unchecking the current default is not offered: another profile has to be made default instead, so the config never ends up with a dangling `default_profile` from the dialog.

Shell-word fields format with `shlex::try_join` and parse with `shlex::split`; `None` from `split` means invalid. Env words are formatted as `KEY=` plus the quoted value, so `PAGER='less -R'` reads naturally. Each word is parsed by splitting on the first `=`, and the key must match `[A-Za-z_][A-Za-z0-9_]*`. Name uniqueness ignores case because lookup does. The tmux fields stay in the model when hidden, so toggling the mode doesn't lose them. A profile saved with mode none drops `tmux_session` and `tmux_args`.

Choice and toggle widgets get focus handles with tab stops, in a `SettingsChoice` key context that binds left/right and space. They take part in tab order with the text inputs.

### 6. Profile preview reuses the reload path

`Workspace` gains `profile_preview: Option<(String /* original name */, Profile)>`. The lookup that `apply_config` uses to re-find a tab's profile becomes `effective_profile(tab_name, config)`, which returns the preview profile when the name matches (ignoring case) and the config's profile otherwise. Setting or clearing the preview runs the same per-tab update as a reload (theme through `tab_theme`, font size through `set_config`), so preview and saved state can't diverge. As with `theme_preview`, no tmux color reports are sent while previewing; the real reload after Save sends them. New and duplicated profiles have no tabs, so their preview is a no-op.

### 7. Renames re-key tabs before the reload matches them

`reload_config` takes `renames: &[(String, String)]`. Before matching, `apply_config` sets `tab.profile.name = new` for any tab whose name equals an old name (ignoring case), in every window. The dialog passes the rename with the `Written` outcome (`Outcome::Written { renames }`). The mtime poll and ctrl-shift-r pass none. The preview is cleared before the reload, as Apply does for themes, so a stale preview can't win.

### 8. Actions and dump

- New unlabelled actions: `settings_section_next` and `settings_section_prev` (also how e2e reaches Profiles), `settings_duplicate`, `settings_move_up`/`settings_move_down` (alt-up/alt-down), and `settings_choice_next`/`settings_choice_prev`/`settings_toggle`.
- Existing `settings_new`, `settings_edit`, `settings_delete`, `settings_confirm` and `settings_cancel` act on the current section.
- The `--- settings` dump gains `section=Profiles` states: the list with the selected profile and default, or the editor with its fields, invalid fields and `can_save`.

## Risks / Trade-offs

- [Position-plus-name refuses a save after an unrelated reorder by hand.] → Rare, and the message tells the user what to do. Proper ids would mean a new config key.
- [Re-pushing an `ArrayOfTables` to reorder might shift blank lines between tables.] → A unit test pins the round-trip of the default config with one move up and back.
- [Font size preview resizes the PTYs of the previewed tabs, and each keystroke in the font field could trigger a resize.] → Preview only when the parsed value changes and is valid. Resizing is what a reload does anyway.
- [Rename re-keying ignores case while tab titles show the name as typed.] → Titles come from the profile name after re-keying, so they update with the rename.
- [Deleting a profile leaves its tabs on a stale copy.] → Specified as intended; the tabs keep running.

## Migration Plan

None. The config format is unchanged. The dialog writes ordinary `[[profiles]]` keys.
