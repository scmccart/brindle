# Tasks

Run `cargo test --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings` at the end of each group. GUI runs (the gallery, e2e) go under `scripts/with-xvfb`, or ask first.

## 1. Pure decisions in the app (design decisions 1, 2, 3, 4, 6)

- [x] 1.1 Add `src/motion.rs` with `STEP` (100 ms), `Tick::{Start, Stop, Keep}`, `next(running, wanted)`, `loader_step(step)` and `shimmer_position(step, len)` (decision 1), and unit tests for each; verify with `cargo test -p brindle motion`.
- [x] 1.2 Add `WorkingRow` and `ChatModel::working_row()` (decision 2) with unit tests for every turn state and phase, a tool in progress after and before the latest user message, a completed tool, and an ended session; verify with `cargo test -p brindle agent::model`.
- [x] 1.3 Order the model's permission buttons reject kinds first, then allow kinds, each in the agent's order, and change `BlockBody::Permission`'s doc; add a unit test with `Always allow`, `Allow once`, `Reject`; check the dump's entry line still lists the agent's order (its existing test passes unchanged).
- [x] 1.4 Add `kind_word`, `notice_title` and `permission_body` and use them in `ChatModel::notice` (decision 6), with unit tests for a call of each kind, `other`, no call, a trimmed call, `+N more`, and done and error titles with and without a zone title that differs from the profile.
- [x] 1.5 Add `working_splice(before, after, len)` in `src/agent/view.rs` (decision 2) with unit tests for appearing, going and unchanged.
- [x] 1.6 Add `Mark` and `mark(status, agent_working)` beside the workspace's models (decision 3), with unit tests for each status with and without an agent working.
- [x] 1.7 Change `policy::layout`'s agent arm to take `TabTitle::{Automatic, Manual}` and the tab's profile name, appending a manual title only when it is none of the title's parts nor the profile name (decision 6); unit tests for automatic, manual, equal-to-zone-title and equal-to-profile titles; the programs' arms' tests pass unchanged.
- [x] 1.8 Write one `<text>` per body line in `toast::xml`, at most two, a third joining the second (decision 6), with unit tests for one, two and three lines and an OSC body; verify on Linux with `cargo test -p brindle notify::toast`.

## 2. brindle-ui (decisions 4 and 5)

- [x] 2.1 Add `Tokens::destructive_tint`, its values in the neutral sets, and in `src/theme.rs` `ui_tokens` (the card mixed toward `destructive`, at most 12 %, keeping `card_foreground` at 4.5:1); extend `ui_tokens_are_legible_for_every_builtin` and the neutral sets' test with `card_foreground` (4.5:1) and `destructive` (3:1) on it; verify with `cargo test -p brindle ui_tokens_are_legible_for_every_builtin` and `cargo test -p brindle-ui theme`.
- [x] 2.2 Add `ConfirmationTone` and `Confirmation::tone` (warning icon, tint and destructive border while requested; the default tone once answered), with `debug_selector`s and `TestAppContext` tests for both states; verify with `cargo test -p brindle-ui confirmation`.
- [x] 2.3 Add `components/loader.rs` (`Loader::new().step(n)`) with a test that two steps differ by a twelfth of a turn and that drawing it requests no animation frame.
- [x] 2.4 Add `components/shimmer.rs` (`Shimmer::new(text).position(p)`) and the pure `shimmer::levels(len, position)` with unit tests at the start, middle, end and outside the text, and a check that no run is dimmer than `muted_foreground`.
- [x] 2.5 Add gallery entries for `Loader`, `Shimmer` (driven by a gallery-local 100 ms timer) and a requested and an answered destructive `Confirmation`, plus a `--check` scenario that an answered destructive confirmation draws in the default tone; list the three in `ui/README.md`, with the tint named as a difference from shadcn; verify with `scripts/with-xvfb cargo run -p brindle-ui --example gallery -- --check`.

## 3. The motion clock and the views (decisions 1 to 4)

- [x] 3.1 Add the `Motion` global (`want`, `step`, the task on the background executor's timer, notifying each wanted entity) and register it in `main.rs`; an entity test in `TestAppContext` advances the test clock by `STEP` and checks the step moves only while wanted, the task is dropped when nothing is, and a released entity is removed.
- [x] 3.2 Draw the working row in `AgentView` as the scroller's last item with `Shimmer` (`Working…`, the tool's title, `Stopping…`), applying `Absorbed.working` through `working_splice` after the model's splices, and want the clock while it shows; an agent view test (`src/agent/view/tests.rs`) checks the row appears after a prompt, follows a tool in progress, hides while blocked, reads `Stopping…`, goes at the end, and the view stops wanting the clock.
- [x] 3.3 Draw pending permission rows with `ConfirmationTone::Destructive`, allow options `Destructive` and reject options `Outline`, registering focus handles in the row's order; a view test checks tab from the prompt reaches the first reject option, and the existing "a request takes no focus on arrival" test passes.
- [x] 3.4 Cache `agent_working` beside each tab's status in `refresh_status` (from `ZoneTreeView`), draw `Loader` for `Mark::Spinner` on zone headers and tab faces, and want the clock from `ZoneTreeView` and `Workspace` while one shows; a workspace test checks a working agent tab's face is `Mark::Spinner` and a terminal's `working` record stays a dot.
- [x] 3.5 Pass the tab's `TabTitle` and profile name to `policy::layout` in `on_attention`; the workspace's notification test checks the agent summary for an automatic and a manual tab title.

## 4. The dump and the e2e cases (decision 7, decision 12)

- [x] 4.1 Print `--- working "TEXT"`, `--- spinner tab=N`, `--- spinner zone=ID` and `--- motion running|stopped` in the dump, with unit tests of the lines where they are pure (`ChatModel::describe`); update the dump's description in CLAUDE.md's "Verifying GUI behaviour" section.
- [x] 4.2 Add e2e case `agent-working` (a gated tool call in progress: `--- working "Read main.rs"`, `--- spinner tab=1`, `--- motion running`; after the gate: no working line, no spinner, `--- motion stopped`); verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent-working`.
- [x] 4.3 Add e2e case `agent-permission-style` (a window capture of a pending row: the tint behind its title and the allow button's destructive fill from the `e2e` theme; after `--key tab --key tab --key enter`, the row back on the card colour) and update `agent-permission-keys` for reject first; verify both under `scripts/with-xvfb`.
- [x] 4.4 Extend `agent-notify` to the new title (`Fake needs permission`) and the body `Command: cargo test Reject · Allow once` in `BRINDLE_NOTIFY_LOG`, and a done notification after a named conversation if `agents-session-title`'s fake-agent step is there; verify under `scripts/with-xvfb`.
- [x] 4.5 Update `docs/code-map.md`'s `agent/`, `notify/` and `brindle-ui` entries (the motion clock, the working row, the permission tone and order, the notification words, the toast's lines, the new components) and add the manual checks of decision 12 to CLAUDE.md's "Manual checks" list.

## 5. Integration

- [ ] 5.1 Run the whole suite headless and check every agent case passes: `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent`, inside `systemd-run --user --scope` when run from a Brindle tab.
- [x] 5.2 Hand the manual checks to the user: the spinner and shimmer at 10 steps a second, the destructive row in brindle-light, brindle-dark and solarized-dark, and a real permission notification on Windows (toast) and on Linux (D-Bus) showing both body lines, through `scripts/run-throwaway.sh` / `run-throwaway.ps1`.

## Workflow follow-up

- If `agents-tool-content` has been archived first, rebase this change's "Permission rows" delta on its text (design decision 11), and add the detail line of decision 8 to `permission_body`.
- Archive the change in the implementation PR once its checks pass.
