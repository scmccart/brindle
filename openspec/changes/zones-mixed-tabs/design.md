# Design

## Context

See proposal.md for why. Facts this design rests on, from the code on `main` at 19368a9 (2026-11-26):

- **Kind is a field of the tab's zone tree.** `ZoneTreeView.kind: TabKind` (`src/zone_view.rs`) is set once from the first zone, and `split_plan(tab: TabKind, profile: Option<ProfileKind>) -> SplitPlan::{Terminal, Agent, Refused(ProfileGone | ProfileKindChanged)}` makes every split start a zone of that kind with the tab's profile, or nothing.
- **A zone's content is already per zone.** `ZoneContent::{Terminal(Entity<TerminalView>), Agent(Entity<AgentView>)}`, with an exhaustive method for every per-zone question (focus, working directory, session, title, status, close, layout, dump). `Zone` holds content, title, status, progress, badge and `login_for`, and no profile.
- **Profile, theme and host are per tab, held twice:** `Tab.profile` and `Tab.host` (`src/workspace.rs`), `ZoneTreeView.profile`, `host`, `theme`, `zone_tokens` and `font_size_override`. `set_profile` pushes the one profile's name, theme and font to every zone.
- **One mixed tab exists.** `open_login_zone` (`agents-login`) starts a terminal beside an agent zone, "the one zone a tab starts of another kind", unrecorded.
- **Already per zone, with only wording that assumes one kind:** the command palette's facts (`PaletteFacts::zone`), `--send` and `--action` targeting, the dump's block for the active zone, the status, spinner, progress and badge summaries, the close confirmation (`closing()` per zone), and focus.
- **Session state** (`src/session.rs`, `VERSION = 1`): a tab records `profile`, `remote`, `cwd`, `title`, and a tree of `ZoneNodeState { cwd, session, active, split, first, second }`. A zone records no profile or kind. Earlier format changes were additive optional fields within version 1; neither type denies unknown fields.
- **Restore** takes each zone's kind from the tab's profile (`restored_zone`), and a recorded session of the other kind is left for the sweep (`Taken::OtherKind`).
- **Detached layouts** (`src/layout.rs`): each session carries its tab's layout blob, a tree with session keys as leaves, which the terminal server stores unread. A launch's sweep groups one terminal server's detached sessions by the tab they name (`group`), keeping only the active zone's kind. Each `SessionInfo` already carries its own kind (`agent`) and the profile it was started with.
- **Keys:** ctrl-shift-e and ctrl-shift-o are the splits; ctrl-shift-alt-e and ctrl-shift-alt-o are unbound.

The user's decisions (2026-11-26) are in "The user's decisions" below.

## Goals / Non-Goals

**Goals:**
- A tab holds zones of any profiles: either kind, any host.
- One keystroke adds a terminal, and one adds an agent, beside whatever pane is active.
- Everything a zone shows and does follows its own profile; nothing is left that asks "what kind is this tab".
- A mixed tab survives a quit and relaunch whole.
- State files and detached layouts written before the change still restore, and no protocol or server changes.

**Non-Goals:** moving a pane to another tab, or dragging to rearrange; a pane changing profile after it starts; resizing splits; choosing a host for a split other than by choosing a profile; a profile or setting of the tab itself; putting a swept tab that spanned hosts back together from several servers' sweeps; a settings dialog for any of it.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `split_choice` (which profile a split runs), `split_host`, `split_cwd` | pure | `brindle`, `src/zone_view.rs` (beside `can_split_zone`) |
| `Zone`'s profile, host, theme, tokens and font size; the tree no longer holding them | GPUI-free state in an entity | `src/zone_view.rs` |
| The six split actions, the choose-a-profile palette | shell (entities) | `src/zone_view.rs`, `src/workspace.rs`, `src/actions.rs` |
| A zone's record: `profile`, `remote` | pure (boundary parse) | `src/session.rs` |
| Resolving a recorded zone: its profile, host and kind | pure | `src/session.rs` (`resolve_zone`), `src/reattach.rs` |
| Grouping a swept layout of both kinds | pure | `src/layout.rs`, `src/reattach.rs` |
| Restore, snapshot, sweeps, config reload per zone | shell | `src/workspace.rs` |
| The dump's zone lines | pure | `src/zone_view.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`). Nothing changes in `brindle-remote-agent`, `brindle-remote-protocol` or `brindle-transcript`.

## Decisions

### 1. A zone owns its profile (`types R1`, `R3`)

```rust
struct Zone {
    content: ZoneContent,
    profile: Profile,            // as the config has it now; its kind is the content's
    host: Host,                  // where this zone's session runs; never changes
    theme: Theme,
    zone_tokens: Tokens,
    font_size: Option<Pixels>,
    // title, status, progress, badge, last_focused, login_for: as today
}
```

- `ZoneTreeView` loses `profile`, `host`, `kind`, `theme`, `zone_tokens` and `font_size_override`; `Tab` loses `profile` and `host`. What read them reads the zone it is about, or the active zone where it was about the tab (decision 7).
- **Kind isn't stored.** A zone's kind is its content's variant (`ZoneContent::kind() -> ProfileKind`). `TabKind` stays what it was made for: the kind a launch subcommand asks for (`src/main.rs`, the hand-off). No type says what kind a tab is, so no code can ask.
- **A zone never changes profile or host.** Its profile copy follows the config by name (a reload, a rename), and is kept as last seen when the profile is deleted.
- *Rejected:* keeping a tab profile as a default for its splits (two profiles to explain, and the first zone's would win for no reason); a `TabKind::Mixed` (every match would grow an arm that means "ask the zone").

### 2. Which profile a split runs (`fcis R1`; pure, unit-tested)

```rust
pub enum SplitChoice { Run(ProfileKey), Ask(Vec<ProfileKey>), Refused(SplitRefusal) }
pub enum SplitRefusal { NoProfileOfKind(ProfileKind) }
pub fn split_choice(want: ProfileKind, zone: Option<&Profile>, default: &Profile, profiles: &[Profile]) -> SplitChoice
```

For a split that asks for a kind, the first of these that applies, read from the config as it is now:
1. the split zone's own profile, when it still exists and is of the kind;
2. the default profile, when it is of the kind;
3. the only profile of the kind;
4. several profiles of the kind: `Ask`, listing them in the config's order;
5. none: `Refused(NoProfileOfKind)`.

So a terminal split from a terminal pane, and an agent split from an agent pane, behave as today's split. A split from a pane of the other kind lands on the obvious profile, and asks only when there is a real choice. A split whose own profile was deleted or changed kind no longer does nothing: it falls through to the rules below it.

- **Asking** opens a palette of those profiles (`Palette`, as the profile picker draws them); choosing one splits the pane that was active when it opened, when it still exists and still fits. Escape splits nothing.
- **Split … With Profile…** skips the rules: its palette lists every profile, of both kinds and any host, the default first.
- *Rejected:* the default profile before the zone's own (a split in an SSH pane would open a local shell); asking whenever the pane is of the other kind (the user asked for one keystroke).

### 3. Where the new zone runs, and in which directory (pure)

- **`split_host(split: &Zone, profile: &Profile) -> Host`:** the split zone's host when the profile is the split zone's own, so a split stays on the endpoint the zone runs on even when the profile's `remote` has since changed, as today. Any other profile runs where it says (`Host::of(profile)`).
- **`split_cwd(split_dir: Option<Dir>, same_host: bool, profile_cwd: Option<Dir>) -> Option<Dir>`:** the split zone's working directory when both zones run on the same host (the same endpoint), whatever the new profile's `cwd`, since a split means "here". Otherwise the profile's own directory, else the home directory there. A directory never crosses hosts, as "Remote zone split" already rules for tabs.
- An agent zone's working directory is the one its session was started in; a terminal's is its foreground process's.
- A split onto a remote host goes through that endpoint's connection, opening it if this is the window's first zone there, exactly as opening that profile in a new tab does. While it connects, or if it fails, the new zone shows what a new remote tab shows.

### 4. The commands (`src/actions.rs`)

| Action | Name | Default key |
| --- | --- | --- |
| `split_terminal_right` | Split Terminal Right | ctrl-shift-e |
| `split_terminal_down` | Split Terminal Down | ctrl-shift-o |
| `split_agent_right` | Split Agent Right | ctrl-shift-alt-e |
| `split_agent_down` | Split Agent Down | ctrl-shift-alt-o |
| `split_right_with_profile` | Split Right With Profile… | none |
| `split_down_with_profile` | Split Down With Profile… | none |

- All six are handled by the zone tree and act on the active zone. The palette lists a kind's two only while the config has a profile of that kind.
- `split_zone_right` and `split_zone_down` stay accepted in `[keys]` and by `--action` as other names for the terminal splits, and aren't listed. In every released version a tab was terminals only, so that is what they always did; nobody's binding changes meaning.
- **A zone header's two split buttons** start another zone of that zone's own kind (they run that kind's commands on it), with the tooltips "Split right" and "Split down" and that command's key. **Its menu** lists the four kind commands, the two that ask, and Close Zone; items are disabled where the zone is too small, and a kind's items where no profile of the kind exists.
- **A terminal login's zone can't be split**: it isn't a profile's zone and goes when its program exits. The six do nothing there.
- The size rule ("Minimum size for splitting") is unchanged and checked first.

### 5. Each zone in its own theme

- A zone's card, header and, for an agent zone, its token scope come from its own profile's theme and font size (`Previews::theme_for(&zone.profile)`), so a `gruvbox-dark` shell and a `brindle-light` agent sit side by side each as itself. `scope_of(id)` gives an agent zone its own `zone_tokens`.
- **The tab's face** on the strip takes the active zone's theme, and follows as focus moves between zones. `Tab::face_colors` reads the active zone.
- A config reload re-themes each zone from its own profile; a theme preview in the settings dialog applies to the zones of the profile being edited.
- The chrome keeps the window's theme, as "Chrome ignores per-tab themes" has it; the requirement's wording becomes per zone.

### 6. Titles, notifications and other per-tab reads

- A zone's title falls back to its own profile's name; the tab's title is its active zone's, as today.
- A notification names the zone it is about, with that zone's profile's name.
- `profile_on(host)` (the remote sweep's "profile of the first tab on the endpoint") becomes the profile of the first zone on that endpoint, in tab order then tree order.
- "The window has no tab on an endpoint" (when a launch stops waiting for it) becomes "no zone".
- A new tab's directory still follows the active zone when it runs on the new tab's host.
- A profile rename renames it in every zone that runs it.

### 7. Session state (`src/session.rs`; `process R15`)

- `ZoneNodeState` gains two optional fields on a leaf: `profile: Option<String>` and `remote: Option<RemoteTarget>`. A leaf written by this version always has `profile`, and has `remote` when its zone runs on a remote host. A leaf without `profile`, from an earlier version, takes the tab's `profile` and `remote`.
- The tab's own `profile` and `remote` stay, and hold the active zone's, so an earlier Brindle reading a newer file still restores a tab of a sensible profile. It restores every zone with that profile on that host; a zone whose session is elsewhere or of the other kind isn't taken and is swept as its own tab, which is what such a version does today.
- `VERSION` stays 1: the change is additive and every earlier file reads as before (the add-zones and session-id fields were added the same way).
- **`resolve_zone(record, tab, config) -> ResolvedZone { profile, host, missing: Option<String> }`** (pure, replacing the per-tab part of `resolve_tab`): the recorded profile by name, else the default profile with the missing name noted; the recorded host when the zone has a session record, else the profile's. The dim "profile NAME is gone" line is shown in the zone it is about.

### 8. Restoring a zone

- A zone with a recorded session that is listed is attached as what the session is: a terminal session into a terminal zone, an agent session into an agent zone, whatever kind its recorded profile now has. When the profile isn't of the session's kind (it was changed since), the zone takes the profile the sweep's rules give that session (`session_profile`).
- So `Taken::OtherKind` and the rule "a recorded zone whose listed session is not of the tab's kind is not restored into it" go: there is no tab kind for a session to differ from.
- A zone with no usable session starts its profile's program anew in its recorded directory, as today, on its own host.
- A remote zone attaches on its own endpoint. Endpoints connect independently, so a tab can show one zone attached and another connecting or failed; each says so itself, as a remote tab does today.
- Login zones and ended agents stay unrecorded, as today.

### 9. Swept layouts

- **`layout::group` keeps every session the layout names, of either kind.** The filter that kept the active zone's kind, and its test, go. Each swept zone takes its own session's kind and the profile the existing per-session rules give it (`session_profile`, agent sessions preferring an agent profile), no longer one profile for the tab.
- **A layout is grouped one terminal server at a time**, as now: a sweep sees only the sessions of the endpoint it is sweeping, and leaves a zone out when its session isn't among them, its sibling taking its place. So a tab that spanned this machine and `A8`, swept rather than restored, comes back as a tab of its local zones and, when `A8` is swept, a tab of its `A8` zones. *Rejected:* fitting a later sweep's zones back into the tab an earlier sweep opened: the tree may have changed in between, and the state file already restores the common case whole.
- While Brindle runs it sends each zone's server the whole tab's layout, as today. A leaf naming a session on another server is simply never matched there.
- No blob format change: leaves stay session keys, and kind and profile come from each session's own record.

### 10. The dump (`testing R2`)

```
--- zones count=3 active=2
--- zone id=1 rect=0,0,0.5,1 kind=terminal profile="Shell"
--- zone id=2 rect=0.5,0,0.5,0.5 kind=agent profile="Claude" active
--- zone id=3 rect=0.5,0.5,0.5,0.5 kind=terminal profile="A8" host="ssh:A8"
```

Each `--- zone` line gains `kind=` and `profile=` after its rectangle, and `host=` when the zone runs on a remote host, before `active`. The profile-choosing palette prints as any palette list does, and is driven with the list actions.

### 11. Failures (`railway R6`)

- No profile of the kind asked for: nothing is split, and the window's notice line says "no agent profile to split with" (or terminal). The palette doesn't list that kind's commands.
- The zone is too small: nothing, as today.
- The chosen profile's host can't be reached: the new zone shows the connection's failure, as a new remote tab's does; the other zones are unaffected.
- A recorded zone's profile is gone: the default profile, with the dim line in that zone.
- A state file leaf with a `remote` that doesn't parse: that zone is restored as its profile says, with a log line; the rest of the file is kept (`railway R5`).

### 12. Tests

- **Unit:** `split_choice` for every rule and each kind; `split_host` and `split_cwd`; a zone's record round-tripping with and without the new fields, and an earlier file's leaf taking the tab's; `resolve_zone` for a gone profile, a changed kind and a recorded host; `group` over a layout of both kinds and one with leaves on another server; the dump lines; the palette's listing of the split commands with and without a profile of each kind; the old action names resolving to the terminal splits.
- **E2E** (fake ACP agent): each of the six commands from a terminal pane and from an agent pane; the ask palette with two agent profiles; a split in the split pane's directory; a mixed tab restored after a quit, each zone with its session; a mixed tab swept after `--no-restore`; two zones of different themes, sampled; a login zone refusing to split. The existing split and close cases keep passing with the new zone lines.
- **Remote** (`remote-*`, Docker): a local tab split with an SSH profile; that tab restored, and restored with the container stopped.
- **Manual:** the header's split buttons and menu with a pointer; the tab's face changing colour as focus moves between zones of different themes; ctrl-shift-alt-e and ctrl-shift-alt-o on Linux and on Windows, with a layout that has AltGr.

## The user's decisions

Settled on 2026-11-26, one at a time:
- Mixed tabs are their own change, ahead of `agents-session-list`, which needs a terminal pane to be splittable with an agent.
- Separate commands and keys for splitting to a terminal and to an agent, the user's own proposal, as four commands: the terminal pair takes the old keys, and the old kind-following commands go (decision 4).
- A tab may hold panes on different hosts, which the user chose over the recommended one host per tab (decisions 3, 8, 9).
- Split Agent Right and Down are ctrl-shift-alt-e and ctrl-shift-alt-o, chosen over the recommended ctrl-shift-g and ctrl-shift-b.

- A split to a kind with several profiles runs the default profile when it is of that kind, and asks only otherwise, chosen over always asking (decision 2).
- Split Right With Profile… and Split Down With Profile… are kept as two commands, chosen over one command that asks twice and over leaving them out (decision 4). With any host allowed in a tab, they are how a profile the kind rules wouldn't choose is added, such as an SSH shell beside a local one.

Choices made here, for review:
- A split starts in the split pane's directory whenever both run on one host, even when the new profile sets its own (decision 3).
- A swept tab that spanned hosts comes back as one tab per host (decision 9).
- The tab's face follows the active zone's theme (decision 5).

## Risks / Trade-offs

- ctrl-shift-alt on Windows is shift-AltGr, which types a character on some layouts → the keys can be rebound, the commands are in the palette and the header menu, and the manual checks try such a layout.
- ctrl-shift-e in an agent pane now opens a terminal where it opened an agent → agents are unreleased, so no released habit changes; ctrl-shift-alt-e is the agent split.
- A tab's face changes colour as focus moves between zones of different themes → it is what tells which pane is active from the strip; a tab of one theme never changes.
- A tab spanning hosts restores in pieces when one host is slow or down → each zone says its own state, and the others work at once.
- A swept mixed-host tab splits into one tab per host → only the sweep does this, which covers sessions the state file didn't restore.
- An earlier Brindle reading a newer state file restores each zone with the tab's profile → it attaches what it can and sweeps the rest, and loses nothing.
- More commands (six for two) → four are a kind and a direction, and two ask.

## Migration Plan

Nothing to migrate. State files and layouts from before the change restore as they did: every zone of such a tab takes the tab's recorded profile and host. Config key bindings naming `split_zone_right` or `split_zone_down` keep working.
