# Batch D: Brindle's client-side agent model and its thread seam

## Coverage

Read in full (production code):
- `src/agent/sink.rs`, `src/agent/shared.rs`, `src/agent/mod.rs`
- `src/agent/model.rs` lines 1–3293 (production; tests skimmed only for the `Zone` helper)
- `src/agent/spawn.rs`, `login.rs`, `close.rs`, `tool.rs`, `transcript.rs`, `conversations.rs` (production parts)
- `src/remote/local/agent.rs`
- `src/remote/local/session.rs` lines 1–516 (all production)
- `src/terminal/pace.rs` (production and its tests)

Read in part:
- `src/remote/local/connection.rs`: `State::route_spawned`/`route_replay_start` and `link_lost` (900–971), `ask`/`withdraw`/`spawn`/`spawn_agent`/`attach` (1380–1506), `handle` (1782–2032), `route`, `note_session`, `sink`, `deliver`, `break_session`, `displaced_lost` (2034–2136), the writer's send loop and `mark_written` (2450–2590), `Running` whole (2870–3158).
- `src/remote/mod.rs`: `AgentStart`, `AgentLink`, `agent_link` (grep hits, 40–250).
- `transcript/src/lib.rs`: `entries`/`entry`/`window` (1300–1340), `activity` (1544–1570), the constants.
- `src/workspace.rs` 1640–1790 and `remove_tab` (2053–2080); `src/zone_view.rs` `replace_agent` and the close/quit dispatch (786–796).
- `src/agent/view.rs` only `AgentView`'s fields (reviewer E owns it).
- `remote-agent/src/server/core/sessions.rs` `session_spawned` (905–963), to see what follows `SPAWNED`.
- `tests/perf/agent.rs` head; `tests/e2e/cases/agent.rs` `queue` and `continue_profile_gone`.
- Archived designs: `agents-markdown` (decision 6 and its as-built notes), `agents-profiles` (memory line), `agents-login` (decision 9).

Skipped: `markdown.rs` (not in the batch), the fold itself (reviewer B), the view's drawing (reviewer E).

## Findings

### D-1: the first prompt is refused as "not connected" when the agent's `Ready` folds before the session thread installs `Phase::Running` (the `agent-queue` / `agent-continue-profile-gone` flake)
- Severity: high
- Confidence: confirmed for every hop of the code path; the timing (UI absorbing `Ready` before the session thread wakes) is the race window, which the e2e flake fits
- Category: race
- Where: `src/remote/local/connection.rs:1789-1801` (reader shows the session, then `reply.try_send`, then keeps reading); `remote-agent/src/server/core/sessions.rs:945-962` (`SPAWNED`, then `agent_step(Input::Spawned)` at once); `src/remote/local/connection.rs:1424-1431` and `1494-1495` (session thread wakes from `recv`, removes the request, builds `Running`); `src/remote/local/session.rs:466-504` (`run` only then takes the phase lock and sets `Phase::Running`); `src/remote/local/session.rs:335-347` (`Session::running` returns `None` in `Phase::Starting`); `src/agent/model.rs:2297-2302` (`settle_first` gives `First::Send` on the first live `Ready`); `src/agent/mod.rs:767-774` (`requests.prompt` → `None` → `model.unsent(FirstPrompt)`); `src/agent/model.rs:2596-2598, 2656-2658` (refusal `UNSENT`, `SetDraft`).
- Scenario: a launch with `-p one` (or a restored `first_prompt`) against an agent that answers `initialize` and `session/new` quickly (the fake ACP agent's `Script::ready`). The reader thread handles `SPAWNED`: it puts the sink in `state.sessions`, hands `Started` to the session thread through the sync channel and goes straight on reading. The core sends `initialize` right after `SPAWNED`; the agent's answers come back as `TRANSCRIPT` frames (`Starting`, `Conversation`, `Ready`), which the reader folds into `AgentShared` and wakes the zone. If the UI thread's drain absorbs `Ready` before the session thread has been scheduled, woken from `recv`, taken `state` to remove the request, and taken the phase lock (all under CI load), `Session::running` sees `Phase::Starting` and returns `None`. The zone then shows `state=ready` with `prompt status=ready refusal="the agent session isn't connected; nothing was sent"` and the first prompt as a draft, exactly the flake. The same window exists on an attach (`connection.rs:1856-1861`: `replay(true)` delivered before the reply): a user who presses enter as soon as the replay shows `Ready` gets the same refusal.
- Root cause: two unordered sources of truth for "can send": the transcript's agent state (reader thread) and the `Session`'s phase (session thread). `ChatModel::can_send` sees only the first.
- Fix: make the session's readiness a fact the model orders against. Smallest sound change: after `*phase = Phase::Running(running)` in `run`, tell the sink (`SessionSink::started()` → `AgentEvent::Started`, a life event, so the channel orders it after everything the reader sent before it); `ChatModel` keeps `First` pending and `prompt_status` at `Connecting` until it has seen `Started`. Alternatively have `Requests` return `Result<RequestId, NotSent::{Starting, Gone}>` and hold the first prompt on `Starting` instead of refusing it (`fcis R1`: the hold is the model's decision). Test gap: no unit test drives `First::Send` against a `Requests` that is still starting; `FakeRequests` always sends.

### D-2: `ChatModel::absorb` runs under `AgentShared`'s lock, so Markdown rendering on the UI thread stalls the connection's reader thread, and with it every session on the endpoint
- Severity: medium
- Confidence: confirmed
- Category: performance
- Where: `src/agent/mod.rs:702-711` (the guard from `self.shared.lock()` lives across `self.model.absorb(...)`); `src/agent/sink.rs:178-186` (the reader takes the same lock for every `TRANSCRIPT`, while holding the connection's per-session sink mutex from `Connection::deliver`, `connection.rs:2096-2097`); `src/remote/local/connection.rs:1958-1967` (frames are handled one at a time on the reader thread); `src/agent/model.rs:1690-1707, 1800-1806, 1664-1683` (what absorb does: clones entries, `render_entry` of the whole message, `keep_texts`, `draw`, `refresh_rows`).
- Scenario: all local tabs, terminals and agent zones alike, share one connection to the local server and one reader thread. While an agent streams a long reply, every absorb re-renders the whole message (the design's own number: 12 ms for 256 KiB in a debug build) with the lock held; the reader blocks on that lock at the agent session's next `TRANSCRIPT` frame, so output for every other tab on the endpoint waits too. An attach's reset absorb renders the whole replayed page (`PAGE_BYTES`, 256 KiB) under the lock, and each fetched page renders its 256 KiB under it again (`prepend_entries`). The leaf-lock rule (`process R14`) holds in form, but the work under it is the heaviest in the zone. The `agents-markdown` design's fallback ("render on the reader thread after `apply`, outside `AgentShared`'s lock") assumes rendering already happens outside it; it doesn't.
- Fix: split absorb in two. Under the lock, only take the changes and copy the changed entries' tails (`update_entry`'s copy, `prepend`'s and `reset`'s clones); release; then render, draw and splice from the zone's copy. `ChatModel` already holds every entry it needs, so the second half doesn't need the transcript (`process R14`, `R13`).

### D-3: absorbs are not frame-paced; each re-parses the growing message whole, and the design's bound of "one parse per frame" is off by several times
- Severity: medium
- Confidence: plausible (the code path is confirmed; the multiplier depends on how the core batches frames)
- Category: performance
- Where: `src/agent/mod.rs:668-697` (`drain`: `pace::drain` keeps taking chunks for `BUDGET`, 16 ms, and each `Changed` taken is absorbed); `src/terminal/pace.rs` (`PACE` 4 ms after a burst of 4, so up to ~250 wakes a second); `src/agent/model.rs:1800-1806` (whole re-render per absorb), `1802` (deep `PartialEq` of fresh parts against cached ones), `2016-2012` `refresh_rows` → `refresh_facts` (a `HashMap` over every held entry, two binary searches per entry, on every absorb and life event), `1729` (`transcript.entries().filter(id >= from)` scans from the first entry), `1792` (`*own = source.clone()` for any non-`Appended` dirt, so a tool entry is cloned whole, up to `CALL_LIMIT` 256 KiB, on every `ToolOutput`), `1936-1953` (`tool::detail` rebuilds every detail part, copying the kept output, for each redraw).
- Scenario: the merged `Changed` bounds absorbs per *wake*, not per frame. Inside one `drain` call the gate re-arms as soon as `took` runs, so a `Changed` queued while the zone absorbs is taken by `now_or_never` within the same 16 ms budget and absorbed again; across wakes the pace allows ~250 a second. With a real agent trickling chunks (Claude, Codex), each arriving `TRANSCRIPT` frame can cost one parse of the whole message so far: a 64 KiB reply in 2,048 frames is up to 2,048 parses averaging 32 KiB (64 MiB of Markdown), against the design's "under 1,200 parses … about 40 MB" for 20 s at 60 a second. Combined with D-2 the reader and the UI ping-pong on the lock, one frame per absorb. The perf case doesn't see it because the fake agent bursts and the core coalesces the burst into a few frames (`agents-markdown` as-built: "measures that burst, not a reply trickling in"). The same shape applies to `agent-tool-output`: every absorb clones the tool entry and rebuilds its 64 KiB of detail lines, then compares them.
- Fix: absorb copies text and marks entries dirty; render the dirty entries once per paint (`cx.on_next_frame`, or the first `Changed` of a drain call doing the render after the drain loop ends), keeping replies ordered after the absorb that precedes them. For tools, add `Dirty::Output` handling that appends to the zone's copy as `Appended` does, and redraw only the last output part. Test gap: no unit test pins the number of renders per frame of streamed input; the perf case needs a trickle mode (chunks spaced over seconds) to measure the real case.

### D-4: a prompt written just before a link drop is reported "not sent" though the core may have taken it, and sending it again duplicates it
- Severity: medium
- Confidence: confirmed for the client path; the core receiving the frame before the link died is the assumed (ordinary) hop
- Category: correctness
- Where: `src/remote/local/connection.rs:915-933` (`link_lost` fails every written request); `src/agent/model.rs:2670-2673` (`Sent::Prompt` refused → `refusal`, `Replied::Nothing`, so the input keeps the text); `src/agent/model.rs:1690-1708` (the reconnect's reset absorb doesn't reconcile `pending` or the refusal with the replayed transcript).
- Scenario: over SSH, the user presses enter; `AGENT_PROMPT` is written and reaches the relay; the link drops before `OK`. The zone shows "the connection to the terminal server is down" beside an input still holding the prompt. On reconnect, the replay shows `user "…"` and a running turn (or the prompt in the queue slot). The input still holds the text and says it wasn't sent; pressing enter again sends it a second time (queued behind the first during the turn). A permission answer at the same moment is safe (the core refuses a second answer), a prompt isn't.
- Fix: keep a prompt whose reply failed with the link as `Sent::Uncertain(text)` instead of refused, and on the next reset decide in the model: if the replayed transcript's newest user entry after the zone's last known one, or the slot, holds the text, clear the draft (`Replied::ClearDraft`); else show the refusal then. A protocol-level fix would be a client-chosen prompt id the core deduplicates, which is a new negotiated version (`process R4`).

### D-5: an `AGENT_FETCH` still queued when the link drops is carried to the next link, and its page can fold into the replayed transcript and break the session
- Severity: medium
- Confidence: plausible (assumed hop: the carried fetch is written on the new link after the core's reattach replay starts, which depends on the new link's ordering)
- Category: race
- Where: `src/remote/local/connection.rs:3089-3105` (`Running::fetch` checks `link.is_up()` and queues); `src/remote/local/connection.rs:2521-2537` (a failed write marks the request unwritten and retries on the next link); `src/remote/local/connection.rs:915-933` (`link_lost` fails only written requests); `src/agent/sink.rs:224-228` (`replay(true)` drops only fetches that already had frames, `self.pages`); `src/agent/shared.rs:237-251` (`prepend` folds whatever comes); `src/agent/model.rs:1690-1692` (the model forgets the fetch on reset, but the sink still folds its page).
- Scenario: the user scrolls up twice, so the zone holds the newest page plus one fetched page and asks for a third (`before` = the zone's window start). The write fails as the link dies (`mark_written(false)`), so the request stays `Queued`. On reconnect the core replays the newest page only, so the fold's window starts later than the fetch's `before`. The carried `AGENT_FETCH` goes out on the new link, its `AGENT_PAGE` matches the still-waiting `Waiting::Agent`, the sink isn't dropping it (it had no frames when the replay started), and `Transcript::prepend` refuses the gap. The session is broken ("a page from the terminal server couldn't be applied") and detached, for a scroll during a reconnect. `Running::fetch`'s own comment states the intended rule: a fetch is "answered at once with the link's failure, rather than held for the next link (whose replay starts the zone over anyway)". The queued case breaks that rule.
- Fix: in `State::link_lost`, also fail queued (unwritten) `Waiting::Agent` requests that are fetches (or tag `Waiting::Agent` with whether it may be carried), so they get `LINK_DOWN` and the writer's `mark_written` drops the frame. Alternatively the sink drops every page whose request predates the latest `replay(true)`; the sink would need to see fetch ids at send time. Test gap: no test of a fetch whose write fails as the link drops.

### D-6: a continue in a new session, New conversation and a listed conversation taken in a stopped zone keep the old session's thought toggles and tool rows opened by hand, keyed by `EntryId`
- Severity: low
- Confidence: confirmed
- Category: correctness
- Where: `src/agent/model.rs:1690-1708` (the reset branch clears `rendered`, `texts`, `auto_open`, not `toggled` or `by_hand`); `src/agent/model.rs:2224-2234` (`respawning` clears neither); `src/agent/model.rs:1713-1717` (only a trim prunes `by_hand`; `toggled` is never pruned); `src/agent/mod.rs:542-590` (`renew`).
- Scenario: in the old conversation the user opened tool row 5 and closed thought 3 by hand. They press New conversation; the new session's log starts its ids at 1. When the new session's entry 5 is a tool call, it opens regardless of `[agent] tool_details`; when its entry 3 is a thought, it shows closed while it streams. A replay of the *same* session keeps the ids, which is why keeping them on a reset is right there; a new session isn't the same log.
- Fix: clear `toggled` and `by_hand` in `respawning()` (the model's own "a new log" transition), and prune `toggled` with `by_hand` on a trim.

### D-7: the Earlier row's fetch is recorded twice (`Earlier::Loading { request }` and `pending[request] = Sent::Fetch`); a respawn clears one and not the other
- Severity: low
- Confidence: plausible (assumed hop: the view keeps the loader and the motion clock running while the model says `Loading`, as `docs/code-map.md` describes `refresh_motion`)
- Category: types
- Where: `src/agent/model.rs:1602-1609` (`fetching` writes both); `src/agent/model.rs:2224-2234` (`respawning` clears `pending`, leaves `earlier`); `src/agent/model.rs:1625-1638` (`fetched` only acts while `Loading` with that request); `src/agent/model.rs:1588-1591` (an exited agent is still `Live`, so its zone fetches).
- Scenario: the agent has exited (the server keeps the session, `Live(Exited)`), the user scrolls up so a fetch is in flight, and presses Continue conversation when the server no longer has the session (`Offer::Respawn`). `respawning` drops the fetch from `pending`; `earlier` stays `Loading`. If the new session doesn't start (`Agent::renew`'s connect fails, or `SPAWN_AGENT` is refused, `NotStarted`), no change ever resets the window, and the Earlier row loads forever with its spinner, which keeps the motion clock (and repaints) running. The same pair of facts shows elsewhere in the model: `slot` and the cached `queue` (patched by hand in `answered`'s `Restart` arm and in `unqueue`), `first` and `first_shown`, `terminal_login` beside `pending`'s `Sent::Login` and `login_note`.
- Fix: make the ledger the only record: `Earlier::Loading` without a request id, derived from "a `Sent::Fetch` is pending", or `respawning` setting `earlier = Earlier::None` and `window = None`. Fold `first`/`first_shown` into one `enum FirstPrompt { None, Pending(text), Shown(text) }` (`types R1`).

### D-8: an overflow on a hunk's line number from the core panics the UI thread in debug builds, crashing Brindle rather than breaking the session
- Severity: low
- Confidence: confirmed
- Category: errors
- Where: `src/agent/tool.rs:287-305` (`old += 1`, `new += 1` on `u32` from `Hunk::{old_start, new_start}`, `transcript/src/lib.rs:272-273`, not bounded by the decoder); reached from `absorb` → `draw` → `tool_draw` → `shown_parts` → `tool::detail` → `diff_parts` (`src/agent/model.rs:1936-1953`), on the UI thread, outside any `catch_unwind` (only `markdown::render` is guarded).
- Scenario: a core (an older or broken helper on a remote endpoint) sends a `ToolItems` diff with `old_start: 4294967295` and two context lines. The fold accepts it; in a debug build the first open row or pending permission that draws it panics on `old += 1`, which takes down every window (`railway R5`, `process R3`: a fault from one session's bytes should break that session). A release build wraps silently and shows line 0.
- Fix: `checked_add` and stop numbering (`None`) on overflow, or bound `old_start`/`new_start` in the transcript's decoder. The fuzz run (`transcript/src/fuzz.rs`) covers the fold, not the client's drawing of what it accepts.

### D-9: the zone keeps three to four copies of the conversation, against the recorded "at most about twice the core's 16 MiB"
- Severity: medium
- Confidence: confirmed
- Category: memory
- Where: `src/agent/shared.rs:134-138` (`AgentShared.transcript`, the whole log); `src/agent/model.rs:1138` (`ChatModel.entries`, a full copy of every entry); `src/agent/model.rs:1142` (`rendered`, `RichBlock`s holding the text again); `src/agent/model.rs:348` (`UserText` blocks copy user text into new `SharedString`s); `src/agent/model.rs:1185` and `432` (`texts` holds each tool text item's source again, plus its render); `src/agent/tool.rs:358` (an open output's detail parts copy its kept text).
- Scenario: the client fold has no bound of its own; it follows the core's `Trimmed` at `LOG_LIMIT` (16 MiB). A long live conversation, or one paged back to its start, holds about 16 MiB in `AgentShared`, 16 MiB in `entries`, and the agent text a third time in the rendered blocks: about 48 MiB per zone, more with open tool rows. Several agent zones multiply it. `agents-profiles` design.md: "the zone holds a copy beside the fold, so at most about twice the core's 16 MiB" didn't count the render cache added by `agents-markdown` or the tool caches added by `agents-tool-content`.
- Fix: one copy of the log on the client. Either the UI thread owns the only `Transcript` (the reader decodes into `Event`s and hands a `Vec` over a leaf lock, the zone folds them and reports a `FoldError` back through `Requests` to break the session), or `ChatModel` drops `entries` and reads the shared transcript under the lock in its handlers (never in `render`). Either also removes D-2's need to copy under the lock.

### D-10: live events folded right after a replay inherit its `from_replay`, so they raise no notification and a first prompt becomes a draft
- Severity: low
- Confidence: confirmed for the merge; the window is one paced wake
- Category: correctness
- Where: `src/agent/shared.rs:205` (`changes.from_replay |= self.replaying`, one flag for the whole batch); `src/agent/model.rs:1771, 2280, 2299-2300` (`finish` passes it to `notice` and `settle_first`).
- Scenario: a reconnect replays the session; before the UI takes the replay's changes, the agent asks a permission live (or the turn ends live). The one `Changes` holds both, marked `from_replay`, so the zone turns `blocked` with no desktop notification, and a first prompt still pending becomes a draft though `Ready` came live. The window is short (one pace), so this is a latent edge, not a common loss.
- Fix: record the replay's end inside `Changes` (for example `live_from: Option<EntryId>` and a `live_state_change` bit set by events folded after `replay_end`), and decide notices and the first prompt on the live part only.

### D-11: decisions in the `Agent` entity that belong to the model
- Severity: low
- Confidence: confirmed
- Category: placement
- Where: `src/agent/mod.rs:1029-1040` (`quit_with` chooses `CLOSE` versus `DETACH` from `started_choosing() || is_choosing()`); `src/agent/mod.rs:712-716` (`Replay(false)` decides from the link whether it means "back up"); `src/agent/mod.rs:820-832` (`login_zone_ended` decides from `profile_found` what to show); `src/agent/mod.rs:602-611` (`choose` silently does nothing when `!can_choose()`, so a conversation picked while the chooser's link reconnects is dropped with no word, `workspace.rs:1736-1739`); `src/agent/conversations.rs:488-491` (a `_ =>` arm over `(ListState, More)`, Brindle's own enums).
- Scenario: each is a branch on state that only an entity test (a GPUI app) can reach (`fcis R1`, `R10`); the silent `choose` drop is user-visible: the palette closes and the zone stays on the choosing screen.
- Fix: `ChatModel::leave_on_quit() -> Leaving`, `ChatModel::replay_ended() -> Absorbed`, `ChatModel::choose(..) -> Choose::{Send, Refused(text)}` showing why; spell out the tuple's arms (`railway R3`).

### D-12: the terminal-login withheld list misses commands that re-parse their arguments (`pwsh -Command`, `powershell -Command`, `wsl`)
- Severity: low
- Confidence: plausible (assumed: a profile of that shape is in use)
- Category: correctness
- Where: `src/agent/login.rs:148-162` (`reparses` knows a batch file, `cmd /c|/k` and `ssh`); disputes `agents-login` design decision 9's list.
- Scenario: a Windows profile `command = "pwsh"`, `args = ["-Command", "agy", "acp"]`. A terminal method's `--login` and any argument with spaces or quotes are appended to a PowerShell command line and parsed as PowerShell; `wsl agy acp` hands the joined arguments to the distribution's shell. The button shows "Runs with …" as if argv were passed through. There is no privilege gain (the agent already runs as the user on that host), only mangled or surprising login commands.
- Fix: add the `-Command`/`-c` forms of `pwsh`/`powershell` and `wsl` (and `bash|sh -c` when the appended args would land in the script string rather than positional parameters) to `reparses`, with the same pure tests.

## Structural observations

1. **Cut `model.rs` (3,293 production lines, 12 responsibilities) along its state, not its call sites.** Each of these owns its fields and returns its own `Absorbed` part, and `ChatModel` becomes the composition that calls them in `finish`:
   - `agent/phase.rs`: `StartFailure`, `agent_start`, `AgentPhase`, `Link`, `SessionLife`, `failure_text`, `Alert`, `Names`, `batch_form`, `DisabledWhy`, `PromptStatus`, `prompt_status`.
   - `agent/blocks.rs` (the conversation list): `Block`, `BlockBody`, `Splice`, `parts`, `user_blocks`, `blocks_of`, `render_entry`, `text_items`, `ShownPart`, `ToolDraw`, plus the fields `entries`, `rendered`, `texts`, `blocks`, `toggled`, `auto_open`, `by_hand`, `last_user`, `permission_tools`, and `absorb`'s entry half, `update_entry`, `redraw`, `prepend_entries`, `refresh_rows`, `toggle_*`. This is the part D-2, D-3 and D-9 change, and the one that benefits from being alone.
   - `agent/ledger.rs`: `Sent`, `pending`, `sent`/`replied`/`unsent`/`answered`, `Replied`, `in_flight`, and the `Uncertain` state D-4 needs.
   - `agent/queue.rs`: `QueueRow`, `Resend`, `queue_row`, `hold_words`, `hold_word`, `stop_word`, `resend_of`, `unqueue`, `resend`.
   - `agent/prompt.rs`: `Submit`, `Why`, `submit`, `First`, `settle_first`, `first_shown` (with D-7's one enum).
   - `agent/login.rs` gains the row: `LoginRow`, `MethodChoice`, `LoginChoice`, `CancelLogin`, `LoginZoneEnd`, `AfterLogin` and the `terminal_login`/`login_note` fields as one `LoginState`.
   - `agent/recovery.rs`: `Offer`, `NoOffer`, `ProfileFound`, `continue_offer`.
   - `agent/conversations.rs` gains choosing: `ListFor`, `LaunchChoice`, `OnChoosing`, `ListDone`, `ListerState`, `ZoneChoice`, `Choosing`, `listed`, `list_refused`, `without_session`, `zone_choices`.
   - `agent/paging.rs`: `Earlier`, `Fetch`, `fetch_due`, `retry`, `fetching`, `fetch_unsent`, `fetched`, `follow_window`.
   - `agent/attention.rs`: `shown_status`, `seen`, `notice`, `notice_title`, `permission_body`, `kind_word`, `pending_body`, `error_text`, `AgentNotice`, `WorkingRow`, `working_row`.
   - `agent/dump.rs`: `describe`, `wait_text`, `lines`, `part_lines`, `login_lines`, `entry_line`, `tool_lines` and the `*_word` functions (about 350 lines that change with every dump field).

2. **One fold, owned by one thread.** Today the reader folds into `AgentShared`, the UI copies into `ChatModel.entries`, and the model mirrors `turn`, `activity`, `slot`, `phase`, `conversation` and `window` from the transcript under flags (`turn_or_agent`, `slot_changed`, `reset`). That is three sources of truth and the lock D-2 contends. Having the reader decode only (pure `Event::from_value`, no state) and the UI fold into the only `Transcript` that `ChatModel` owns would remove a full copy (D-9), make the lock hold a `Vec` swap, and let the model derive phase, turn and slot from the transcript instead of caching them. The cost is that a `FoldError` is found on the UI thread; it would break the session through `Requests` (a `DETACH`), as `Connection::break_session` does now.

3. **`Requests` returns `Option<RequestId>`, which merges "still starting", "not shown any more" and "link down" into one `None`.** D-1 is the result: the model can only refuse. A `Result<RequestId, NotSent>` with `Starting`, `Gone` and `LinkDown` lets the model hold, refuse or retry, and the `UNSENT` text then says which (`types R1`).

4. **The `Agent` entity's `held: Held` sits beside `requests: Option<Box<dyn Requests>>`**, so `Left` with `Some(requests)` is representable and every method checks both. Taking the requests out on leave (`self.requests.take()`) makes `Held` unnecessary and a send after leaving impossible.

5. **Pure modules reach into GPUI view modules for plain data.** `conversations.rs` imports `crate::workspace::TabId` and `crate::zone_view::{ZoneId, split_target}`, and `close.rs` imports `crate::zone_view::ZoneId`. `split_target` is a pure decision living in an entity module. Moving the ids and `split_target` into the zone tree's pure model (`workspace/` or a `zone_tree.rs`) keeps the agent's pure modules free of the shell's module graph (`ports`, `fcis R5`).
