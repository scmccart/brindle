# Proposal

**Depends on:** `agents-profiles` (agent zones, `ZoneContent`, `split_plan`), `agents-login` (the terminal login zone, the one mixed tab there is today) and `agents-zone-theme-scope` (an agent zone drawn in its profile's theme), implemented and archived.
**Next:** `agents-session-list`, which asks "split this pane or a new tab" from any pane and so needs a terminal pane to be splittable with an agent.

## Why

A tab holds zones of one kind and one profile today: a split of a terminal tab starts another terminal of the tab's profile, and a split of an agent tab starts another agent. So an agent can't sit beside the shell it is working on, and a shell on a remote host can't sit beside a local one. The user works with both at once and switches tabs to do it.

Nothing about a zone needs the rule. A zone's content is already its own type (`ZoneContent::{Terminal, Agent}`), every zone already asks its own status, title and close questions, and a terminal login zone already runs beside an agent zone. What is per tab is the profile, the theme and the host, each read by the split.

## What Changes

- **A zone has its own profile, and with it its own kind, theme, font size and host.** A tab holds zones of any profiles: terminals and agents, on this machine and on remote hosts, side by side.
- **Four split commands replace the two.** Split Terminal Right and Split Terminal Down keep ctrl-shift-e and ctrl-shift-o. Split Agent Right and Split Agent Down are new, on ctrl-shift-alt-e and ctrl-shift-alt-o. Each starts a zone of its kind whatever the split pane is: with the pane's own profile when that is of the kind, else the default profile when it is, else the only profile of the kind, else it asks which in a palette.
- **Split Right With Profile… and Split Down With Profile…** split the active pane with any profile picked from a palette, which is how a pane on another host, or of a second profile of the same kind, is added. No default keys.
- **A new pane starts in the split pane's directory when both run in the same place**, whatever its profile's own directory, since a split means "here". A directory is never carried from one host to another.
- **Each pane is drawn in its own profile's theme.** The tab's face on the strip follows its active pane.
- **Session restore records each pane's profile and host**, so a mixed tab comes back whole, each pane attached where it ran. A pane whose host can't be reached shows that, as a remote tab does today, and the others are unaffected.
- **Sessions left detached and swept at launch** group into tabs of both kinds. A layout is kept with each terminal server's own sessions, so a swept tab that spanned hosts comes back as one tab per host; the state file is what restores it whole.
- **The dump's zone lines say each pane's kind and profile.**

The old `split_zone_right` and `split_zone_down` action names stay accepted in the config and by `--action`, as the terminal splits, which is what they did in every released version.

Not in this change: moving a pane between tabs or dragging to rearrange; a pane changing its profile after it starts; resizing splits; choosing a host for a split apart from choosing a profile; a tab-level profile setting.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `zones`: splitting (the four commands and the two that ask), a new zone's profile, directory and host, the header menu and buttons, and each card's theme.
- `tabs`: the tab's title fallback and face follow the active zone.
- `session-restore`: each zone's profile and host are recorded and restored; swept layouts group both kinds, one terminal server at a time.
- `profiles`: a profile's appearance and a rename apply to its zones, not to tabs.
- `ui-theme-tokens`: the chrome ignores per-zone themes.
- `command-line`: the dump's zone lines.
- `agent-zones`: opening an agent profile is unchanged; a login zone is one of several ways a tab is mixed.

## Impact

- **`brindle`** (`src/zone_view.rs`): `Zone` gains its profile, host, theme, tokens and font size; `ZoneTreeView` loses `profile`, `host`, `kind`, `theme`; `split_plan` takes the kind asked for and gives a profile or a question; `split_zone` takes what to start; the header menu's rows.
- **`src/workspace.rs`** and `src/workspace/`: `Tab` loses `profile` and `host` (read from the active zone); the six split actions; the profile-choosing palette; restore, snapshot and sweeps per zone; `apply_config` re-theming and renaming zones by their own profiles.
- **`src/session.rs`**: optional `profile` and `remote` on a zone's record, additive within state format 1.
- **`src/layout.rs`**, `src/reattach.rs`: `group` keeps both kinds; each swept zone takes its own session's profile.
- **`src/actions.rs`**: four new actions and keys, two that ask, the two old names as aliases.
- **`src/terminal/`**, `src/agent/`: nothing but what a zone is given at its start.
- **No protocol change, no change in the terminal server**: a layout stays an opaque blob per session, and each session already records its own profile and kind.
- **Tests:** unit tests of the pure plans; e2e cases for each split command, a mixed tab's restore, a mixed sweep and themes; the remote cases for a tab spanning hosts.
- **Docs:** `README.md`'s keys and splitting, `docs/code-map.md`, `CLAUDE.md`'s dump lines, the manual checks.
