# Design

## Context

See proposal.md for why. The code this design rests on, at `main` 725ed28 (file:line evidence in the manual-findings investigation, questions 5 to 7):

- **Working.** While a turn runs, the prompt input's submit button becomes the stop button (`PromptStatus::Streaming`), and the zone's and tab's status dot is a static `working` dot (`ZoneContent::status`, `Tab::status`, drawn with `dot(status_color(state))` in `workspace.rs` and in the zone header in `zone_view.rs`). The only animation is the `loader-circle` spinner on the submit button while a prompt is in flight (`prompt_input.rs`, `with_animation("prompt-spinner", Animation::new(SPIN).repeat(), …)`). Nothing shows in the conversation.
- **gpui 0.2.2's animation.** `AnimationElement::request_layout` calls `window.request_animation_frame()` while it isn't done, which is `on_next_frame(|cx| cx.notify(view))`. A repeating animation therefore notifies its view every display frame, and since gpui 0.2.2 caches no views, every notify re-renders the whole window (`process R14`). That is fine for the second a prompt is in flight; it is 60 to 144 whole-window renders a second for a turn that runs for minutes.
- **Permission rows.** `BlockBody::Permission` is drawn by `brindle_ui::Confirmation` (`view.rs`), one button per option, `ButtonVariant::Default` for allow kinds and `Outline` for reject kinds, allow kinds first (the model's order). The frame is shadcn's alert frame in `border`, `card` and `card_foreground`. `ButtonVariant::Destructive`, `Tokens::destructive`/`destructive_foreground` and `AlertVariant::Destructive` exist; `Confirmation` has no tone. The spec says a pending request takes no focus when it arrives ("Nothing is answered for the user").
- **Notifications.** `ChatModel::notice(prev, next, replaying)` titles a notice with `title()` and gives the pending permission's title, `Done` or the failure's words as its body; `Agent` emits it as a `Notice { source: Agent, … }` in `TerminalEvent::Status`; `Workspace::on_attention` applies `policy::wanted` and `policy::layout(source, title, body, tab_title)`. D-Bus gets the body as given (escaped when the server has `body-markup`); the toast puts the title and the body in one `<text>` each (`toast::xml`); `BRINDLE_NOTIFY_LOG` writes tabs and line breaks as spaces (`log_line`). The transcript keeps a permission's `call`, title, options and outcome, and a tool entry's `call`, title, kind and status, so the kind of the tool call a permission names is known client-side. The command a tool call runs isn't kept.
- **`agents-session-title`** (archived before this change) makes the zone's title the conversation's title, else the profile's name, and changes the agent arm of `policy::layout` to take the tab's title only when it is a manual one.

## Goals / Non-Goals

**Goals:**
- A working agent is visible in its conversation, its zone header and its tab, without costing the window more than a few renders a second, and costing nothing once no agent works.
- A pending permission reads as a risk, and the safe answer is the first one the keyboard reaches.
- A permission notification says who asks, what for, and what the choices are, in the same words on Linux and Windows.
- Every decision is a pure function with unit tests; the components take plain values and ask for no frames themselves.

**Non-Goals:** the command or file behind a permission beyond its title (decision 8); answering from a notification's buttons; a reduced-motion setting (gpui 0.2.2 reads no such preference; a config key can follow); spinners for terminals' OSC 7501 `working` records; any wire, transcript, config or state file change.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Loader` (a stepped spinner) and `Shimmer` (text with a moving band), with the pure `shimmer::levels(len, position)` | components and a pure helper | `brindle-ui`, `ui/src/components/{loader,shimmer}.rs` (new) |
| `ConfirmationTone::{Default, Destructive}`, `Confirmation::tone` | component | `ui/src/components/confirmation.rs` |
| `Tokens::destructive_tint` | token | `ui/src/theme.rs`; values in `src/theme.rs` `ui_tokens` and the neutral sets |
| The motion clock's decisions: `motion::next(running, wanted) -> Tick`, `STEP`, `loader_step(step)`, `shimmer_position(step, len)` | pure | `brindle`, `src/motion.rs` (new; the pure items in a shell module, reviewed as such) |
| `Motion`, the one clock: a GPUI global holding the step, the entities that want it and its task | shell (global, task) | `src/motion.rs` |
| `WorkingRow`, `ChatModel::working_row()`, the permission buttons' order, `AgentNotice`'s parts, `kind_word`, `permission_body` | GPUI-free model | `src/agent/model.rs` |
| `working_splice(before, after, len)` | pure | `src/agent/view.rs` |
| Drawing the row, the permission tone and variants; wanting motion | shell (entity) | `src/agent/view.rs` |
| `Mark::{None, Dot(State), Spinner}` and `mark(status, agent_working)` | pure | `src/workspace/` (beside `tabs.rs`), used by `zone_view.rs` and `workspace.rs` |
| The agent summary in `policy::layout`; a toast `<text>` per body line | pure | `src/notify/policy.rs`, `src/notify/toast.rs` (`toast::xml` is neutral, tested on Linux) |

Crate references don't change (`ports R1`), and no closed dependency list does (`ports R3`). `brindle-ui` learns nothing about agents: it gets a step, a position, a tone and strings (`ports R2`).

## Decisions

### 1. One stepped clock for the window (`process R13`, `R14`; `fcis R1`–`R3`)

- **What runs.** `Motion` is a GPUI global: `step: u32`, `wanted: BTreeSet<EntityId>`, `task: Option<Task<()>>`. `Motion::want(entity, bool, cx)` adds or removes an entity, then applies `motion::next(running, !wanted.is_empty())`: `Tick::Start` spawns the task, `Tick::Stop` drops it, `Tick::Keep` does nothing. The task waits `STEP` = 100 ms on the background executor's timer, then on the main thread adds one to `step` and calls `cx.notify(entity)` for each wanted entity; it ends when its update fails (the app is quitting) or the set is empty.
- **Who wants it.** Each entity that shows motion wants it, for itself, from its own change handlers, never from `render`: an `AgentView` while its working row shows (decision 2); a `ZoneTreeView` while one of its headers shows a spinner, and the `Workspace` while one of its faces does (decision 3). Each removes itself when that stops and in `on_release`, so a closed zone can't keep the clock running. Notifying the wanting entities, not refreshing every window, keeps the cost right if a later gpui caches views; with 0.2.2 one tick is one window render, however many entities want it.
- **What components get.** `render` reads `Motion::step(cx)` and passes plain values: `loader_step(step) = step % 12` to `Loader`, `shimmer_position(step, len)` to `Shimmer` (a sweep of 20 steps, 2 s, from 3 characters before the start to 3 past the end). Neither component reads the clock or asks for frames.
- **Why 10 steps a second.** A spinner of 12 steps a turn at 10 steps a second turns once in 1.2 s, which reads as motion; the shimmer's band moves a fraction of a word per step. It costs at most 10 renders a second while an agent works, against 60 or more for gpui's animation, and on Windows a 100 ms timer leaves the message loop free for `WM_PAINT` and `WM_QUIT` (CLAUDE.md's Windows pitfall).
- *Rejected:* gpui's `with_animation` with `repeat()` (a whole-window render every display frame for the whole turn); a timer per zone (several working zones would each wake the window out of phase); an opacity pulse only (`pulsating_between`), cheaper still but not the shimmer the user chose. The prompt input's in-flight spinner keeps its `with_animation`: it lasts until the agent acknowledges the prompt, normally well under a second.
- **Tests.** `motion::next`, `loader_step` and `shimmer_position` are unit-tested. An entity test in `TestAppContext` wants and unwants the clock, advances the test clock by `STEP` and checks the step moves only while wanted, the task is dropped once nothing is, and a released entity is removed.

### 2. The working row (`fcis R1`, `R5`; `types R1`)

```rust
pub enum WorkingRow { Working, Tool { title: SharedString }, Stopping }
pub fn working_row(&self) -> Option<WorkingRow>
```

- **The rule.** `None` unless the phase is `Live(Ready { .. })` and the session is open. Then by the turn: `Working` gives `Tool { title }` for the newest tool entry after the latest user entry whose status is `in_progress`, else `Working`; `Cancelling` gives `Stopping`; `Blocked`, `Idle`, `Ended` and `Failed` give `None`. So the row hides while a permission waits (the permission row is then the call to action) and comes back when the turn goes on.
- **Shown for the whole turn,** streaming included, not only in the gaps between chunks: a row that came and went between chunks would flicker at the pace of the agent's output. *Flagged for the user*, decision 13.
- **In the list, not beside it.** The row is the scroller's last item, after the model's blocks, so it reads as the end of the conversation and scrolls with it; the header and tab spinners cover a conversation scrolled away. The model knows no list index for it: `Absorbed` gains `working: Option<Option<WorkingRow>>` when it changed, and the view applies the pure `working_splice(before: bool, after: bool, len) -> Option<Splice>` (insert at `len`, remove at `len`, or nothing) after the model's splices, which all fall within the blocks. `agents-plans`' plan block sits between the conversation and the prompt input, so the two don't meet.
- **Drawn as** a muted row: `Shimmer` with `Working…`, the tool's title cut to one line, or `Stopping…`. It isn't an entry: the dump prints it as its own line, and copy actions never see it.

### 3. The spinner in place of the working dot (`fcis R1`; `types R1`)

- `mark(status: Option<State>, agent_working: bool) -> Mark::{None, Dot(State), Spinner}`: `Spinner` when the status is `Working` and `agent_working`, else the dot as today. Pure, beside the workspace's other models.
- **Zone header** (2+ zones): `agent_working` is the zone's own: an agent zone whose shown status is `working`. **Tab face:** `agent_working` is "one of the tab's working zones is an agent zone", computed by `ZoneTreeView` where it already computes `status_summary`, and cached on the `Tab` with its status in `refresh_status`, which every agent `Status` event already calls.
- A tab of one zone has no header, which is why the face shows it too; a terminal's OSC 7501 `working` keeps its dot, since a build's record can last for hours and a spinner there would keep the clock running. *Flagged*, decision 13.
- `Loader` is sized as the dot's box, so the face and header don't shift between states.

### 4. Permission rows (`ports R2`)

- **Order.** The model's permission buttons go reject kinds first, then allow kinds, each in the agent's order (`BlockBody::Permission`'s doc changes with it). The view registers the focus handles in that order, so tab from the prompt input reaches the first reject option first and `FocusStops` follows the row. The dump's `--- entry` line keeps the agent's order, which the spec now says outright.
- **"Reject focused by default"** is read as "the first stop the keyboard reaches", not as moving focus to the row when it arrives: the spec's "A pending request SHALL NOT take keyboard focus from where it is" stays, since a request arriving while the user types would otherwise take their next key. *Flagged*, decision 13.
- **Tone.** `Confirmation::tone(ConfirmationTone::Destructive)` while requested draws `Icon::TriangleAlert` in `destructive` before the title, the frame's background in `destructive_tint` and its border in `destructive`; the title stays `card_foreground`. Answered, a confirmation draws in the default tone whatever its tone, so the conversation's history doesn't keep warning. *Flagged*, decision 13. The login row keeps the default tone.
- **Variants.** Allow options are `ButtonVariant::Destructive`, reject options `Outline`, in the view (the component keeps whatever variants the owner gives).
- **The token.** `destructive_tint` is the card mixed with `destructive` as far as keeps `card_foreground` at 4.5:1 and at most 12 % of the way (`Color::legible_on`'s bound, in `src/theme.rs` `ui_tokens`); the neutral sets take fixed tints. `ui_tokens_are_legible_for_every_builtin` gains `card_foreground` on it (4.5:1) and `destructive` on it (3:1, the icon and border). A stored token, not an alpha computed in the component, so the legibility test sees the colour drawn and a profile's token set (`agents-zone-theme-scope`) carries its own.
- `ui/README.md` lists the tone as an addition: shadcn's destructive Alert has no tinted background in v4; the tint is the user's choice.

### 5. The components (`ports R2`; `fcis R5`)

- **`Loader`**: `Loader::new().step(n)`, an `IconElement` of `Icon::LoaderCircle` turned `n / 12` (`IconElement::turn`), sized in rems like the status dot. `RenderOnce`, no state, no focus.
- **`Shimmer`**: `Shimmer::new(text).position(p)`, one line of `StyledText` whose runs come from the pure `shimmer::levels(len, position) -> Vec<(Range<usize>, Level)>`: four `Level`s from `muted_foreground` to `foreground`, a character's level from its distance to the band's centre, adjacent equal levels merged, so a line has at most seven runs. Mixing only between those two tokens keeps the text at least as legible as `muted_foreground` on `card`, which the token test already pins at 3:1. gpui can't fill text with a gradient, so runs are the faithful form (investigation, question 5).
- **Gallery.** Entries for `Loader` and `Shimmer` driven by a gallery-local 100 ms timer, and a requested and an answered destructive `Confirmation`; `--check` gains "a destructive confirmation answered draws in the default tone".
- **Tests** in `TestAppContext`: `shimmer::levels` at the start, middle, end and outside the text; a `Loader` at two steps turns by a twelfth; a destructive confirmation's frame and icon bounds while requested and none once answered (`debug_selector`s).

### 6. Notifications' words (`fcis R1`; `desktop-notifications`)

- **The model's parts.** `ChatModel::notice` composes the title with `notice_title(profile, zone_title, kind)`: the profile's name, ` needs permission` for `blocked`, then ` — <zone title>` when the zone's title differs from the profile's name. The body for `blocked` is `permission_body(request, kind, more)`: line one `"<kind word>: <title>"` (or the title alone when the request names no tool call the model holds, or one of kind `other`), line two the options' names in the row's order joined by ` · `, then ` · +N more` for the other pending requests of the zone; `Waiting for permission` stays the title of a request with none. `kind_word` is `Read`, `Edit`, `Delete`, `Move`, `Search`, `Command` (execute), `Think`, `Fetch`, `Mode` (switch mode), and none for `other`. `Done` and the failure's words are unchanged.
- **The tab's part.** `policy::layout`'s agent arm (from `agents-session-title`) appends ` — <tab title>` only for a manual title that is neither one of the title's ` — `-separated parts nor the tab's profile name; it gets `TabTitle::{Automatic(&str), Manual(&str)}` and the tab's profile name, which `on_attention` has. The programs' arms are unchanged.
- **One format, two platforms.** D-Bus gets the body with its line break. `toast::xml` writes the title and then one `<text>` per body line, at most two (ToastGeneric shows a title and two text lines; a third line joins the second with a space). The log keeps writing a line break as a space. The OSC sources never put a line break in a body (`clean` drops control characters), so their toasts are unchanged.
- *Rejected:* the tool title in the summary (it is the longest part and would push the names out of a truncated title); one line `Allow once / Reject` in the summary.

### 7. The dump (`testing R2`)

- `--- working "TEXT"` after the entry lines while the row shows, `TEXT` the row's words.
- `--- spinner tab=N` per face with a spinner (numbered as the tab lines), then `--- spinner zone=ID` per header with one in the active tab of 2+ zones, after the progress and badge lines; `--- motion running|stopped` once, last before the focus line. `--wait-for` matches the agent lines as before.

### 8. The command behind a permission

The user asked whether to keep the command. **Recommendation: not in this change.** `agents-tool-content` keeps a tool call's raw input (`input`), its command output's `command` and its file changes in the transcript, as protocol 4 forms, and applies a permission's `toolCall` to the entry it names; a second field for the same thing here would be a protocol amendment that change then replaces. Most agents' permission titles already hold the command or path (Claude: the command; Copilot and Codex: a verb and the file). Whichever of the two changes lands second adds a detail line to `permission_body` (the call's `Output.command`, else its first file change's path), which fits the toast's third line. *Flagged*, decision 13.

### 9. Failures (`railway R6`)

Nothing new can fail where the user would see it. A clock task whose update fails (the app is quitting) ends. A permission naming a tool call the model no longer holds (trimmed) is worded without a kind. A tool title or option name is shown as the server cleaned and bounded it.

### 10. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Nothing here is server state. After a core restart or an upgrade the replay rebuilds the entries, the turn and the agent state, so the working row and spinner follow from the model as before; a replay never notifies, as today.

### 11. How it meets the other changes

- **`agents-tool-content`** also modifies "Permission rows" (a pending request shows its tool call's detail above its buttons). Whichever archives second writes its delta against the other's text: this change's order rule and that change's detail sentence both stay. Its open-while-running tool rows don't change the working row's rule.
- **`agents-plans`** can draw an in-progress step's mark with `Loader`, wanting the clock while it shows.
- **`agents-session-title`** is archived first (proposal).
- **`agents-zone-theme-scope`**, if it lands, gives an agent zone its profile's token set; `destructive_tint` is in it, and the spinner and shimmer read their tokens there too.

### 12. Tests (`testing R1`, `R13`)

- Unit: `working_row` for every turn and phase, a tool in progress, completed and after the latest user message; `working_splice`; the permission order; `kind_word`, `notice_title`, `permission_body` with and without a call, kind `other` and `+N more`; `policy::layout`'s agent arm with automatic, manual, equal and profile-named titles; `toast::xml` with one, two and three body lines; `mark`; the motion functions; `shimmer::levels`; the token pairs.
- Entity (`TestAppContext`): the clock (decision 1); an agent view test that the row appears after a prompt, follows a tool in progress, hides while blocked, says `Stopping…` and goes at the end, with the view unwanting the clock; the permission row's focus order through `FocusStops`.
- E2E (`tests/e2e/cases/agent.rs`): `agent-working` (the fake agent reports a tool in progress behind a gate: the dump's `--- working`, `--- spinner tab=1` and `--- motion running`; then the turn ends: no working line, `--- motion stopped`); `agent-permission-style` (a window capture of a pending row: the tint behind the title and the destructive fill of the allow button, from the `e2e` theme's colours; after answering, the card colour again); `agent-notify` extended to the new title and two-line body in `BRINDLE_NOTIFY_LOG`; `agent-permission-keys` updated for reject first.
- Manual (real pointer and eyes): the spinner and shimmer look smooth enough at 10 steps a second on Linux and Windows; the destructive row in brindle-light, brindle-dark and solarized-dark; a real permission toast on Windows and a D-Bus notification on GNOME showing both body lines.

### 13. Choices the user didn't make (flags)

1. "Reject focused by default" is the reject options coming first in the row and in the tab order; a request still doesn't take focus when it arrives (decision 4). **Recommended**, since stealing focus mid-typing breaks an existing requirement.
2. The working row shows for the whole working turn, streaming included, hides while a permission waits, and says `Stopping…` while a cancel is on its way (decision 2). **Recommended**, to avoid flicker.
3. The spinner also replaces the working dot on the tab's face, since a one-zone tab has no header; terminals' `working` records keep their dot (decision 3). **Recommended.**
4. Motion steps at 10 a second from one clock rather than gpui's frame-rate animation (decision 1). **Recommended** for `process R14`.
5. Only pending permission rows are destructive; answered ones go back to plain (decision 4). **Recommended.**
6. The notification title is `<profile> needs permission — <zone title>`, the body two lines with the kind word and the options, `+N more` for other pending requests, and the tab's title only when it is a manual one (decision 6). **Recommended.**
7. The command isn't kept here; `agents-tool-content` keeps it, and the later of the two adds a detail line (decision 8). **Recommended.**
8. Execute's word is `Command` and switch mode's `Mode` (decision 6).
9. The tint is a stored token, `destructive_tint`, at most 12 % of the way to `destructive` (decision 4). **Recommended** over a component-side alpha, so the legibility test covers it.

## Risks / Trade-offs

- 10 steps a second looks steppier than a 60 Hz animation → a spinner of 12 discrete steps is the usual look of a stepped loader; the manual check judges it, and `STEP` is one constant.
- A turn of many minutes costs 10 window renders a second → bounded and only while working; the clock stops with the last working zone, and a dump line pins that it does.
- An entity that forgets to unwant the clock keeps it running → each wanting entity also unwants in `on_release`, and the dump's `--- motion` line makes a leak visible in every e2e case that ends a turn.
- Two changes modify "Permission rows" → decision 11 says how the second rebases.
- Reject-first changes muscle memory for tab-enter → the safe answer is the one a reflex now gives.

## Migration Plan

Nothing to migrate: no config key, state file or protocol changes. The e2e expectations that change (`agent-permission-keys`, `agent-notify`) change with the code.
