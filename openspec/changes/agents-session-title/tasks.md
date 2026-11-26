# Tasks

Run `cargo test --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings` at the end of each group. E2E runs go under `scripts/with-xvfb`, or ask first.

## 1. The shared contract (design decision 1)

- [x] 1.1 Add `Conversation.title: Option<String>` (defaulted, omitted when `None`) in `transcript/src/lib.rs`; extend the replay property test and `src/fuzz.rs` with titled conversations; verify with `cargo test -p brindle-transcript`.
- [x] 1.2 Add `tests/fixtures/v4/conversation-titled.json` with `BRINDLE_REGEN_FIXTURES=1` and check every existing fixture still passes unchanged (`git status` shows only the new file); note in the commit that protocol 4 is amended in place, unreleased.

## 2. The server's ACP client (decisions 2, 3, 4)

- [x] 2.1 Decode `session_info_update` into `Update::SessionInfo { title: Patch<String> }` in `acp/wire.rs` (or `acp/v1/decode.rs` with `acp/forms.rs` if `agents-tool-content` landed first), defining `Patch` if it doesn't exist yet; unit tests for absent, `null`, a string and a number, and the capture tests decoding Claude's, Antigravity's and Copilot's updates as `Set` and Codex's as `Keep`; verify with `cargo test -p brindle-remote-agent acp::wire`.
- [x] 2.2 Add `session_title` and `MAX_SESSION_TITLE` (256) in `acp/reduce.rs` with unit tests: control characters, line breaks and runs of spaces, blank, exactly 256 and 400 characters, a cut inside a multi-byte character.
- [x] 2.3 Apply `SessionInfo` in the reducer (a new `Conversation` only when the title differs; nothing before a conversation exists; applied and not counted during a load's replay) and carry the title in `ready` for the same session id; unit tests for set, keep, clear, the same title twice, an update before `session/new`'s answer, a login restart on the same session keeping it, a load keeping it, and a new session clearing it; replace `a_session_info_update_without_a_title_changes_nothing`'s scope with these; the capture replays pass.
- [x] 2.4 Prefer a non-blank `agentInfo.title` for the greeting's name, with unit tests from the Codex, Claude and Antigravity captures' `initialize` answers.
- [x] 2.5 Round-trip a titled session in `acp/checkpoint.rs`'s tests, and decode a checkpoint written without titles unchanged.
- [x] 2.6 Add a real-role test in `remote-agent/tests/agents.rs`: the fake agent sends `session_info_update` with a title, the test client receives the titled conversation, the core is killed with SIGKILL, and a client attaching to the new core receives it in the replay; add a `title` helper to `support/fake_agent.rs`; verify with `cargo test -p brindle-remote-agent --test agents`.

## 3. Brindle (decisions 5 and 6)

- [ ] 3.1 Make `ChatModel::title()` the conversation's title else the profile's, add `agent_label()` for `failure_text`'s `{agent}`, and update the model's title tests: the agent's own name doesn't title the zone, a title and a cleared title emit `title_changed`, a profile rename still retitles an untitled zone, a respawn falls back to the profile's name; verify with `cargo test -p brindle agent::model`.
- [ ] 3.2 Add `TabTitle::{Automatic, Manual}` to `policy::layout`, with the agent arm appending a manual title only when it differs from the notice's title; build it in `Workspace::on_attention` from the tab's `manual_title`; unit tests for an automatic tab, a manual one, a manual one equal to the zone's title, and the programs' arms unchanged; verify with `cargo test -p brindle notify::policy`.
- [ ] 3.3 Update the agent entity and view tests that expected the tab or zone to read `Fake Agent` (`src/agent/tests.rs`, `src/agent/view/tests.rs`) to the profile's name.

## 4. End to end and docs

- [ ] 4.1 Add e2e case `agent-session-title`: a fake agent that names itself `Fake Agent` leaves the tab titled with the profile's name; after a gated `session_info_update` the tab and the dump's zone read the title; a `null` title brings the profile's name back; a tab renamed through the rename prompt keeps its name after another title; verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent-session-title`.
- [ ] 4.2 Update the agent cases that expected `Fake Agent` as a tab title (`agent-close-idle`, `agent-close-confirm` and others found by `grep -n 'Fake Agent' tests/e2e/cases/agent.rs`) and `agent-notify`'s expected title, then run `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- agent` and check every agent case passes.
- [ ] 4.3 Update `docs/code-map.md`'s `brindle-transcript`, ACP client, `agent/` and `notify/` entries (the conversation's title, `session_info_update`, the readable name, `title()` and `agent_label()`, `TabTitle`).

## Workflow follow-up

- Archive this change before `agents-zone-attention`, whose "Agent notifications" text replaces this one's.
- Archive the change in the implementation PR once its checks pass.
