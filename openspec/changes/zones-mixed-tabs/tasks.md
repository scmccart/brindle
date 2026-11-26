# Tasks

Before starting:
- `agents-profiles`, `agents-login` and `agents-zone-theme-scope` are implemented and archived. The user's decisions are settled; design.md lists them, and the choices made in the design that the user hasn't reviewed.
- This change MODIFIES `agent-zones`' "Agent commands in the palette", which `agents-tool-content` also modifies (it adds Toggle Tool Details). Whichever archives second carries the other's text: re-read the main spec before archiving.
- `agents-session-list` depends on this change.
- The function names below are from `main` at 19368a9; check each against the code.

## 1. The pure plans

- [ ] 1.1 Replace `split_plan` in `src/zone_view.rs` with `split_choice(want, zone, default, profiles) -> SplitChoice::{Run, Ask, Refused(NoProfileOfKind)}` (design decision 2); verify with unit tests for each rule and each kind: the zone's own profile, its profile gone, its profile of the other kind, the default of the kind, the only one of the kind, several, none.
- [ ] 1.2 Add `split_host(split, profile)` and `split_cwd(split_dir, same_host, profile_cwd)` (decision 3); verify with unit tests: the zone's own profile staying on its endpoint after the profile's target moved, another profile running where it says, the split zone's directory on the same host whatever the profile's `cwd`, and no directory crossing between local and remote or between two remotes.
- [ ] 1.3 Keep `can_split_zone` first in every split; verify the existing size tests still pass.

## 2. A zone owns its profile

- [ ] 2.1 Give `Zone` its `profile`, `host`, `theme`, `zone_tokens` and `font_size`, and `ZoneContent::kind()`; remove `profile`, `host`, `kind`, `theme`, `zone_tokens` and `font_size_override` from `ZoneTreeView` and `profile` and `host` from `Tab`, each reader taking the zone it is about or the active zone (decisions 1, 6). `TabKind` stays for launches and the hand-off only. Verify `cargo test -p brindle --test architecture` and the existing unit tests pass.
- [ ] 2.2 Rework `split_zone` to take what to start (a kind, or a chosen profile) and to use `split_choice`, `split_host` and `split_cwd`; a remote split opens its endpoint's connection as a new tab of that profile does; a login zone refuses every split (decision 4).
- [ ] 2.3 Draw each zone's card, header and agent token scope from its own profile (`insert_zone`, `leaf_look`, `scope_of`, `set_profile`); `Tab::face_colors` from the active zone; `apply_config`, `apply_tab_themes` and the settings previews per zone's profile; a rename following each zone's profile (decision 5); verify with unit tests for the theme a zone and a tab's face get in a mixed tab and after a reload.
- [ ] 2.4 Titles and notifications from the zone's own profile; `profile_on(host)` and the "no tab on an endpoint" checks per zone (decision 6); verify with unit tests of the title fallback and the endpoint checks.

## 3. Commands

- [ ] 3.1 Add `split_terminal_right`, `split_terminal_down` (ctrl-shift-e, ctrl-shift-o), `split_agent_right`, `split_agent_down` (ctrl-shift-alt-e, ctrl-shift-alt-o), `split_right_with_profile` and `split_down_with_profile` (no keys) to `ACTIONS`; keep `split_zone_right` and `split_zone_down` as unlisted names for the terminal splits in `[keys]` and `--action`; list a kind's two only while the config has a profile of it; verify with unit tests of the palette's listing with and without each kind, the old names resolving, and `--list-actions`.
- [ ] 3.2 Add the choose-a-profile palette (the profiles of one kind for `Ask`, every profile for the two that ask), splitting the zone that was active when it opened if it still exists and fits; a refusal on the notice line; verify in the e2e cases of section 6.
- [ ] 3.3 A zone header's split buttons run its own kind's commands; its menu lists the six and Close Zone, disabled by size and by kind; tooltips keep "Split right" and "Split down" with the key of the command they run; verify with a unit test of `zone_menu_rows`.

## 4. Session state and restore

- [ ] 4.1 Add optional `profile` and `remote` to a leaf's `ZoneNodeState` and to a single-zone tab's record, written for every zone; the tab's own `profile` and `remote` hold the active zone's; `VERSION` stays 1 (decision 7); verify with unit tests: a record round-trips, an earlier file's leaf takes the tab's profile and host, a newer file's tab-level fields are the active zone's, and a leaf whose `remote` doesn't parse falls back with the rest of the file kept.
- [ ] 4.2 Add `resolve_zone` and use it in `restore_zones` and `restored_zone`: each zone's own profile and host, the "profile is gone" line in its zone, a listed session attached as what it is under the sweep's profile rule when its recorded profile is of another kind; remove `Taken::OtherKind` and its tests (decision 8); verify with unit tests for each.
- [ ] 4.3 Record each zone in `Workspace::snapshot`; login zones and ended agents stay unrecorded.
- [ ] 4.4 `layout::group` keeps both kinds, and `swept_tabs`, `insert_layout_tab` and `attach_detached` give each zone its own session's profile; a leaf on another server is left out as a missing session is (decision 9); replace `a_mixed_layout_keeps_the_active_zones_kind` with tests for a layout of both kinds and one whose leaves ran on two servers.

## 5. The dump

- [ ] 5.1 Add `kind=`, `profile=` and, for a remote zone, `host=` to the `--- zone` lines (decision 10); update the e2e cases that compare zone lines; verify with unit tests for each form and escaping.

## 6. End-to-end cases

- [ ] 6.1 `zones-split-kinds` (fake ACP agent): each of the four kind commands from a shell and from an agent zone, by `--key`, with the zone lines' kinds and profiles; the old `split_zone_right` name through `--action`.
- [ ] 6.2 `zones-split-ask`: two agent profiles, the palette's rows, a choice with the list actions, and escape splitting nothing; `split_right_with_profile` choosing a second terminal profile.
- [ ] 6.3 `zones-split-directory`: an agent split of a shell that has changed directory starts there, though the agent profile sets `cwd`; a terminal split of an agent zone starts in the agent's directory.
- [ ] 6.4 `zones-mixed-restore`: a tab of a shell and an agent zone quit and relaunched comes back with both sessions; the same tab after a `--no-restore` launch comes back from the sweep as one tab.
- [ ] 6.5 `zones-mixed-themes`: two zones of different themes, each card and header sampled, and the tab's face before and after `focus_zone_left`.
- [ ] 6.6 `zones-login-no-split`: the split keys do nothing in a terminal login's zone.
- [ ] 6.7 `remote-mixed-tab` (Docker): a local shell split with an SSH profile; the tab restored; restored with the container stopped, the local zone working and the remote one saying it can't connect; and swept with another state directory, as one tab per host.
- [ ] 6.8 Run the Windows cases that split (`windows-zones`) and add a `windows-agent-*` case for an agent split of a shell.

## 7. Docs and checks

- [ ] 7.1 Update `README.md` (keys, splitting, a tab of both kinds), `docs/code-map.md` (the zone tree, the tab model, restore and sweeps; fix the stale `as_terminal_view()` mention), `CLAUDE.md` (the dump's zone lines), and `docs/manual-checks/` (replace "an agent profile opened from the `⌄` menu, with its split buttons disabled", which no longer holds).
- [ ] 7.2 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate zones-mixed-tabs --strict`.
- [ ] 7.3 Manual checks for the user, with `scripts/run-throwaway.sh`: a pointer press on each header split button and each header menu item in a mixed tab; the tab's face changing as focus moves between zones of different themes; ctrl-shift-alt-e and ctrl-shift-alt-o on Linux and on Windows, with a keyboard layout that has AltGr; a real agent beside a shell in the same directory; a local shell beside a shell on a real SSH host, quit and relaunched.
