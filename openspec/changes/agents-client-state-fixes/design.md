# Design: agents-client-state-fixes

## Context

See `proposal.md` for the findings this change fixes. Everything below was read at `5f335dc`; `agents-commands` (PR #151) changes some of these sites before this change is implemented, so task 1.1 re-opens them on `main`.

How an agent zone's requests and events flow today:

- **Spawn.** `Session::start_agent` (`src/remote/local/session.rs`) starts a `server-spawn` thread running `run`. `run` calls `Connection::spawn_agent`, which blocks in `Connection::ask` until the reader thread reads `SPAWNED`. The reader's `ToClient::Spawned` arm (`connection.rs:1789-1804`) shows the session in `state.sessions` with the zone's sink, delivers an endpoint note, sends the reply on the sync channel and goes on reading. The core sends `initialize` right after `SPAWNED` (`remote-agent/src/server/core/sessions.rs` `session_spawned`), so `TRANSCRIPT` frames with `Starting`, `Conversation` and `Ready` can be folded into `AgentShared` (`src/agent/sink.rs` `Sink::transcript`) before the session thread wakes. It then removes the request, builds `Running`, takes the phase lock, hands over queued input, and only then sets `Phase::Running` (`session.rs:475-504`).
- **Attach.** The reader's `ReplayStart` arm (`connection.rs:1856-1862`) delivers `replay(true)` before the reply, and the replay's frames and `REPLAY_END` follow on the reader. The session thread sets `Phase::Running` some time after.
- **Sending.** The `Agent` entity (`src/agent/mod.rs`) sends through `Requests`, which `AgentLink` implements over `Session`. `Session::running` (`session.rs:335-340`) returns `None` in `Phase::Starting`, and so do `Phase::Left` and `Phase::Gone`, and `Running::agent_request` (`connection.rs:3039-3046`) returns `None` once the connection no longer shows the session. The entity turns every `None` into `ChatModel::unsent`, a refusal with `UNSENT` ("the agent session isn't connected; nothing was sent"), and a first prompt becomes the draft (`mod.rs:767-774`, `model.rs:2596-2598`, `2656-2659`).
- **The model's gate.** `ChatModel::can_send` and `prompt_status` (`model.rs:2507-2535`, `2678-2684`) look only at the transcript's agent state, the session's life and the link. `settle_first` (`model.rs:2294-2333`) returns `First::Send` on the first live `Ready`. Nothing in the model knows whether the session thread has finished.

This is D-1: two facts, "the agent is ready" (reader thread) and "the session can send" (session thread), in no order.

The other findings live in the same files: `State::link_lost` (`connection.rs:919-934`), `Running::fetch` (`3089-3105`), `ChatModel::absorb` (`model.rs:1687-1772`), `respawning` (`2224-2234`), `fetching`/`fetched` and `Earlier` (`971-976`, `1602-1638`), `AgentShared::fold` (`shared.rs:205`), `Agent::choose` (`mod.rs:602-611`) and `login::reparses` (`login.rs:148-162`).

### Layer map

| Part | Kind | Crate, file |
|---|---|---|
| `NotSent`, `ReplyError`, `SessionHandle`, `FirstState`, `Uncertain`, `Taken`, `Choose`, `Paging`, `ChatModel::not_sent`, `session_running`, `settle_first`, `reconcile_prompt` | pure model (`fcis R1`), already in `PURE_MODULES` | `brindle`, `src/agent/model.rs` |
| `Changes::status_from`, `StatusFrom` | pure | `brindle`, `src/agent/shared.rs` |
| `reparses` | pure | `brindle`, `src/agent/login.rs` |
| `clean` | pure, shared by both sides | `brindle-transcript`, `transcript/src/lib.rs` (moved from `remote-agent/src/acp/reduce.rs`) |
| `Requests`, its callers, `AgentEvent::SessionRunning` handling | shell (entity): gathers, calls the model, applies | `brindle`, `src/agent/mod.rs`, `src/agent/sink.rs`, `src/remote/local/agent.rs` |
| raising `SessionRunning`, `Session::running` → `Result`, the fault switch | shell (session thread) | `brindle`, `src/remote/local/session.rs` |
| `Waiting::Agent`'s `Carry`, `State::link_lost`, `SessionSink::reply`/`running`, `Running::fetch` | shell (connection); `State::link_lost` is its pure state step, unit-tested as today | `brindle`, `src/remote/local/connection.rs` |
| `TestSwitch::SessionStartDelay`, two e2e cases | e2e harness | `tests/e2e/` |

No decision goes into the view, a thread or a seam. The session thread raises one event and decides nothing. The connection's new rule (which requests a lost link fails) lives in `State::link_lost`, which is already a pure step on `State`. No platform `cfg` is added: the fault switch uses `cfg(debug_assertions)`, as `BRINDLE_TEST_LAUNCH_LISTING` does.

## Goals / Non-Goals

**Goals:**
- Make the first-prompt refusal of D-1 impossible on a spawn and on an attach, show it with a deterministic reproduction, and report failure counts before and after (`testing R9`, `R10`).
- Give every `Requests` outcome a type, so the model can hold, refuse or report each case (`types R1`, `railway R1`).
- Make each of D-4, D-5, D-6, D-7, D-10, D-11 (the dropped choice) and D-12 show with a failing test first, then fix it.

**Non-Goals:**
- A protocol-level prompt id that the core deduplicates. It needs a new negotiated message (`process R4`); the remediation plan's "Not planned" keeps it out.
- The rest of D-11 (`leave_on_quit`, `replay_ended`, the `_ =>` arm in `conversations.rs`), which `agents-module-structure` takes.
- Moving the fold to the UI thread (decision D1 of the plan keeps the reader's fold).
- Any server change beyond moving `clean` to the shared crate.

## Decisions

### 1. `Requests` returns `Result<RequestId, NotSent>` (`types R1`, `railway R1`)

```rust
// src/agent/model.rs
pub enum NotSent {
    /// The session isn't handed to the zone yet (`Phase::Starting`):
    /// nothing was sent, and the zone can send once it runs.
    Starting,
    /// The zone let it go, it never started, or the connection no longer
    /// shows it (exited, lost, broken): nothing was sent, and nothing will be.
    Gone,
    /// The link is down, for a request that never waits for the next link
    /// (a fetch, decision 6).
    LinkDown,
}
```

- **Methods:** every `Requests` method that returns `Option<RequestId>` returns `Result<RequestId, NotSent>`, and `cancel` returns `Result<(), NotSent>`. `close`, `detach_with`, `set_layout` and `id_and_key` don't change.
- **Where each case comes from:**
  - `Session::running` maps `Phase::Starting(_)` to `Starting` and `Phase::Left(_)` and `Phase::Gone` to `Gone`.
  - `Running::agent_request` returns `Gone` when `state.shown(..)` is `None`.
  - `Running::fetch` returns `LinkDown` when the link is down (decision 6).
- **The other layers follow:**
  - `AgentLink` (`src/remote/mod.rs`) and the `impl Requests for AgentLink` change signature only.
  - `FakeRequests` (`src/agent/test_support.rs`) replaces `down: Rc<Cell<bool>>` with `refuse: Rc<Cell<Option<NotSent>>>`.
- **Where the type lives:** `NotSent` is defined in `model.rs` beside `StartFailure`, which `session.rs` already imports from there. The model is the code that decides on it.
- **What the model does with it:** one pure function, `ChatModel::not_sent(&mut self, sent: Sent, why: NotSent) -> Replied`, replaces `unsent`. It decides each caller's case:

| Request (`Sent`) | `Starting` | `Gone` | `LinkDown` |
|---|---|---|---|
| `FirstPrompt` (from `First::Send` or `send_first`) | back to `FirstState::Waiting` (or `Shown`), no refusal; sent on `SessionRunning` | refusal `UNSENT`, `SetDraft(text)` (as today) | not returned |
| `Prompt` (enter) | `Nothing`; the input keeps its text, no refusal | refusal `UNSENT`, input kept (as today) | not returned |
| `Resend`, `Unqueue`, `Answer`, `Restart`, `Choose` | nothing recorded, no refusal | refusal `UNSENT` (a `Restart` shows the ended agent again, as its refused reply does) | not returned |
| `Login` | nothing recorded | `login_note = UNSENT` | not returned |
| `List(Palette)` | `Listed(Palette(Err(STARTING)))` (the palette says the agent is still starting) | `Listed(Palette(Err(UNSENT)))` | not returned |
| `List(Latest)` | `choosing.launch` set back to `Last`, so the launch asks again once the session runs | `choosing.note = UNSENT` (as today) | not returned |
| `Fetch` | paging state unchanged (`Available`); asked again while the row shows | `Failed { why: UNSENT }` | `Failed { why: LINK_DOWN }` |
| cancel, Cancel login (no `Sent`) | nothing | `server_note(UNSENT)` (as today) | not returned |

`Starting` is unreachable once decision 2 holds, since `can_send` requires the running session. It is handled anyway, as "hold, never refuse", so a decision made from state that changed between the model's check and the send costs the user nothing. `fetch_unsent` goes; `send_fetch` calls `not_sent(Sent::Fetch, why)`. A test checks each row of the table on the pure model (task 2.2).

`STARTING` is new text: "the agent session is still starting". `UNSENT`'s text doesn't change. `LINK_DOWN` already exists in `connection.rs`; it moves to `model.rs` as a `pub const`, and the connection uses it from there.

*Rejected:*
- **Hold agent requests in `Phase::Starting` inside `Session`, as input is held** (`Queued`). The hold would be a decision on the session thread (`fcis R1`), invisible to the model's `prompt_status`, and a held permission answer or fetch would go out against a log the zone hasn't seen.
- **A boolean "maybe later".** `Gone` and `LinkDown` call for different text, and a boolean would merge the three cases again.

### 2. The session tells the zone when it runs (`process R13`, `R14`)

- **Raising it:** `SessionSink` gains `fn running(&mut self) {}` (default no-op; a terminal's sink ignores it). In `run` (`session.rs`), after `*phase = Phase::Running(running)`, the thread drops the phase lock and calls `sink.lock().running()`. This happens on every path that installs `Phase::Running`: spawn, agent spawn and attach. A path that ends in `Left` or `Gone` raises nothing. The agent sink's `running` sends `AgentEvent::SessionRunning`, a life event (`kind_of` → `Kind::Life`), so the gate never merges or drops it.
- **Why the order holds:** the reader delivers through `Connection::deliver`, which locks the same `Sink` mutex (`Arc<Mutex<Box<dyn SessionSink>>>`), and the agent sink queues on its channel under its gate lock in call order. So every event the reader delivered before the session thread takes the sink's lock is queued before `SessionRunning`. More importantly, `Phase::Running` is set before `SessionRunning` is queued, so when the zone takes `SessionRunning`, `Session::running` finds `Running`. That is the fact D-1 lacked. Events the reader delivers later can come before or after `SessionRunning`. Neither order matters, because the model decides only when it holds both facts.
- **No lock-order problem:** the phase lock is released before the sink's is taken. No code takes the phase lock under the sink's, and `failed` is already called the same way on this thread.
- **The model** gains `handle: SessionHandle`:

```rust
pub enum SessionHandle {
    /// The session thread hasn't installed the running session: nothing can be sent.
    Starting,
    /// `SessionRunning` came: requests go through.
    Running,
}
```

  - **Kept beside, not inside, `SessionLife` and `Link`.** `SessionLife::{Open, Ended}` comes from `EXIT` and `Link` from the reader. `EXIT` can arrive before `SessionRunning` (an agent that exits at once), and so can `LinkDown`. One enum combining them would need the product of their states. `SessionHandle` changes once, `Starting` to `Running`, per session.
  - **Set:** `ChatModel::new` starts at `Starting`. `ChatModel::session_running(&mut self) -> Absorbed` moves it to `Running` through `life_event`, which runs `finish` and so settles a waiting first prompt. `respawning` sets it back to `Starting` (a new session). `restarting` keeps `Running`, since the same session continues.
  - **Read:** `can_send` requires `Running`. `prompt_status` returns `Disabled(DisabledWhy::Connecting)` for `Live(Ready)` while `Starting`. `fetch` (paging) and `can_choose` require `Running`. `finish` raises `OnChoosing` only when `Running`. Without that last check, a `-c` launch's `list` hits the same race as the first prompt: it fires when the agent's `Choosing` folds.
  - **Not read by `keeps_session`.** A zone quit while its session is still being installed is detached as today.
- **The entity** handles `AgentEvent::SessionRunning` with `self.model.session_running()` and `apply_absorbed`.
- **Tests:** in `src/agent/test_support.rs`, `Zone::new` calls `sink.running()` after building the entity, so the existing entity tests keep their meaning. `Zone::starting` doesn't, and `fake_connect` calls `running()` on each new session's sink before keeping it (the fake session runs at once). The model tests' `Zone::new` (`model.rs` tests) calls `model.session_running()`, and a `Zone::starting` leaves it out.

*Rejected:*
- **Install `Phase::Running` from the reader thread on `SPAWNED`.** The reader would need each `Session`'s phase and budget, and would hand queued input to the writer from the reader's handler. That moves the session's start into the connection's frame loop, and a terminal's start would change with it.
- **Make the reader wait for the session thread.** It blocks the reader of every session on the endpoint on one thread's scheduling (`process R13`).
- **Only return `NotSent::Starting` and retry the first prompt on the next event.** The next event may never come: an idle agent sends nothing after `Ready`.

### 3. The first prompt's state is one enum (D-7's second pair, `types R1`)

```rust
enum FirstState {
    None,
    /// The launch's prompt, waiting for the agent and the session.
    Waiting(FirstPrompt),
    /// Shown in the read-only input while the agent needs a login.
    Shown(FirstPrompt),
}
```

`first: Option<FirstPrompt>` and `first_shown: bool` become `first: FirstState`, which rules out `first_shown` without a prompt. `first_shown()` reads `Shown`.

`settle_first(status_from)` decides, in order (decision 8 defines `StatusFrom`):
1. The session ended: `Draft`.
2. `Live(Ready)` on an open session:
   - `status_from == Replay`: `Draft`, never sent from a replay, as today.
   - The link is reconnecting: wait (`None`).
   - `handle == Starting`: wait (`None`).
   - `Shown`: `SendIfUnchanged`.
   - Otherwise: `Send`.
3. The other phases: as today.

Ready absorbed live while `Starting` therefore waits, and `session_running`'s `finish` (with `StatusFrom::Unchanged`) sends it. A reconnect replay before `SessionRunning` that shows `Ready` makes it a draft, which is the safe outcome: the prompt is never sent twice.

### 4. Reproducing D-1: a fault switch and two e2e cases (`process R20`, `testing R9`, `R10`, `R18`)

- **The switch:** `BRINDLE_TEST_SESSION_START_DELAY=<milliseconds>`, honoured only in debug builds. Its name is `pub const SESSION_START_DELAY` in `src/remote/mod.rs`, which every platform compiles, so `main.rs` names it without a platform `cfg`. `run` reads it once, before connecting, through `#[cfg(debug_assertions)] fn start_delay() -> Option<Duration>`; a value that isn't a whole number of milliseconds is ignored. After the core's answer and before the phase lock, `run` sleeps that long, commented as `launch_listing`'s is: the fault injector's delay, not a wait for a condition. A release build has no such code. `fault_switches()` in `main.rs` lists it, and `fault_switches_are_listed_only_in_debug_builds` expects three names.
- **The harness:** `tests/e2e/switches.rs` gains `TestSwitch::SessionStartDelay`, named `BRINDLE_TEST_SESSION_START_DELAY`, hosted by `SwitchHost::Brindle`. It also joins the switches module's self-test table.
- **The cases,** in `tests/e2e/cases/agent.rs`, registered in `cases/mod.rs` with `needing(…, &[TestSwitch::SessionStartDelay], …).needs_fake_agent()`:
  - `agent-first-prompt-before-the-session-runs`:
    - **Setup:** `Script::ready` profile, `.first_prompt("one")`, the switch at `ctx.scale().secs(1.5)` in milliseconds, so a slow run keeps the window open.
    - **Checks:** `--wait-for` the user entry `one`, then a dump. The dump has no `refusal=`, the entries are `user "one"` and the agent's reply, and the agent received `["one"]` (`prompts_sent`).
  - `agent-prompt-before-the-attached-session-runs`:
    - **First run** (`keep_detached`): the agent's ready turn, then quit, leaving the session detached.
    - **Second run,** restoring with the switch:
      1. `--wait-for` `state=ready`, then `--send hi` and `--key enter` while the session thread sleeps.
      2. `--wait-for` `prompt status=ready`, then `--key enter`.
      3. `--wait-for` the user entry `hi`.
    - **Checks:** no `refusal=` in any dump, and the agent received `hi` once.
- **Before the fix,** with the switch and the cases added first (task group 1), both cases fail on every run: the zone shows `refusal="the agent session isn't connected; nothing was sent"`. **After,** both pass.
- **Numbers** for the PR body (`testing R10`):
  - Each new case, 20 runs before and after.
  - The original flake: `agent-queue` and `agent-continue-profile-gone` looped as whole e2e binaries, 4 loops in parallel under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2`, each inside `systemd-run --user --scope`, 100 iterations a loop (400 runs a case), on `main` and on the branch.
  - If `main` doesn't fail locally, the switch cases carry the evidence. The PR says so (`testing R11`) rather than claiming a measured cure of the CI flake.

### 5. A prompt whose answer was lost is settled by the replay (D-4)

- **The reply says which failure it was.** `SessionSink::reply` and `AgentEvent::Reply` carry `Result<(), ReplyError>`:

```rust
pub enum ReplyError {
    /// The core refused it, or it never went out: nothing happened.
    Failed(String),
    /// It was written on a link that went down before its answer: the core may have taken it.
    Unanswered(String),
}
```

  - `Unanswered` comes only from `State::link_lost`'s written requests. In the connection, `link_lost`'s caller (the link-down handler near `connection.rs:2179`) calls a new `Waiting::unanswered(message)` for them. It sends `Unanswered` to an agent's sink and fails other waits as `fail` does.
  - Everything else (`ToClient::Error`, `Waiting::fail`: a withdraw, an encode error, a mismatched answer, a failed connect, an ended connection) is `Failed`.
  - `ReplyError` lives in `model.rs` beside `Sent`.
- **What the ledger records.** `Sent::Prompt` and `Sent::FirstPrompt` carry `PromptSent { text: String, after: Option<EntryId> }`. `after` is the newest entry the zone held when the prompt was sent, recorded by `ChatModel::sent_prompt(text, first) -> Sent`, which the entity calls in place of building the variant. `in_flight` counts an uncertain prompt, so the input shows `Submitted` and enter can't send it twice.
- **The reply.** `answered(Prompt | FirstPrompt, Err(Unanswered(why)))` sets `uncertain: Option<Uncertain>`, where `Uncertain { sent: PromptSent, first: bool, why: String }`. It sets no refusal and returns `Replied::Nothing`. The input keeps the prompt, and the link-down event that follows says the zone is reconnecting.
- **The decision** is a pure function, `fn taken(prompt: &PromptSent, entries: &[(EntryId, Entry)], window: Option<Window>, slot: &Slot) -> Taken`, with `enum Taken { Yes, No, CantTell }`. With `kept = transcript::clean(&prompt.text)`:
  - **`Yes`** when the slot is `Waiting` or `Held` with text `kept` (queued behind a turn), or a held `Entry::User` after `after` has text `kept`.
  - **`CantTell`** when the window starts past the entry after `after`. The prompt's entry, if any, may be in the part the replay's page left out: a turn of more than a page ran after it.
  - **`No`** otherwise.
- **When it is decided:**
  - On the next absorb with `changes.reset` (the reconnect's replay), after the entries, window and slot are read. `Absorbed` gains `replied: Option<Replied>`, which `apply_absorbed` hands to `apply_replied`:
    - `Yes`, typed prompt: `ClearDraft`.
    - `Yes`, first prompt: nothing (it never was in the input).
    - `No`: refusal `PROMPT_NOT_TAKEN` ("the connection to the terminal server dropped before the prompt arrived; nothing was sent"). A first prompt also gets `SetDraft(text)`.
    - `CantTell`: refusal `PROMPT_MAYBE_TAKEN` ("the connection to the terminal server dropped as the prompt was sent; it may have reached the agent"), plus `SetDraft` for a first prompt.
  - `lost`, `broken` and `not_started` decide `CantTell`, since nothing will replay.
  - `respawning` drops it: a new session has no record of the old one's prompt.
- **One definition of "cleaned".** The core stores a prompt as `reduce::clean(text)` (`remote-agent/src/acp/reduce.rs:3015`). `clean` moves verbatim to `brindle-transcript` (`transcript/src/lib.rs`, `pub fn clean`) with a unit test of its rule: control characters to U+FFFD, keeping `\n`, `\t` and a `\r` before `\n` or at the end. `reduce.rs` imports it, so the helper's behaviour doesn't change and its existing tests pin that. This is the crate both sides already share; no dependency is added.
- **Only prompts.** A lost `Resend`, `Unqueue`, `Answer`, `Restart`, `Login` or `Choose` is still shown as failed with its message. The replay shows what the core did, and a second try is refused by the core (`NoneHeld`, an answered permission), never duplicated.
- **The prompt changes that follow.** `agents-prompt-context` and `agents-prompt-images` add attachments to `AGENT_PROMPT`, the user entry and the slot. `taken` must then compare the attachment list as well as the text. Task 6.4 adds that requirement to each of those changes' designs.

*Rejected:*
- **A client-chosen prompt id the core deduplicates:** a new negotiated message, out of scope (see Non-Goals).
- **Matching the newest user entry only:** a prompt queued behind a turn that has since run may not be the newest. "Any user entry after `after`" holds, because the zone sends nothing else while the prompt is uncertain.
- **Telling the failure apart by its text:** that is a stringly-typed decision (`types R6`).

### 6. No fetch crosses a link (D-5)

- **The tag.** `Waiting::Agent` gains `carry: Carry`:

```rust
/// Whether an agent request still queued when its link goes down goes out on the next link.
enum Carry {
    NextLink,
    /// A fetch: its `before` names the fold the next link's replay starts over.
    Never,
}
```

- **Which requests get which.** `Running::agent_request` makes `NextLink`, as every agent request but a fetch is carried today. `Running::fetch` makes `Never`, and with the link down it returns `Err(NotSent::LinkDown)` at once, in place of the synthetic `reply_to(…, LINK_DOWN)`.
- **What a lost link does.** `State::link_lost` returns `LinkLost { unanswered: Vec<Pending>, unsent: Vec<Pending> }`:
  - `unanswered`: written requests, attaches excepted, as today.
  - `unsent`: queued `Waiting::Agent { carry: Carry::Never, .. }`, removed from `requests`.
  - The handler fails `unsent` with `ReplyError::Failed(LINK_DOWN)`.
- **The frame goes nowhere.** When the writer later takes the fetch's frame, `Connection::mark_written(req, true)` finds no request and returns `false`, so the frame is dropped (`connection.rs:2486-2490`, unchanged). This holds whether the frame is still in the writer's channel or in its retry loop after a failed write.
- **The model** sees `Failed` with the link's text for that fetch. The reset that follows starts the paging over (decision 7), so nothing waits.

### 7. The ledger is the only record of a fetch in flight (D-7)

- **Stored state:** `ChatModel.earlier` becomes `paging: Paging`, with `enum Paging { None, Available { count: u64 }, Failed { count: u64, why: SharedString } }`. It has no `Loading` variant and no request id.
- **What the row reads:** `ChatModel::earlier() -> Earlier` is derived on each call. It returns `Earlier::Loading { count }` while `pending` holds a `Sent::Fetch` and the paging state has a count; otherwise it mirrors `Paging`. The public `Earlier` loses `Loading`'s `request` field.
- **The other paging functions:**
  - `fetching(request)` only inserts `Sent::Fetch`.
  - `fetched` acts on the `Sent::Fetch` that `replied` removed, as today.
  - `fetch_due` and `retry` treat "a fetch is pending" as `Loading`.
  - `follow_window` keeps `Failed`'s reason, as today.
- **The view and the dump read `earlier()`:** `src/agent/view.rs:391`, `401` and `842-848`, `model.rs`'s `earlier=` line. Only the `Loading { .. }` pattern changes, and `view/tests.rs:993`, which reads the request id from it, takes the id from the fake requests' calls instead.
- **What this fixes.** A respawn clears `pending`, and with it the loading state, so the row can't spin forever. The motion clock stops with it, since `refresh_motion` reads the same derived state.

### 8. Replays and live events apart within one batch of changes (D-10)

- **The new field.** `Changes::from_replay: bool` becomes `status_from: StatusFrom`:

```rust
/// Where the newest `Turn` or `Agent` event in these changes was folded.
pub enum StatusFrom {
    /// None came.
    Unchanged,
    Replay,
    Live,
}
```

- **How it is set:**
  - `AgentShared::fold` sets it on every `Turn` and `Agent` event to `Replay` while `replaying`, else `Live`. The newest one wins.
  - `replay_start` resets `Changes` with `Unchanged`, and the replay's own `Agent` event sets `Replay`.
  - `prepend` no longer touches it: a page carries entries only, and its "already happened" is `prepended.is_some()`.
- **How the model reads it:**
  - `absorb` passes `changes.status_from` to `finish`.
  - `notice` is raised when the shown status changed and `status_from != Replay`.
  - `settle_first` follows decision 3.
  - `OnChoosing` is raised when `status_from != Replay` (and the session runs, decision 2).
  - Life events pass `Unchanged`.
- **Why this rather than the report's `live_from: Option<EntryId>`.** The zone's status, the first prompt and the choosing launch derive only from the agent and turn states (`Transcript::activity`). So the origin of the newest such event is the exact fact needed, while an entry id says where entries start, which none of these read.
- **The three cases:**
  - A replay that shows `Blocked`, followed by live text only: `Replay`, so no notice, which is right.
  - A replay followed by a live `Turn { Blocked }`: `Live`, so the notice is raised.
  - A live `Ready` right after a replay: `Live`, so the first prompt is sent once the link is up and the session runs.

### 9. A new log clears what belonged to the old one (D-6)

`respawning` also clears `toggled`, `by_hand` and `auto_open`, and sets `handle = Starting` and `uncertain = None`. The trim branch of `absorb` prunes `toggled` with `by_hand` (`*id > through`).

A reset by a replay of the same session keeps both, since its entry ids are the same entries. The rows of the old conversation, still shown until the new session's first change replaces them (`AgentShared::start_over`), are redrawn by `refresh_rows` with the policy's state. That is the state the new conversation's rows will have.

### 10. A choice that can't go says why (D-11, part; `fcis R1`)

- **The model decides.** `ChatModel::choose(&mut self) -> Choose` replaces the entity's `if !self.model.can_choose() { return }`. Its result is `enum Choose { Send, Refused }`:
  - **`Refused`** sets `choosing.note`, which the choosing screen shows as its reason (`choosing_reason`), to one of:
    - `CHOICE_LINK_DOWN` (`"the connection to the terminal server is down; the choice wasn't sent"`) while reconnecting.
    - `CHOICE_PENDING` (`"a choice is already on its way"`) while a `Sent::Choose` is pending.
    - `STARTING` while the session is still starting.
    - `"the agent isn't choosing a conversation"` when the agent isn't choosing.
  - **`Send`** clears the note.
- **The entity applies it.** `Agent::choose` calls `requests.choose` on `Send`, records with `chose` or `not_sent`, and notifies on `Refused`.
- **Callers are unchanged.** `Workspace::open_here` (`src/workspace.rs:1729-1745`) calls `choose` exactly as now: the palette closes, and the zone shows why.
- **Tests:** the table is tested on the model, and an entity test checks that a choice during `LinkDown` shows the reason.

### 11. More commands withhold terminal logins (D-12; choice C5; `agents-login` decision 9 amended)

`agents-login` decision 9 listed `cmd /c|/k`, a batch file and `ssh`. This change adds the commands that hand appended arguments to a parser, deciding from the program's stem (as `program_name` gives it, on every platform) and its arguments, compared case-insensitively. An argument "names a switch" when, with one leading `-`, two `--` or one `/` removed, it is one of the listed spellings:

| Program | Withheld when | Words |
|---|---|---|
| `pwsh` | an argument names `-Command` (any prefix of `command`, from `c` on: pwsh takes the first switch a prefix matches, and `-co` is `-Command`; task 1.9 checks the rows against pwsh's `CommandLineParameterParser`. A longer word such as `commandwithargs` is not a prefix of `command`) or `-EncodedCommand` (`e`, `ec`, or a prefix of `encodedcommand` of 2+ letters) | "the login can't be run through `pwsh -Command`" |
| `powershell` | no argument names `-File` (`f`, or a prefix of `file` of 2+ letters): Windows PowerShell reads its arguments as a command by default | "the login can't be run through `powershell`, which reads its arguments as a command" |
| `wsl` | no argument is `-e` or `--exec`: without it, WSL hands the command line to the distribution's shell | "the login can't be run through `wsl` without `--exec`" |

- **Why `pwsh` and `powershell` differ.** `pwsh`'s default for a bare argument is `-File`, so it is withheld only for the command forms; `powershell`'s default is `-Command`. `-CommandWithArgs` (`-cwa`) passes arguments as `$args` and is not withheld.
- **Where it lives:** `reparses` stays one pure function. Its tests are table rows (`commands_that_reparse_their_arguments_are_withheld`, extended), plus a new `commands_that_pass_arguments_through_are_offered` for `pwsh -File`, `pwsh -cwa`, `powershell -File` and `wsl --exec`. They run on every platform (`testing R17`).
- **The record:** `agents-login`'s archived design isn't edited. This design is the record that its list changed, and the main spec's requirement changes with this change's delta.

### 12. No wire, config or state change; what a crash or upgrade does

- Every type added is in-process. `SessionSink`, `AgentEvent`, `Requests` and `ReplyError` are Brindle's own and change together.
- `clean` moves within the workspace, with the helper's behaviour unchanged. Its transcript tests (`remote-agent`'s reducer tests) pin that.
- The server is untouched, so a core restart, an upgrade and a Brindle crash do to agent sessions what they do today.
- The new client state (`SessionHandle`, `Uncertain`) is in memory and goes with a crash. A relaunch attaches, gets `SessionRunning` again from its new `Session`, and has no first prompt and no uncertain prompt.

### Findings, traced

- **D-1,** traced at every hop above. The comment on `Session::running` ("the zone offers none until its agent is ready") states the assumption that fails, and the attach path has the same window (`connection.rs:1856-1862`).
- **D-5** (*plausible*) depends on the carried fetch being written on the new link. That is what `write`'s retry loop does for a `Sent::Queued` request (`connection.rs:2456-2539`), and `link_lost` leaves queued requests alone (`919-934`). Task 1.4 shows the client side with a failing `State` test. The consequence, a gap in `Transcript::prepend` breaking the session, is already pinned by `a_page_with_a_gap_breaks_the_session_and_changes_nothing` (`src/agent/sink.rs`).
- **D-7** (*plausible*) depends on the view keeping the motion clock running while the model says `Loading`. `refresh_motion` reads `earlier()`, so the derived state of decision 7 removes the cause whatever the view does. Task 1.6 shows the model's half.
- **D-12** (*plausible*) depends on a profile of that shape being in use. Task 1.9's failing table rows show the rule's gap; whether anyone runs such a profile doesn't change the fix.
- **Not shown.** If a "show it first" test of D-4, D-5, D-6, D-7, D-10, D-11 or D-12 passes at `main` (the fault isn't there), that finding's fix is dropped from this change, and the as-built notes say "Finding X-n, traced, not shown" with the test kept as a regression test.
- **The `-c` launch.** One more instance of D-1's race found while tracing it: a `-c` launch's `ListFor::Latest` list is asked when `Choosing` folds (`finish` → `OnChoosing::ListLatest`), which can be before `Phase::Running`, refused with `UNSENT` and shown as the choosing screen's reason. Decision 2 gates it on the session too. `a_launch_lists_for_the_latest_or_opens_the_list` gains the starting row (task 2.3).

## Risks / Trade-offs

- **The prompt input stays disabled a little longer.** The model now waits for `SessionRunning`, which is normally microseconds after `Ready` and is a whole scheduling delay under load. → The input says "connecting", which is true, and nothing is refused.
- **`ClearDraft` after a reconcile empties the input even if the user typed during the reconnect.** This is the same as today's `OK` reply, which empties the input whenever it arrives. → It is accepted. If it shows up in use, a follow-up can make it `ClearDraftIf(text)`, as `SendFirstIfUnchanged` does.
- **`CantTell` keeps a prompt the core may have taken.** The user may send it again. → The words say it may have been sent; this only happens when a turn of more than a page ran during the reconnect.
- **The session thread calls into the sink outside `Connection::deliver`'s `catch_unwind`.** It does so as `failed` already does. → An agent sink's `running` only queues an event; a terminal's is a no-op.
- **The `Ready`-then-reconnect window before `SessionRunning` turns a first prompt into a draft** where today it might have been sent. → That window is a link drop within the start's scheduling delay, and a draft is never a duplicate.
- **`agents-commands` merges first and touches the same sites.** → Task 1.1 re-opens them, and its option and command state is independent of the ledger.

## Migration Plan

None: no wire, config or state change, and no server change beyond moving `clean`. A Brindle with this change talks to the same helpers. Rolling back is a revert.

## Open Questions

1. **Should `sh -c`, `bash -c` (including Windows' `bash.exe`, which is WSL) and similar `-c` shells withhold terminal logins too?** Appended arguments become `$0`, `$1`, … of the script, so `--login` is dropped unless the script forwards `"$@"`. The login would run the agent as a plain start. Recommendation: withhold them in a follow-up row of the same table ("the login can't be run through `sh -c`"), once a real profile of that shape is seen, since C5 named only `pwsh`, `powershell` and `wsl`.
2. **Should `SessionRunning` go through `Connection::deliver`'s `catch_unwind`, like the reader's calls?** Recommendation: no. It would need the connection in `run` after the start, the sink's `running` only queues, and `failed` sets the precedent.
