# Tasks

## 1. Config editing for profiles

- [x] 1.1 Add `shlex = "1.3"` to `Cargo.toml` without running `cargo update`. Verify that `git diff Cargo.lock` shows only the new dependency edge on the existing 1.3.0 and that `cargo build` succeeds.
- [x] 1.2 In `config_edit.rs`, add the per-entry profile accessor for `[[profiles]]` and inline arrays, `profiles(doc)` (each entry deserialized on its own), `ProfileRef` with the position-plus-name check, and `materialize_implicit`. Verify with `cargo test config_edit`: both array forms are read, one bad profile doesn't hide the others, a stale reference is refused, and an empty config gains `Shell`.
- [x] 1.3 Add `upsert_profile` (blank fields remove keys; args, tmux args and env written inline), `rename_profile` (updates `default_profile`, ignoring case), `delete_profile` (refuses the last profile; clears `default_profile` for the default), `move_profile` (comments move with the table) and `set_default_profile`. Verify with unit tests for each in both array forms, including a move up and back that round-trips `DEFAULT_CONFIG` byte for byte.

## 2. Profiles section model

- [x] 2.1 Restructure `settings_dialog::Model` into `Section`, `ThemeState` and `ProfilesState` (design Decision 1). Leaving Theme resets its preview, and switching is refused while an editor has unsaved edits. Verify that the existing theme unit tests pass unchanged, and add tests for section switching.
- [x] 2.2 Add the profile list model: config order, ctrl-alt-N numbers, default marker, implicit Shell, unreadable entries not editable, and the fallback to `Settings.config.profiles` when the file doesn't parse. Verify with unit tests, including the "error elsewhere in the config" scenario.
- [x] 2.3 Add `ProfileEditor`: field strings from a `Profile` (shell words via `shlex::try_join`, env as `KEY=` plus the quoted value), parsing back to a `Profile`, validation per the "Profile field validation" requirement, theme choices (follow global, built-ins, customs, missing name kept), and New/Duplicate naming. Verify with unit tests for every validation and naming scenario in the spec, and for a round-trip of a profile with quoted args and env.
- [x] 2.4 Wire Save (upsert, rename, set-default; returns `Outcome::Written { renames }`), Discard, Delete with confirmation, and Move up/down to `config_edit`. Verify with model tests against a temp config: rename the default profile, delete the default, move a profile, and save to a file changed underneath, which reports the reopen message.

## 3. Workspace preview and renames

- [x] 3.1 Add `profile_preview` and `effective_profile` to `Workspace`, and route `apply_config`'s per-tab update through it. Setting or clearing the preview re-applies theme and font size to the matching tabs and tmux views without tmux color reports. Verify with `cargo build`, and check that a reload with no preview behaves as before (the existing e2e cases in task 5.2).
- [x] 3.2 Make `reload_config` take renames and re-key `tab.profile.name` (and the tmux session view's profile) in every window before matching. The mtime poll and `ReloadConfig` pass none. Verify with a unit test of the re-keying helper (case-insensitive match) and `cargo build`.
- [x] 3.3 Update the `profiles` documentation: README's profiles section mentions the dialog, and the profiles comment in `assets/default-config.toml` points to it. Verify that `default_config_file_parses` passes.

## 4. Profiles section view

- [x] 4.1 Add the section list entry, the profile list (numbers, description, default marker, error rows) with New, Duplicate, Edit, Delete and move buttons, the delete confirmation, and the new actions and bindings (design Decision 8). Verify that `brindle --list-actions` lists them and that the `actions.rs` tests pass.
- [x] 4.2 Add the editor view: `TextInput` fields, the choice widgets (left/right, click dropdown) and the default toggle in the `SettingsChoice` context, all in tab order. tmux fields show only when the mode isn't none, invalid fields are marked, there is a "new tabs only" note, and the body scrolls. Push edits into the model, and emit the profile preview to `Workspace`. Verify with `cargo build` and the model tests from section 2.
- [x] 4.3 Extend the `--- settings` dump for the Profiles section (list state and editor state), and add a `--self-test` parser assertion on a sample Profiles line. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.

## 5. End-to-end and integration

- [x] 5.1 Add e2e cases (ask the user before running, since they open windows), using the generated test config's profile and the `e2e`/`e2e-alt` themes:
  - `settings-profile-preview`: open the dialog, `settings_section_next`, `settings_edit`, focus the theme choice and `settings_choice_next` to `e2e-alt`. The window background samples as `e2e-alt`. Then `settings_cancel` twice restores `e2e`, and the config is unchanged.
  - `settings-profile-save`: the same, then `settings_confirm`. The profile table has `theme = "e2e-alt"`, its comments are intact, and the dialog is on the profile list.
  - `settings-profile-rename`: rename the profile with `--send`, save, then set its theme and save. The open tab shows the new theme, proving it was re-keyed.

  Verify that `BRINDLE_E2E=1 cargo test --test e2e -- settings-profile` passes.
- [x] 5.2 Integration check: run `cargo test`, `cargo build --release` and the full e2e suite (with permission). Offer to reinstall with `scripts/install.sh`, and give the user a manual checklist: dropdown clicks, alt-up/alt-down reorder followed by ctrl-alt-N, delete confirm on the default profile, and creating a profile on a config with no `[[profiles]]`.
- [x] 5.3 Stop closing the settings dialog on a click outside, in lists as in editors (escape, Cancel and Close still close it). Verify with `cargo build`, and with the dialog still open in the dump of a run that previews a theme and then gets a stray click, checked by hand since e2e can't click.
