# Tasks

Before starting:
- `agents-session-recovery`, `agents-login` and `agents-session-title` are implemented and archived, and so is `zones-mixed-tabs`, whose split with a given profile this change uses. The user's decisions are settled; design.md lists them, and the choices made in the design that the user hasn't reviewed.
- Client protocol 4 is still unreleased. If a release has shipped it, stop and raise design decision 9.
- Message numbers are the next free ones when this lands (31, 32 and the core's 45 after the planned `agents-commands`, `agents-prompt-context`, `agents-prompt-images` and `agents-plans`).
- When `agents-tool-content` has landed, `session/list` and its fields are named only in `acp/v1/`, behind `Dialect`; before it, they sit where the other v1 decoding does and move with it.
- The function names below are from `main` at 19368a9; check each against the code.

## 1. Captures and shared contracts

- [ ] 1.1 Capture each real ACP v1 agent we can run answering `session/list` with and without `cwd`, with a cursor where it pages, and in a directory with no conversation; add the logs to `remote-agent/tests/fixtures/acp/` with identifying values replaced as the existing ones are; write down in design.md's "As built" each agent's order (newest first or not), whether it honours `cwd`, the form of `updatedAt`, its page size, and whether listing needs a login.
- [ ] 1.2 Add `AgentState::Choosing` to `brindle-transcript` with a new `tests/fixtures/v4/` file; verify it round-trips, `fold(replay(t)) == t` and the page laws with it, it joins `fuzz.rs`, and every existing fixture passes unedited.
- [ ] 1.3 Add to `brindle-remote-protocol`: `AGENT_LIST { req, session, cursor? }`, `AGENT_CHOOSE { req, session, conversation? }`, `AGENT_CONVERSATIONS { req, session, conversations, next? }` and `SPAWN_AGENT`'s optional `start`; refused by name below protocol 4 as the other agent messages are; verify with transcript fixtures for each message, both forms of the choice, each refusal and an old `SPAWN_AGENT` unchanged, decoding errors, and fuzzing.
- [ ] 1.4 Add `Listed`, `Request::List` and the list capability to the ACP forms; verify `cargo test -p brindle --test architecture` passes.

## 2. Pure code in the server

- [ ] 2.1 Encode `session/list { cwd, cursor }` and decode its answer entry by entry from the raw JSON (an unreadable entry lost alone), and read `sessionCapabilities.list` from `initialize`; verify with unit tests pinning the request and over the captures of 1.1.
- [ ] 2.2 Extend the reducer with `Start::Choose` and `Phase::Choosing`: no `session/new` after `initialize`; `AgentState::Choosing`; a choice of an id continuing as `Start::Continue { log: Fresh }` does by each `Continuation`, a choice of none sending `session/new`; `NotChoosing`; a prompt refused `NotReady`; a login needed by a list or by the chosen start returning to `Choosing` or to that start; a refused continue going back to `Choosing` with the agent's message; verify with unit tests for each.
- [ ] 2.3 Add the list step: accepted in `Ready` and `Choosing`; `NotOffered`, `NotReady`, `Busy`, the agent's error; the bounds (256 entries, titles one line of 256 characters, ids, times, cursors); entries of another directory dropped; `next` passed through; verify with unit tests for each.
- [ ] 2.4 Extend `acp/checkpoint.rs` (format 2, amended) with the phase and the pending list request; verify a choosing session round-trips, and a checkpoint from before this change decodes as before.

## 3. Server roles

- [ ] 3.1 Handle `SPAWN_AGENT`'s `start`, `AGENT_LIST` and `AGENT_CHOOSE` in the core: gather, call the reducer, write the agent's line, answer `OK`, `AGENT_CONVERSATIONS` or `ERROR`; verify with real-role tests through the fake ACP agent: a chooser lists and then continues a conversation in the same process (one spawn); a list through a ready agent; a choice of none; each refusal's text; paging with a cursor; a chooser closed without a choice leaving no session; a login during a list.
- [ ] 3.2 Add the restart cases: a choosing session keeps choosing across a core restart and an upgrade, and a choice is then carried out; a list in flight at a restart fails with the connection's error.
- [ ] 3.3 Give the fake ACP agent what the cases need (offering `list`, answering `session/list` with pages and with an error) through its existing script steps, adding a step only if a case can't be written without one.

## 4. Brindle's model

- [ ] 4.1 Extend `src/agent/shared.rs` and `transcript.rs` for `AgentState::Choosing`; add the client's `list` and `choose` requests and the choosing start, with the protocol gate naming the endpoint; verify with unit tests.
- [ ] 4.2 Add the pure plans: `lister_plan`, `list_dir` (by `zones-mixed-tabs`' split rule), `open_plan`, `latest_plan` and `ago(now, text)`; verify with unit tests for: a ready zone of the profile in the directory chosen as lister and one in another directory not; each of `list_dir`'s cases; `open_plan` for a conversation already open, a stopped zone, a choosing zone, a running agent and a shell; `latest_plan` for an empty list; `ago` for minutes, hours, yesterday, a date, and text that doesn't parse.
- [ ] 4.3 Add the choosing screen's parts to `src/agent/model.rs` (design decision 6's table: the reason and the buttons for each state, the profile gone offering none), with no working row and no attention while choosing; verify with unit tests for each row of the table.
- [ ] 4.4 Add the conversations palette's model (states, rows, "More…", the 1,024 bound, the "open" mark, the question's rows and their disabling) and the dump lines; verify with unit tests for each state and row form and escaping.

## 5. The zone and the workspace

- [ ] 5.1 Add `agent_conversations` and `agent_continue_conversation` to `ACTIONS` (no default keys), listed as design decision 7 says, and rename `agent_continue`'s palette name to "Continue This Conversation"; verify with unit tests of the palette's listing in a shell, in an agent zone, with no agent profile and in a stopped zone.
- [ ] 5.2 Build the palette in `src/workspace.rs`: the profile question when several, the chooser's start and the list request off the UI thread (`process R13`), "More…", escape closing a chooser that was started for it, and closing it with the window and on quit.
- [ ] 5.3 Carry out a choice by `open_plan`: show the zone that has it; take it in the zone (`AGENT_CHOOSE`, or a stopped zone reset onto the new session with the old one closed); or ask, then split with the profile through `zones-mixed-tabs`' split or open a tab, the new zone attaching the chooser or starting a continuing session.
- [ ] 5.4 Draw the choosing screen and a stopped zone's buttons in `src/agent/view.rs`, as keyboard stops after the existing one, wired to the palette and to New conversation.
- [ ] 5.5 Leave a choosing zone out of the session state and the detached layout, and end its session on quit, as an ended agent's is.

## 6. The command line

- [ ] 6.1 Parse `-c/--continue` and `-r/--resume` under `agent` in `src/main.rs`, with their usage errors; carry the choice in the hand-off request as an optional field, and refuse, with status 1 and the message, a hand-off to a running Brindle that doesn't know it; verify with unit tests of the parser and of the hand-off's encoding and its version rule.
- [ ] 6.2 Start the tab's zone as a chooser; for `-c` ask for the first page and apply `latest_plan`; for `-r` open the palette on the zone once it is choosing; `-p` sent on the first `Ready`.

## 7. End-to-end cases (fake ACP agent)

- [ ] 7.1 `agent-conversations-list`: the palette from an agent zone and from a shell, its title, rows, an untitled row, "More…", the empty directory, and a second agent profile's question.
- [ ] 7.2 `agent-conversations-open`: from a shell, Split Right and New Tab; from a running agent zone, the question; a conversation already open showing its zone; escape at the question returning to the list; one spawn of the agent's command for a list and its choice.
- [ ] 7.3 `agent-conversations-stopped`: an exited agent's zone shows the three buttons; Conversations… and a choice replace it in place; New conversation starts over; a profile removed from the config offers none.
- [ ] 7.4 `agent-conversations-cli`: `-c` continuing the first listed conversation; `-c` with none showing the choosing screen and its reason, then New conversation; `-r` opening the list, a choice taken in the tab, and escape leaving the choosing screen; `-c -r` as a usage error; a quit while choosing leaving nothing to restore.
- [ ] 7.5 `agent-conversations-failures`: an agent without a list; an agent's error on `session/list`; a refused continue returning a choosing zone to the choosing screen.
- [ ] 7.6 Run one `windows-agent-*` case with `-c`, and the agent cases in the archive jobs.

## 8. Docs and checks

- [ ] 8.1 Update `README.md` (the Agent tabs section and the launch options), `docs/code-map.md`, and `CLAUDE.md` (the dump's agent states, the palette lines and focus stops, the launch options).
- [ ] 8.2 Tell `agents-acp-v2` what it inherits: add to its proposal that `session/list` is unchanged in v2 and that a choice continues through its `session/resume`.
- [ ] 8.3 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate agents-session-list --strict`.
- [ ] 8.4 Manual checks for the user, with `scripts/run-throwaway.sh` and real agent profiles: each agent's list, with titles and times that read right; `brindle agent NAME -c` and `-r` in a project with conversations and in one with none; a conversation chosen from a shell into a split in the same directory; pointer presses on the choosing screen's buttons and on the palette's rows.
