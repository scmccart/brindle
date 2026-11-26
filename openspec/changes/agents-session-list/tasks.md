# Tasks

Before starting:
- `agents-session-recovery`, `agents-login`, `agents-session-title`, `agents-log-paging`, `agents-tool-content` and `zones-mixed-tabs` are implemented and archived. The user's decisions are settled; design.md lists them, the choices made in the design that the user hasn't reviewed, and the flags (decision 16).
- Client protocol 4 and the ACP checkpoint's format 2 are still unreleased. If a release has shipped either, stop and raise design decision 9.
- Message numbers are fixed by design decision 9: `AGENT_LIST` 31, `AGENT_CHOOSE` 64, `AGENT_CONVERSATIONS` 45.
- `session/list` and its fields are named only in `acp/v1/`, behind `Dialect` (`ports R14`).
- Every test, build and e2e run here uses its own `CARGO_TARGET_DIR`, and e2e runs go through `systemd-run --user --scope scripts/with-xvfb …`.

## 1. Captures and shared contracts

- [x] 1.1 Capture each real ACP v1 agent we can run (Codex, Antigravity, the Claude adapter; Copilot when its login works) answering `session/list` with and without `cwd`, with a cursor where it pages, and in a directory with no conversation; add the logs to `remote-agent/tests/fixtures/acp/` named `AGENT-VERSION-list.log`, with identifying values replaced as the existing ones are; write down in design.md's decision 15 each agent's order (newest first or not), whether it honours `cwd`, the form of `updatedAt`, its page size, and whether listing needs a login.
- [x] 1.2 Add `AgentState::Choosing { message: Option<String> }` to `brindle-transcript` (`transcript/src/lib.rs`), with new `tests/fixtures/v4/agent-choosing.json` and `agent-choosing-refused.json` in `tests/events.rs`' `fixtures()`; draw it in `src/fuzz.rs`; verify it round-trips, `fold(replay(t)) == t` and the page laws with it, and every existing fixture passes unedited.
- [x] 1.3 Add to `brindle-remote-protocol` (`remote-protocol/src/client.rs`): `MSG_AGENT_LIST` 31 (`AgentList { req, session, cursor? }`), `MSG_AGENT_CHOOSE` 64 (`AgentChoose { req, session, conversation? }`), `MSG_AGENT_CONVERSATIONS` 45 (`AgentConversations { req, session, conversations: [{ id, title?, updated? }], next? }`) and `SpawnAgent`'s optional `start` (`SpawnStart::Choose`, `"choose"`), with the module's table; verify with transcript fixtures for each message, both forms of the choice and an old `SPAWN_AGENT` unchanged, decoding errors and the wrong-direction test, and the crate's `fuzz.rs`.
- [x] 1.4 Add `Listed`, `Listing` and `Request::List { id, cwd, cursor }` to `acp/forms.rs`, and `Initialized::lists`; verify `cargo test -p brindle --test architecture` passes.

## 2. Pure code in the server

- [x] 2.1 In `acp/v1/`: encode `Request::List` as `session/list { cwd, cursor? }` (`encode.rs`), and decode its answer entry by entry from the raw JSON, an unreadable entry lost alone (`decode::list_result`, through `Dialect::list_result`); read `sessionCapabilities.list` in `decode::initialize_result`; verify with unit tests pinning the request and over the captures of 1.1.
- [x] 2.2 Extend `acp/reduce.rs` with `Start::Choose` and `Phase::Choosing { greeting }` (`AgentSession::choosing`): `Step::start` sends no `session/new` and emits `AgentState::Choosing`; `Command::Choose { conversation }` continuing as `Start::Continue { log: Log::Fresh }` does by each `Continuation`, or sending `session/new` with none; `Refused::NotChoosing`; a prompt refused `NotReady`; a login needed by a list or by the chosen start returning to `Choosing` or to that start (`AfterLogin`); a refused continue going back to `Choosing { message }`; verify with unit tests for each.
- [x] 2.3 Add the list step: `Command::List { req, cursor }` accepted in `Ready` and `Choosing`, `Purpose::List { req }` in `acp/rpc.rs`, its answer as `Output::listed`; `Refused::{NotOffered, ListBusy}`, `NotReady`, the agent's error; the bounds (256 entries, titles one line of 256 characters, ids, times, cursors); entries of another directory dropped; `next` passed through; verify with unit tests for each.
- [x] 2.4 Extend `acp/checkpoint.rs` (format 2, amended) with `Phase::Choosing`, `Start::Choose` and the list capability, a pending list left out (flag 4); verify a choosing session round-trips, and a checkpoint from before this change decodes as before.

## 3. Server roles

- [ ] 3.1 Handle `SpawnAgent::start`, `AGENT_LIST` and `AGENT_CHOOSE` in the core (`server/core/{clients,agents,policy}.rs`, `AgentRequest::{List, Choose}`): gather, step, write the agent's line, answer `OK`, `AGENT_CONVERSATIONS` or `ERROR`; verify with real-role tests in `remote-agent/tests/agents.rs` through the fake ACP agent: a chooser lists and then continues a conversation in the same process (one spawn); a list through a ready agent; a choice of none; each refusal's text; paging with a cursor; a chooser closed without a choice leaving no session; a login during a list.
- [ ] 3.2 Add the restart cases (`remote-agent/tests/upgrade.rs` or `agents.rs`): a choosing session keeps choosing across a core restart and an upgrade, and a choice is then carried out.
- [ ] 3.3 Give the fake ACP agent what the cases need (offering `list`, answering `session/list` with pages and with an error) through its existing script steps, adding a step only if a case can't be written without one.

## 4. Brindle's model

- [ ] 4.1 Extend `src/agent/shared.rs` and `transcript.rs` for `AgentState::Choosing`; add `list`, `choose` and the choosing start to `Requests` and `remote::AgentLink`, with the protocol gate naming the endpoint, and the reply `AGENT_CONVERSATIONS` through the sink; verify with unit tests.
- [ ] 4.2 Add the pure plans in `src/agent/conversations.rs`: `lister_plan`, `list_dir` (shared with `zones-mixed-tabs`' split rule), `open_plan`, `latest_plan` and `ago(now, text)`; verify with unit tests for: a ready zone of the profile in the directory chosen as lister and one in another directory not; each of `list_dir`'s cases; `open_plan` for a conversation already open, a stopped zone, a choosing zone, a running agent and a shell; `latest_plan` for an empty list, for Antigravity's order (the newest `updated`, not the first) and for times that don't parse (the first); `ago` for minutes, hours, yesterday, a date, and text that doesn't parse.
- [ ] 4.3 Add the choosing screen's parts to `src/agent/model.rs` (design decision 6's table: the reason and the buttons for each state, the profile gone offering none), with no working row and no attention while choosing; `state=choosing` in `describe`; verify with unit tests for each row of the table.
- [ ] 4.4 Add the conversations palette's model (states, rows, "More…", the 1,024 bound, the "open" mark, the question's rows and their disabling) and the dump lines; verify with unit tests for each state and row form and escaping.

## 5. The zone and the workspace

- [ ] 5.1 Add `agent_conversations` and `agent_continue_conversation` to `ACTIONS` in `src/actions.rs` (no default keys), listed as design decision 7 says, and rename `agent_continue`'s palette name to "Continue This Conversation"; verify with unit tests of the palette's listing in a shell, in an agent zone, with no agent profile and in a stopped zone.
- [ ] 5.2 Build the palette in `src/workspace.rs` (a `PaletteKind` of its own beside `SplitProfile`): the profile question when several, the chooser's start and the list request off the UI thread (`process R13`), "More…", escape closing a chooser that was started for it, and closing it with the window and on quit.
- [ ] 5.3 Carry out a choice by `open_plan`: show the zone that has it; take it in the zone (`AGENT_CHOOSE`, or a stopped zone reset onto the new session with the old one closed); or ask, then split with the profile through `SplitWith::Profile` or open a tab, the new zone attaching the chooser or starting a continuing session.
- [ ] 5.4 Draw the choosing screen and a stopped zone's buttons in `src/agent/view.rs`, as keyboard stops after the existing one, wired to the palette and to New conversation.
- [ ] 5.5 Leave a choosing zone out of the session state and the detached layout, and end its session on quit, as an ended agent's is.

## 6. The command line

- [ ] 6.1 Parse `-c/--continue` and `-r/--resume` under `agent` in `src/main.rs` (`AgentLaunch::choose`), with their usage errors; carry the choice in `instance::AgentRequest::choose` under the kind `agent-choose`, and turn an older Brindle's refusal of that kind into the message and status 1; verify with unit tests of the parser, of the hand-off's encoding and of `Request::asked` for both kinds.
- [ ] 6.2 Start the tab's zone as a chooser; for `-c` ask for the first page and apply `latest_plan`; for `-r` open the palette on the zone once it is choosing; `-p` sent on the first `Ready`.

## 7. End-to-end cases (fake ACP agent)

- [ ] 7.1 `agent-conversations-list`: the palette from an agent zone and from a shell, its title, rows, an untitled row, "More…", the empty directory, and a second agent profile's question.
- [ ] 7.2 `agent-conversations-open`: from a shell, Split Right and New Tab; from a running agent zone, the question; a conversation already open showing its zone; escape at the question returning to the list; one spawn of the agent's command for a list and its choice.
- [ ] 7.3 `agent-conversations-stopped`: an exited agent's zone shows the three buttons; Conversations… and a choice replace it in place; New conversation starts over; a profile removed from the config offers none.
- [ ] 7.4 `agent-conversations-cli`: `-c` continuing the first listed conversation; `-c` with none showing the choosing screen and its reason, then New conversation; `-r` opening the list, a choice taken in the tab, and escape leaving the choosing screen; `-c -r` as a usage error; a quit while choosing leaving nothing to restore.
- [ ] 7.5 `agent-conversations-failures`: an agent without a list; an agent's error on `session/list`; a refused continue returning a choosing zone to the choosing screen.

## 8. Docs

- [ ] 8.1 Update `README.md` (the Agent tabs section and the launch options), `docs/code-map.md`, and `CLAUDE.md` (the dump's agent states, the palette lines and focus stops, the launch options).
- [ ] 8.2 Tell `agents-acp-v2` what it inherits: add to its proposal that `session/list` is unchanged in v2 and that a choice continues through its `session/resume`.
- [ ] 8.3 Add the manual checks to `CLAUDE.md`'s list: each agent's list with titles and times that read right; `brindle agent NAME -c` and `-r` in a project with conversations and in one with none; a conversation chosen from a shell into a split in the same directory; pointer presses on the choosing screen's buttons and on the palette's rows.

## 9. Integration

- [ ] 9.1 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests, the release build and the e2e suite under `scripts/with-xvfb` in a systemd scope; `openspec validate agents-session-list --strict`.
- [ ] 9.2 The PR's CI: every job, the three e2e shards and the archive jobs; a `windows-agent-*` case with `-c` on the Windows runner.
- [ ] 9.3 Archive the change in the implementation PR, checking the MODIFIED blocks against the main specs first.
- [ ] 9.4 Hand the manual checks to the user, with `scripts/run-throwaway.sh` and real agent profiles.
