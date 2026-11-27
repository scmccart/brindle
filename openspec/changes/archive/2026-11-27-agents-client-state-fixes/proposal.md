# Proposal: agents-client-state-fixes

## Why

The 2026-11-26 agent review (`docs/architecture/audits/2026-11-26-agents.md`, batch report `docs/architecture/audits/2026-11-26-agents/d.md`) found that an agent zone's client keeps facts in two places that nothing orders, and refuses or forgets work when they disagree. The worst is **D-1** (high): a launch's first prompt is refused as "the agent session isn't connected; nothing was sent" while the zone shows the agent ready. The reader thread folds the agent's `Ready` before the session thread has installed the running session, and `Requests` can only say `None`. This is the known `agent-queue` and `agent-continue-profile-gone` e2e flake, and the same window exists when a user presses enter as soon as an attached zone's replay shows the agent ready. The other findings in this change are smaller faults in the same layer:

- **D-4:** a prompt written just before the link drops is shown as refused, and sending it again duplicates it.
- **D-5** (*plausible*): a page fetch still queued when the link drops is carried to the next link. Its page can fold into the new replay and break the session.
- **D-6:** a new conversation in a stopped zone keeps the old log's thought and tool-row toggles.
- **D-7** (*plausible*): the Earlier row's fetch is recorded twice, and a respawn that doesn't start can leave the row loading forever.
- **D-10:** live events folded just after a replay inherit its "from a replay" flag. They then raise no notification, and a first prompt becomes a draft.
- **D-11** (in part): a conversation chosen while the chooser can't send is dropped with no word.
- **D-12** (*plausible*): the terminal-login withheld list misses `pwsh`/`powershell -Command` and `wsl`.

D-1 is one of the three findings the review says to fix first. The rest share its cause: the client model decides from state it can't see whole.

## What Changes

- **The zone knows when its session runs.** The session thread tells the zone's sink once it has installed the running session, for a spawn and for an attach. This is a life event, so the sink's channel delivers it after everything the reader delivered before it. Until then the model doesn't send the first prompt, and the prompt input says the zone is connecting. A launch's `-c` list also waits for it.
- **`Requests` says why nothing was sent.** Each request returns `Result<RequestId, NotSent>`, with `NotSent::{Starting, Gone, LinkDown}`, in place of `Option<RequestId>`. The model decides what each caller does with each case: a send that meets `Starting` is held and never refused. `Gone` is refused as today, and `LinkDown` is how a fetch fails while the link is down.
- **A prompt whose answer was lost with the link isn't called refused.** The core's reply now says whether a request failed or its answer was lost with a link (`ReplyError::{Failed, Unanswered}`). A prompt whose answer was lost waits for the reconnect's replay. If the replayed log (after the zone's last entry at send time) or the queue slot holds its text, as the core cleaned it, the input empties. If not, the refusal shows then. If the replay can't tell, because the page doesn't reach back far enough, the zone says the prompt may have been sent. The core's `clean` moves to `brindle-transcript` so both sides use one definition.
- **No fetch crosses a link.** When the link drops, the connection also fails page fetches that were queued but not yet written, and the writer drops their frames.
- **One record per fact in the zone's model.** A fetch in flight is recorded only in the request ledger, and the Earlier row's loading state is derived from it. The first prompt's state is one enum. A new session's log clears the thought and tool-row toggles.
- **Replays and live events are told apart within one batch of changes.** `Changes` records where the newest turn or agent-state event came from (`StatusFrom::{Unchanged, Replay, Live}`). Notices, the first prompt and a launch's choice are decided from that.
- **A choice that can't go says why.** `ChatModel::choose` returns `Choose::{Send, Refused(text)}`, and a choosing zone shows the refusal as its reason.
- **More commands withhold terminal logins** (choice C5): `pwsh` with `-Command` or `-EncodedCommand`, `powershell` without `-File`, and `wsl` without `--exec`. The rule stays a pure function, tested on every platform.
- **A debug-only fault switch**, `BRINDLE_TEST_SESSION_START_DELAY`, delays a session thread between the core's answer and the running session. Two new e2e cases use it to reproduce D-1 on a spawn and on an attach. The fix is shown by failure counts before and after.

There is no wire change, no config change and no state file change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-zones`:
  - "A launch's first prompt": sent once the session runs; never refused because the session was still being set up.
  - "The prompt input": enter in a zone still being set up keeps the text, with no refusal.
  - "A disconnected endpoint": a prompt whose answer was lost with the link is settled by the reconnect's replay.
  - "Earlier messages": a fetch queued when the link drops fails and never breaks the session; a new session in the zone drops a loading row.
  - "Replays raise nothing": live events right after a replay still raise their notifications.
  - "Opening a tool call row": a new conversation in the zone forgets the old rows' toggles.
  - "A choice that can't be continued": a choice that can't be sent is said.
  - "Terminal logins are withheld from commands that re-parse arguments": `pwsh`, `powershell` and `wsl` forms are added.

## Impact

- **App, pure:**
  - `src/agent/model.rs`: `NotSent`, `ReplyError`, `SessionHandle`, `FirstState`, the `Uncertain` prompt and its reconciliation, `Choose`, the paging state, and `respawning`.
  - `src/agent/shared.rs`: `Changes::status_from`.
  - `src/agent/login.rs`: `reparses`.
- **App, shell:**
  - `src/agent/mod.rs`: the `Requests` trait, its callers and `AgentEvent` handling.
  - `src/agent/sink.rs`: `AgentEvent::SessionRunning`, and `Reply`'s outcome type.
  - `src/remote/local/agent.rs`.
  - `src/remote/local/session.rs`: `run`, `Session::running`, the fault switch.
  - `src/remote/local/connection.rs`: `SessionSink::reply` and `running`, `Waiting::Agent`'s `Carry`, `State::link_lost`, `Running::fetch`.
  - `src/remote/mod.rs`: `AgentLink`, the switch's name.
  - `src/main.rs`: `fault_switches`.
  - `src/agent/view.rs`: reading the Earlier row.
  - `src/workspace.rs`: `open_here`, unchanged in shape.
- **Shared crate:** `brindle-transcript` gains `clean`, moved from `remote-agent/src/acp/reduce.rs`, with its tests. The helper calls it from there, and its behaviour is unchanged.
- **Tests:**
  - `src/agent/test_support.rs`'s `FakeRequests` answers with a `NotSent`.
  - `tests/e2e/switches.rs` gains `TestSwitch::SessionStartDelay`.
  - `tests/e2e/cases/agent.rs` and `cases/mod.rs` gain two cases.
- **Docs:** `docs/code-map.md`'s `agent/` and `remote/` entries, and `CLAUDE.md`'s fault-switch and agent-case lines.

## Overlap

- **`agents-commands` (PR #151, open, merges first).** It changes `remote-agent/src/acp/reduce.rs` (where `clean` lives), `src/agent/model.rs` (new state beside `Sent`, the prompt input's suggestions) and the view. This change is implemented after it merges, and its first task re-opens every cited site on `main`. A slash command Brindle carries out itself never becomes a `Sent::Prompt`, so it is outside D-4's reconciliation.
- **`agents-prompt-context` and `agents-prompt-images` (proposed).** A prompt gains attachments, and the user entry and slot gain a list. D-4's match compares the prompt's cleaned text. When those changes land, the match must also compare the attachment list. Each of their designs is told so in this change's design (decision 5).
- **`agents-plans` (proposed).** Plans are new session state in the replay. They don't affect `StatusFrom`, which only turn and agent-state events set.
- **Sibling remediation changes:**
  - `agents-zone-focus-and-scroll` changes the view's Earlier row and scroller. This change only changes where the row's loading state comes from (`ChatModel::earlier`). Both changes modify `agent-zones` "Earlier messages", each adding its own scenarios: whichever is archived second carries the other's text in its delta.
  - `agents-event-bounds`, `agents-process-lifecycle` and `agents-checkpoint-deltas` are server-side and don't touch these sites.
  - `agents-module-structure` later moves the ledger, the first prompt, paging and choosing out of `model.rs`. The types this change adds (`NotSent`, `ReplyError`, `FirstState`, `Uncertain`, `Choose`) move with them, and it takes the rest of D-11.
  - `agents-absorb-cost` reworks `ChatModel::absorb` into two halves. This change's reconciliation sits in the reset branch, and `StatusFrom` in `Changes`, which both halves keep.
