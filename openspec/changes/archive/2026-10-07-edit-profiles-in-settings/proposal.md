# Proposal

## Why

Profiles are the main thing a Brindle user customizes: what each tab runs, where it starts, how it looks, and whether it's a tmux tab. Today they can only be changed by editing `[[profiles]]` in `config.toml`. The settings dialog was built so further sections could sit beside the Theme section, and profiles are the natural next one. Changing a profile's theme or font size there can also preview on that profile's open tabs, which a file edit can't do.

**Depends on `add-text-input`**, which must land first: paths, command lines and environment variables need a text field with a cursor, selection and IME.

## What Changes

- A **Profiles** section in the settings dialog, listed under Theme. It lists every profile in config order, with its ctrl-alt-N number, a short description of what it runs, and a marker on the default profile.
- **Profile editor**, opened by Edit, New or Duplicate. It covers every profile setting:
  - name;
  - command, and args as one line of shell words;
  - working directory;
  - environment as one line of `KEY=value` shell words;
  - theme: follow the global theme, or any built-in or custom theme;
  - font size: blank means the global size;
  - tmux mode (none, plain or control), with session and tmux args shown when the mode isn't `none`;
  - whether this is the default profile.
- **Live preview:** while the editor is open, the theme and font size apply at once to the open tabs of the profile being edited, and only to those. The other fields affect only new tabs, and the editor says so.
- **Saving** writes that one profile's table, keeping comments and formatting, and the dialog stays open on the list. Discard reverts the preview and writes nothing. This mirrors the theme editor.
- **Renaming** a profile also updates `default_profile`, and keeps open tabs of that profile linked to it, in every window, so later edits still reach them.
- **New, Duplicate, Delete and reordering:**
  - Delete asks first and is refused for the last profile.
  - Reordering (alt-up/alt-down) is written immediately and renumbers the ctrl-alt-N shortcuts.
- A config with no `[[profiles]]` shows the implicit "Shell" profile. The first profile edit writes it out as a real table, so it doesn't vanish.
- Out of scope: per-profile keybindings and new profile settings.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `settings-dialog`:
  - Sections now lists Theme and Profiles;
  - new requirements for the profile list, editor, field validation, preview, save, rename, delete, duplicate and reorder;
  - a click outside no longer closes the dialog, in a list or an editor; keyboard operation extends to the Profiles section;
  - switching sections drops an unapplied theme preview.
- `profiles`: renaming a profile from the dialog keeps its open tabs following it, and the default profile is kept consistent when profiles are renamed or deleted.

## Impact

- **Code:**
  - `config_edit.rs`: read profiles one by one from the document; upsert, rename, delete, move and set-default operations for both `[[profiles]]` and the inline `profiles = [...]` form; materializing the implicit Shell profile. All unit-tested.
  - `settings_dialog.rs`: the Profiles section, its list and editor model (pure, unit-tested), and non-text fields (theme selector, tmux mode, default checkbox).
  - `workspace.rs`: a profile preview override, applied through the same path as `apply_config`, and re-keying tabs on rename.
  - `main.rs`: `reload_config` takes the renames to apply.
  - `actions.rs`: list and field actions for the section.
- **Dependencies:** `shlex = "1.3"`, which is already in `Cargo.lock` at 1.3.0 as a transitive dependency, so no new crate is fetched.
- **Tests:** unit tests for the config edits and the editor model; e2e cases for a profile theme preview, save, and rename keeping a tab linked.
