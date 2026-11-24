# Proposal

**Depends on:** `agents-launch-commands` (the subcommand form, `TabKind`, `ProfileKind`, the launch's kind check and the hand-off's `kind` field, which `brindle agent` joins) and `agents-sessions` (agent sessions in the terminal server, the `brindle-transcript` crate's events and their fold, client protocol 4, and the `brindle-fake-acp-agent` test binary), both implemented and archived.
**Next:** `agents-markdown`, rendering markdown in agent messages. The five follow-ups (`agents-markdown`, `agents-login`, `agents-session-recovery`, `agents-log-paging`, `agents-acp-v2`) each depend only on this change and are independent of one another, so that order is a suggestion.

Vocabulary: "ACP agent", "agent session" and "agent zone" are the new thing; "the helper" and "the terminal server" are `brindle-remote`.

## Why

After `agents-sessions`, the terminal server can run an ACP agent (Agent Client Protocol) and keep its conversation, but nothing in Brindle can start one or show it: Brindle's offer is pinned at protocols 1–3, so it never sees an agent session. Users want to keep an ACP agent beside their terminals, in a zone of its own, chatting with it and answering its permission requests, with the same lifetime terminals have: it survives a quit or a crash and comes back at the next launch. `agents-launch-commands` reserved `brindle agent` for this change.

## What Changes

- **A profile can be an agent profile:** `type = "agent"` in a `[[profiles]]` table (the default is `type = "terminal"`), with `command`, `args`, `cwd`, `env`, `theme`, `font_size` and `remote` as for terminals. An agent profile must name a command. Its description in the profile menu and picker reads `agent · <command>` (after the remote target, if any). With a `remote` target the ACP agent runs on that endpoint, through the terminal server there.
- **`brindle agent [PROFILE] [-d DIR] [-p PROMPT]`** opens an agent tab, added to `agents-launch-commands`' subcommand form as that change prescribes: `-p` gives a first prompt, sent once the agent is ready and never again (dropped if Brindle quits first), and `-p -` reads it from standard input. Without `-d` or a profile `cwd`, the agent starts in the `brindle agent` command's working directory (an agent tab opened from the profile menu or picker starts in the home directory). An unknown profile name, or a profile of the other kind, under either subcommand is a usage error (status 2, listing the profile names), never a fallback; bare `brindle` opens the default profile whatever its kind; `brindle terminal` and `brindle agent` without a profile open the default profile when it is of their kind, else the first profile of their kind. The hand-off to a running Brindle carries agent launches in their own field.
- **Agent zones:** opening an agent profile opens a tab whose zone shows the agent session's conversation instead of a terminal. A tab holds zones of one kind: a split of an agent tab starts another agent session in the split zone's directory.
- **The conversation:** user and agent messages (plain text), thoughts (collapsible), tool calls as one-line status rows, permission requests as a row of the agent's options operable from the keyboard, and notices. It scrolls, sticks to the bottom while new text streams in, and offers a jump back to the bottom.
- **The prompt input:** a multi-line text area (enter sends, shift-enter adds a line, IME included) with a send button that becomes a stop button while a turn runs; escape stops the turn. A prompt sent during a turn waits in the terminal server's queue slot (one at most), shown above the input with a remove button, and is sent when the turn ends normally. A turn that is stopped, fails or ends any other way, or an agent that stops, holds it instead: the row shows it as not sent, with why, a send-again button and a discard button, and it survives a quit. Nothing from the slot ever goes into the prompt input; a further prompt during a turn stays in the input.
- **Closing asks when the agent is mid-turn:** closing an agent zone, or a tab holding one, whose agent is working or waiting for permission asks for confirmation first; otherwise closing ends the agent at once. Quitting Brindle never asks and leaves agents running.
- **Every failure is the zone's state:** an agent that can't start (on Windows, with the `cmd /c …` form when the command is a batch file), a terminal server too old for agent sessions, a disconnected endpoint, an agent that needs a login (done through its own command line), an agent that exited (with its status and the end of its standard error), and a session Brindle can no longer show. An agent zone whose agent exited stays open, saying so, until the user closes it.
- **Status and notifications:** an agent zone's activity (`idle`, `working`, `blocked`, `done`, `error`) drives the zone header's and the tab's status dot as a program status record does, and raises desktop notifications for `blocked`, `done` and `error` under a new `notifications.agent` setting.
- **Lifetime as terminals:** quitting Brindle leaves agent sessions running, detached with their tab's layout; the next launch restores or reattaches them, replaying their conversation without notifications. The sessions palette lists detached agent sessions, marked as agents.
- **Brindle offers client protocol 4,** replacing the 1–3 pin `agents-sessions` put in, once it can show agent sessions.
- **The settings dialog** lists agent profiles, marked as agents, and never alters or drops them; editing one is config-file only (Edit and Duplicate are unavailable for it; Delete, reorder and the default marker work).
- **Components in `brindle-ui`,** modelled on shadcn/ui's AI Elements and ported to GPUI: a multi-line `TextArea` (sharing the single-line input's editing core) and its `Textarea` frame, a `MessageScroller` with its scroll button, `Message`, `PromptInput` (with its queue row, waiting or not sent), `Reasoning`, `Tool`, `Confirmation` and `Alert`. The close confirmation uses the existing `Dialog`.
- **Debug hooks reach agent zones:** `--dump-screen-after` prints the agent zone's state, its entries, its queued or held prompt and its prompt, and the close confirmation; `--send` types into the prompt input, `--key` drives it, the queue row, the permission rows and the confirmation, `--wait-for` matches the conversation, and new actions (`agent_submit`, `agent_cancel`, `agent_unqueue`, `agent_resend`, `copy_queued_prompt`, `copy_last_agent_message`, `toggle_agent_thoughts`, `agent_scroll_to_bottom`) are scriptable.
- **E2E cases against `brindle-fake-acp-agent`,** found through `BRINDLE_FAKE_ACP_AGENT` or beside the suite's own build and checked for staleness, so they run against debug and release builds and the packaged archives; CI's archive jobs build it first.

Not in this change, each deferred to its own proposal-only change: markdown (`agents-markdown`), in-zone login (`agents-login`), recovery after an agent exits (`agents-session-recovery`), paging the conversation log (`agents-log-paging`), ACP v2 (`agents-acp-v2`). Also out: ACP `fs` and `terminal` capabilities, modes and slash commands, mixing agent and terminal zones in one tab, find and text selection in a conversation, and creating or editing agent profiles in the settings dialog.

## Capabilities

### New Capabilities

- `agent-zones`: the zone that shows an agent session. It covers opening one from an agent profile, the conversation and how it scrolls, the prompt and its queue of one, permission and tool rows, what each failure looks like, the activity-to-status mapping, the keyboard path through the zone, closing with confirmation, and splitting, quitting, restoring and attaching agent zones. A capability of its own because it owns behaviour terminals don't have and that will grow (`agents-markdown`, `agents-login`, `agents-session-recovery`); `zones` keeps what every zone shares.

### Modified Capabilities

- `profiles`: agent profiles (`type`, the command they need, their description).
- `zones`: a split starts a zone of the tab's kind; the header menu's split items are disabled where the tab can't start one; closing an agent zone mid-turn asks first; an agent zone whose agent exits stays open.
- `tabs`: closing a tab with an agent mid-turn asks first; the status dot counts agent zones' activity.
- `session-restore`: restoring and reattaching agent sessions, and keeping a tab's zones of one kind.
- `command-palette`: the sessions palette lists agent sessions and opens them as agent tabs.
- `desktop-notifications`: agent notifications and their `notifications.agent` setting.
- `settings-dialog`: agent profiles in the profile list, kept intact.
- `command-line`: `brindle agent`, written on top of `agents-launch-commands`' requirements (the launch options, what a launch opens, the hand-off and its versions, the profile check, with an unknown name refused; the reservation of `agent` is removed), a first prompt from standard input, the notification log's `agent` source, and the debug hooks in an agent zone.
- `ui-components`: the conversation components and the `Alert`.
- `text-input`: multi-line text areas.

`terminal-server` and `agent-sessions` are `agents-sessions`' and are not changed here. `program-status` keeps its records; an agent zone's activity is mapped onto its states in `agent-zones`. `configuration` is unchanged: an agent profile is a profile.

## Impact

- **`brindle` (app):** `main.rs` (`Asked::Agent`, `AgentLaunch`, `TabKind::Agent`, the `agent` arm, help text), `instance.rs` (the request's `agent` object, `Forward::Agent`, transcript tests), `config.rs` (`ProfileKind::Agent` and the `type` key), a new `src/agent/` (the pure spawn request, the chat model and shared fold, the `Agent` entity, the `AgentView`), `zone_view.rs` (`ZoneContent::Agent` and every caller of `as_terminal_view`), `workspace.rs` (launch, `profile_for`, split, close with confirmation, restore, sweep, dump, focus, `--send`), `remote/local/` (agent sessions through the client, the sink, the offer raised to 4), `remote/mod.rs` (one gated item), `reattach.rs`, `layout.rs`, `picker.rs` (sessions palette rows), `settings_dialog/`, `config_edit.rs`, `actions.rs`, `notify/`.
- **`brindle-term-core`:** `notice::Source` gains `Agent`.
- **`brindle-transcript`:** used, not changed. Brindle never depends on the ACP schema crate, which lives in `brindle-remote-agent`.
- **`brindle-ui`:** `text_input.rs` (`EditState` lines mode), new `text_area.rs`, new components, `icon.rs`, the gallery and `ui/README.md`. Its closed dependency list (`gpui`, `log`) is unchanged.
- **`brindle-remote-protocol`:** used, not changed (client protocol 4, `SessionInfo.agent`).
- **Tests:** unit tests beside every model; `tests/e2e/cases/agent.rs`, the fake agent lookup and freshness check in `tests/common/`, Windows counterparts in `cases/win.rs`, a remote case in `cases/remote.rs`; CI's `e2e-archive` and `windows-e2e-archive` jobs build the fake agent and run the agent cases.
- **Docs:** `README.md` (agent profiles, `brindle agent`, keys, actions, the release note that an older Brindle rejects a config with an agent profile), `docs/code-map.md`, `CLAUDE.md` (dump lines, fake agent, manual checks), `ui/README.md`, `docs/architecture/functional-core-imperative-shell.md`'s pure-module table and `process-architecture.md` R9's queue list.
- **No new dependency.** No wire change of Brindle's own beyond the instance request's additive `agent` object. The state file is unchanged.
