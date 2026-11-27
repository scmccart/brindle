# Batch A: the server's pure ACP state machine (`remote-agent/src/acp/reduce.rs` and its pure neighbours)

## Coverage

- **Read in full (production code):** `remote-agent/src/acp/reduce.rs` lines 1–3188, `dialect.rs`, `diff.rs`, `plain.rs`, `forms.rs`, `mod.rs`.
- **Read in part, to follow a path:** `rpc.rs` (all of `Rpc`, lines 1–305), `v1/decode.rs` (`session_title`, `tool_call`, `tool_change`, `tool_detail`, `tool_item`, `raw`, `permission_request`, `requested_option`, 340–640), `transcript/src/lib.rs` (the `ToolItem`/`FileChange`/`Output` types and `Output::append`/`cut`, 180–400; `Transcript::apply`, trim, `page`, `fetch`, 1040–1420; `trim_to`, `output_fields_len`, `replay_entry`, `escaped_len`, 1560–1755), `server/core/agents.rs` (module doc, `Agent`, `guarded_step`, `agent_step`, `agent_apply`, `send_transcript`, `agent_checkpoint`, `restart_after_login`, 1–380 and 600–930), `server/core/sessions.rs` (`replay`, `start_again`, 989–1029 and 1929–2000), `server/core/policy.rs` (`agent_checkpoint_due`, `agent_appendable`, `agent_checkpoint_form`), `remote-protocol/src/client.rs` (`pack_transcript`, `pack_events`), `similar-3.2.1/src/algorithms/myers.rs` (its module doc and the work limit).
- **Tests:** `reduce.rs`'s test module only by name and two tests (`tool_content_is_bounded_before_any_event`, `a_delta_past_a_frame_is_sent_in_pieces`) as evidence of gaps; the Claude and Codex captures in `remote-agent/tests/fixtures/acp/` for what agents send.
- **Decisions checked:** the archived designs of `agents-sessions`, `agents-log-paging`, `agents-tool-content` (the `similar` decision, lines 289–318), the `agent-sessions` spec's log bound (line 371), and the open `agents-*` proposals (no overlap with the findings below).
- **Clean:** `dialect.rs` (one exhaustive `match` per method, as `ports R14` asks), `plain.rs` (linear, bounded held line and string skip, split-safe), `mod.rs`. `forms.rs` is clean apart from the unbounded fields it passes through, which A-2 names at the reducer.

## Findings

### A-1: One text entry has no bound, so the 16 MiB log bound doesn't hold and the "unreachable" `TooLarge` is reachable
- Severity: medium
- Confidence: confirmed
- Category: memory
- Where: `remote-agent/src/acp/reduce.rs:1466-1487` (`chunk` appends to the run's entry with no size check); `transcript/src/lib.rs:1575-1597` (`trim_to` always keeps the newest entry); `remote-agent/src/server/core/agents.rs:137-139` and `917-927` (`unstorable`, "the bounds make this unreachable"); the claim is also in `openspec/changes/archive/2026-11-25-agents-sessions/design.md:361` and `2026-11-26-agents-log-paging/design.md:336` ("a huge single entry … bounded by the 16 MiB log").
- Scenario: an agent streams one reply or thought of 20 MiB in `agent_message_chunk`s (a looping thought, or a replayed `user_message_chunk` run in a shown `session/load`). Each chunk is an `Appended` to the same entry; nothing caps the entry. Once the log passes `LOG_LIMIT`, `trim` drops *every older entry* (the giant entry alone is over `LOG_TRIM_TO`), so a single 13 MiB reply erases the whole conversation before it on the server and, through `Trimmed`, in the client. The log then stays above 16 MiB (`trim_to` finds nothing it may drop), against the spec's "SHALL stay within 16 MiB". The entry keeps growing in the core's memory, and at about 64 MiB the next whole checkpoint (`policy::agent_checkpoint_due` fires every 512 KiB of stdout, and a segment run past the base forces a whole one) is `TooLarge`: `agent_checkpoint` steps `Input::Fault`, and the session is `Broken` with "ACP handling failed: …". Each whole checkpoint before that re-encodes the giant entry on the core's single loop thread.
- Fix: a per-entry bound for text entries (e.g. `ENTRY_LIMIT` = 1 MiB, well under `LOG_TRIM_TO`): when an append would take the run's entry past it, end the run and start a new entry of the same kind for the rest (`add_text`), so the text is kept and the log's trim works on whole entries again. Then `TooLarge` really is unreachable and the comments are true. Test gap: no test streams one entry past `LOG_LIMIT` (`the_log_is_trimmed_past_16_mib_keeping_the_newest_entries` uses many entries).

### A-2: The bounds count text bytes, not encoded bytes, so one event can pass the 1 MiB frame, and a replay holding it sends nothing
- Severity: medium
- Confidence: confirmed
- Category: protocol
- Where (each a field whose encoded size isn't bounded where the reducer builds the event):
  - `remote-agent/src/acp/diff.rs:117-129`: `Keep::push` counts `line.text().len()` toward `ITEM_LIMIT`, so an empty line costs 0 and an item holds any number of them; `diff.rs:34-42`: past `MAX_DIFF_LINES` the fallback pushes (and allocates) every line of both texts. `transcript/src/lib.rs:246-252` (`FileChange::text_len`) undercounts the same way, so `CALL_LIMIT` and the `TEXT_CHUNK` split at `reduce.rs:1937-1947` miss it too.
  - `reduce.rs:1672-1680`: `MAX_LOCATIONS` (32) paths, each `bound` to 64 KiB: one `ToolChanged`, and the tool entry's replayed `Added` (`lib.rs:1680-1693` carries the locations), can hold 2 MiB.
  - `reduce.rs:1996-2007` with `v1/decode.rs:601-631`: a permission's options aren't limited in count, and an option's `id` is neither cleaned nor bounded. The `Added` of a permission can hold most of an 8 MiB line, and `Transcript::page` always includes pending permissions.
  - `reduce.rs:1937-1947`: pieces of a `ToolItems` are split by `text_len`, which doesn't count an `Output`'s `command` and `cwd` (64 KiB each). 32 `terminal` items described by `terminal_info` make one 4 MiB event.
  - `reduce.rs:1693`, `1820-1834`, `1882-1903`: terminal ids are kept as map keys at whatever length the agent sent. They aren't sent to clients, but they are checkpointed whole in every segment.
- Scenario: an agent creates a file of 300,000 empty lines (about 600 KB of JSON in one `tool_call`). `hunks("", new)` takes the fallback branch and keeps every `Added("")`: 300,000 lines at 4 encoded bytes each is about 1.2 MB in one `ToolItems`. `pack_events` (`remote-protocol/src/client.rs:793-802`) refuses the whole batch, so `send_transcript` sends an `ERROR` and the client never gets the step's events. Its later events on that entry then fail the client's fold. While that entry is the newest one, every attach's page includes it ("at least the newest entry"), and `sessions.rs:1955-1983`'s replay packs the whole page at once, so the attach replays *nothing*. An `AGENT_FETCH` that reaches it fails the same way (`pack_page`), so the conversation can't be scrolled past it. A pending permission with over 1 MiB of options does the same to every attach while it waits, and its row never reaches the client, which then can't answer it. The 4 MiB `ToolItems` case fails the live send.
- Fix: measure what goes on the wire. Count a diff line as `text.len() + 4` (and an `Output` as text plus command plus cwd) in `Keep::push`, `text_len` and the piece split; stop allocating fallback lines once `Keep` is full. Bound option count (16, as `MAX_LOGIN_METHODS`), option ids (1 KiB, refusing the option as a cut id names another), location paths (4 KiB) and terminal ids (1 KiB, as `MAX_LISTED_ID`). Then extend `tool_content_is_bounded_before_any_event` into a property test that feeds adversarial updates and permissions and asserts every emitted event, and every `replay()`/`page()` event, encodes under `MAX_FRAME_PAYLOAD_SIZE`. Today's test only uses short paths and non-empty lines, so it can't catch these.

### A-3: Starting the agent again drops its pending permission requests and an awaited list without telling the log or the client
- Severity: medium
- Confidence: confirmed for the server side; plausible for the palette's wait (assumed hop: `ChatModel` has no timeout for `Sent::List`)
- Category: correctness
- Where: `reduce.rs:1164-1173` (`respawn` calls `rpc.restart()`, `rpc.rs:252-255`, which clears `permissions` and `outgoing` silently); `reduce.rs:1182-1206` (`restart_for_login` hangs the agent up with no `drop_list`, and `reading()` is false in `Restarting`, so the list's answer is never read); `agents.rs:612-642` (the exit after a login restart steps `Input::Restart`, never `Input::Exited`, so `end()`'s cancelling never runs). A related site: `reduce.rs:1234-1252` (`fail` leaves waiting permissions in the log that `answer` then refuses in `Broken`).
- Scenario: Ready, with a prompt out. The agent asks a permission (entry 7, turn `Blocked`), then answers the prompt `-32000` (its token expired mid-turn). `prompt_needs_login` fails the turn and needs a login, and entry 7 is still waiting in `rpc.permissions`. The user runs the terminal login, and `AGENT_RESTART` goes through `restart_for_login`, the hang-up, the exit and `Input::Restart`, then `respawn`. Entry 7 now keeps `outcome: None` forever:
  - The client draws live Allow/Reject buttons. A press passes every check in `answer()`, then `rpc.answer_permission` returns `None` and the user is told "permission request 7 was already answered", which is false.
  - `Transcript::page` (`lib.rs:1359-1378`) pulls every later attach's page back to entry 7 and its tool entry, so each attach replays everything since then (up to the 16 MiB log rather than the 256 KiB page) until a trim drops it.

  The same path loses a list: if the palette's `AGENT_LIST` is out when the prompt fails, `respawn` forgets it with no `Output::listed`, so the client gets neither `AGENT_CONVERSATIONS` nor `ERROR`.
- Fix: one "this agent process is gone" step, used by `end`, `restart_for_login` and `respawn`. It calls `drop_list(why)` and emits `Answered { Cancelled }` for each waiting permission, so the log never holds a waiting request that `Rpc` doesn't. In `fail`, record the waiting permissions cancelled as well (and answer the agent, which still runs). This restores the invariant that the log's pending permissions are exactly `rpc.permissions()` (`types` R1). Test gap: no test has a waiting permission or list across `a_restart_at_a_prompts_login_demand_takes_up_its_session_keeping_the_log`.

### A-4: Each output delta costs a 64 KiB memmove once the item is full
- Severity: low
- Confidence: confirmed
- Category: performance
- Where: `transcript/src/lib.rs:353-364` (`Output::append`: `push_str`, then `drain(..cut)` moves the kept ~64 KiB, and the dropped prefix is collected into a new `String`), applied by the reducer's `ToolOutput` events (`reduce.rs:1708-1724`) on the core's loop thread and again by the client's fold.
- Scenario: a command whose output arrives in small `terminal_output_delta`s, such as progress lines of about 32 bytes. After the first 64 KiB, every delta memmoves the whole kept text: 1 MiB in 32-byte deltas is about 31,000 × 64 KiB ≈ 2 GiB of copying on each side, where the agent-message path is O(chunk). The perf case `agent-tool-output` uses 4 KiB deltas (240 × 64 KiB ≈ 15 MiB), so it doesn't show this.
- Fix: keep the output's text with a start offset, and compact only when the dead prefix passes `ITEM_LIMIT`. The visible text (and its encoding, equality and `encoded_len`) is the slice after the offset, so the fold's results are unchanged and each append is amortized O(delta).

### A-5: A tool call's diff is recomputed for every message that repeats it
- Severity: low
- Confidence: confirmed
- Category: performance
- Where: `reduce.rs:1861-1881` (`diff::hunks` runs while `kept` is built), before the comparison with the entry's items at `1926-1933`; `reduce.rs:2011-2012` also clones the request's whole `toolCall` (old and new texts) to apply it.
- Scenario: Claude's adapter sends the same `diff` content in the `tool_call_update` and in the `request_permission`'s `toolCall` (`claude-agent-acp-0.112.0-edit-run.log` lines 31 and 32 have identical content). It then sends the whole file's `oldText`/`newText` in the completing update (line 35). Each repeat runs Myers and allocates every kept line on the core's loop thread before finding nothing changed. For whole-file contents near `MAX_DIFF_LINES`, that is tens of milliseconds per repeat, blocking every other session and terminal the core serves.
- Fix: keep a digest of each diff item's `(path, old, new)` beside the entry in the session (not the log), and reuse the item when the digest matches; take the permission's call by value instead of cloning it.

## Answers to the batch's questions

- **Cost per streamed chunk.** An `agent_message_chunk` costs O(chunk) in the reducer: JSON decode of the line, `clean` (one copy), `chunks` (one copy per 64 KiB piece), a binary search for the run's entry, `push_str` and `escaped_len` in the fold, and an O(1) `encoded_len` check in `trim`. The core adds a clone into `unsaved` and a `to_value`. Nothing scans the conversation or the entry, so the server is linear in the reply. A tool-output delta is O(delta) plus A-4's O(`ITEM_LIMIT`) once full.
- **Bounds.**
  - The log: by entries only, with no per-entry cap (A-1).
  - Tool content: `ITEM_LIMIT`/`CALL_LIMIT` measured in text bytes, which don't bound encoded size (A-2). Cleaning after the diff's limit can also triple a line's bytes (a control byte becomes 3-byte U+FFFD), so an item can reach about 192 KiB, which still fits a frame.
  - Raw input and result: `RAW_LIMIT`, but cleaned whole before the cut, and `v1::raw` pretty-prints the whole value first (a constant factor over the decode).
  - Pending permissions: 16, enforced in `hold_permission` and `from_parts`.
  - The slot: `MAX_PROMPT`, checked after `clean`.
  - Login methods and conversation lists: bounded at decode and in `page`.
  - Open outputs: 64; the held filter line is up to 64 KiB each.
  - The `tools` map: bounded only through the log. A restarted session resets the per-process state (`respawn`), and a shown replay goes through the same paths, so the same bounds apply.
  - Unbounded: one text entry, diff line counts, option count and ids, terminal-id key length.
- **Ids.**
  - Request ids are monotonic across `respawn` (`Rpc::restart` keeps `next_id`). They come back from a checkpoint with `from_parts` checking `outgoing < next_id`, and a list's id is reserved by its `BeforeRequest` checkpoint although the list isn't stored. Late answers to forgotten ids (a cancelled login, a prompt lost with the tail) match nothing.
  - Entry ids are never reused, and trims are monotone.
  - Permission wire ids are echoed verbatim; the one gap is A-3.
  - Tool call ids are cleared on `respawn`, so after a resume a `tool_call_update` for a pre-restart call adds a new entry, titled with its id. They are also keyed after `bound(clean(..))`, so two ids that differ only in control characters, or past 64 KiB, merge (adversarial only).
  - List request ids are the client's own. The answer goes to whichever client is attached when the agent answers.
- **Diffs.** `similar` 3.2.1's default Myers uses xdiff's bounded middle-snake search, and its preflight drops unmatched lines. Work is bounded without a deadline, as the `agents-tool-content` design records (lines 289–318), and memory is O(N+M) line slices plus two `V` arrays of `2·max_d`. With `MAX_DIFF_LINES` at 200,000, the worst case is a bounded pause of tens of milliseconds on the core's single loop thread per diff; A-5 is the avoidable part. The real hazard in `diff.rs` is the fallback's unbounded line count (A-2), not its time.
- **Meaningless states.**
  - `chose: bool` means something only while a continue chosen in `Choosing` is in flight.
  - `lists` and `methods` are `initialize`'s answer but live beside `Greeting`, which the phases carry.
  - `Phase::Ready { session }` duplicates `transcript.conversation().id`.
  - The turn and the awaited prompt are two sources of truth (`Working` with no prompt out is representable; decision 11's known gap).
  - Waiting permissions are held both in the log (`outcome: None`) and in `rpc.permissions`, with nothing tying them together (A-3).
  - The guards are scattered: `self.s.run = None` appears in ten places, and every response handler opens with a `let Phase::X = … else return`.

## Structural observations

1. **Cut `reduce.rs` (3,188 production lines, about 4,900 test lines) along its responsibilities.** Each part is an `impl Step<'_>` block in a child module, which costs nothing and keeps the `pub(super)` fields:
   - `reduce/mod.rs`: `Step`, `emit`, `mark`, `finish`, the log's `trim`, and `Input`/`Output`/`Moment`/`Refused`.
   - `lifecycle.rs`: `Phase`, `Start`, `Greeting`, `choose`, and spawned, initialized, start, continued, ready, end, restart, respawn, refuse, fail. About 700 lines.
   - `turn.rs`: decision 10's slot table, that is prompt, unqueue, resend, cancel, `end_turn`, `hold_reason` and `send_prompt`.
   - `tools.rs`: the tool book, from `tool_call` through `tool_content`. About 500 lines, the densest logic.
   - `permission.rs`: permission, answer, withdraw, unblock, `cancel_now`.
   - `login.rs`: `Login`, `AfterLogin`, login, `cancel_login`, authenticated, `keep_methods`, `methods_notice`.
   - `list.rs`: `Page`, `ListAnswer`, the `MAX_LISTED*` constants, list, listed, `page`, `same_dir`.
   - `text.rs`: clean, bound, `cut_at`, `text_item`, `raw_text`, `session_title`, `agent_name`, placeholder.

   Each test group moves with its code. The cut makes each part's invariants reviewable alone, which today's single `Step` hides.
2. **Group the per-agent-process state and replace it in one move.** `respawn`, `end`, `restart_for_login`, `fail` and `trim` each reset or forget a different hand-picked subset of {`rpc.outgoing`, `rpc.permissions`, the list, `run`, `tools`, `outputs`, `bad`, `lines`}, and A-3 is what falls through. A `Process` struct (the awaited requests, `Lines`, `run`, `outputs`, `bad`) replaced whole by `respawn`, plus one `agent_gone(why)` that closes what the log and the client were told was open, would make that class of bug unrepresentable.
3. **Make the tool book its own type with its invariants.** `tools` and `outputs` mirror log entries: every value names a kept tool entry, and outputs point at `Output` items of their entry. `trim` and `respawn` keep them consistent with scattered `retain`/`clear` calls. A `ToolBook` with `forget_through(id)`, `clear()` and its event-producing methods owns that. The `tools` index is also derivable from the log (each `Entry::Tool` carries its `call`), so it could be rebuilt on decode instead of being carried whole in every checkpoint segment (reviewer B's file; it multiplies segment size for long tool-heavy conversations).
4. **Put the wire bounds in one place, with one invariant.** Limits are spread over `bound()` (64 KiB on every string), `MAX_LOCATIONS`, `RAW_LIMIT`, `ITEM_LIMIT`/`CALL_LIMIT` in the transcript crate, and the 1 MiB frame in the protocol crate. A-2 shows that per-field limits don't compose into "every event fits a frame". A small `limits` section (or module) stating that invariant, and a reducer-level property test that checks it for every emitted and replayed event, would keep later fields from breaking it.
5. **Move the flags beside the phase into it.** `chose` would be an origin on `Start::Continue` (and so on `Resuming`/`Loading`/`AfterLogin::Start`); `lists` and `methods` would join `Greeting`. Then the guards in `start` and `continue_refused` read the type rather than a bool that `wait_for_choice` and `continued` must remember to reset.
6. **Stop a step's other effects after a fold fault.** After a fold error `emit` does nothing, but the rest of the step still writes stdin, takes request ids and changes the phase. For example, `send_prompt` still writes `session/prompt` when its user entry failed to apply, and `finish` then marks the session `Broken`. Fold errors are reducer bugs, so this is latent, but `finish` should drop `out.stdin` (or `emit` should short-circuit the step) when `fault` is set, so a broken session never acts on the agent after the step that broke it.
