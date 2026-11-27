# Proposal

## Why

The 2026-11-26 agent review (`docs/architecture/audits/2026-11-26-agents.md`, batch reports in `docs/architecture/audits/2026-11-26-agents/`; remediation plan `docs/architecture/audits/2026-11-26-agents-remediation.md`, change 5) found the agent code correct in its layering but too large to read in one place, with state kept apart from the facts it belongs to:
- **`a.md` observations 1, 2, 3, 5:** `remote-agent/src/acp/reduce.rs` is 8,125 lines (3,187 of production code), one `impl Step` covering the whole lifecycle, turns, tool calls, permissions, logins and listing. A respawn resets six per-process fields one by one (`lines`, `run`, `bad`, `tools`, `outputs`, the rpc's awaited requests), so a field added later is easy to leave out. The tool and output indexes are two maps trimmed and cleared together. `chose`, `lists` and `methods` are session-wide flags whose meaning depends on the phase.
- **`b.md` observations 1, 2, 5, 6:** `acp/checkpoint.rs` mixes the record framing, the state's JSON and adoption; `adopt`'s `merge` restates `Step::mark`'s checkpoint precedence; `transcript/src/lib.rs` is 3,724 lines holding the event forms, the fold, paging and output text.
- **B-6 (`ports R14`):** the reducer names ACP's JSON-RPC error codes (`AUTH_REQUIRED`, `UNSUPPORTED_VERSION`, from `acp/forms.rs`) at five sites, so a version's error codes are read outside `acp/v1/`.
- **`c.md` observations 2, 3, 4, 5:** the agent restart lifecycle lives in `server/core/sessions.rs` beside the terminals'; four handlers each gate on client protocol 4 by hand; `Core::checkpoint` serves terminals and agents through one signature with a sentinel argument.
- **`d.md` observations 1, 3, 4, 5, and D-11's remainder (`railway R3`, `fcis R1`):** `src/agent/model.rs` is 5,914 lines (3,293 of production code); the `Agent` entity keeps `held` beside `requests: Option<…>`, two fields for one fact; it decides on its own what a quit and the end of a replay do; `agent/conversations.rs` has a `_ =>` arm on its own enums; the agent's pure modules import `ZoneId`, `split_target` and `TabId` from view modules.
- **`e.md`'s observation on `workspace.rs` and `zone_view.rs`:** the conversations palette (about 300 lines) sits in `workspace.rs` and the terminal-login zone code in `zone_view.rs`, each apart from the rest of its kind.

Changes 1–4 of the plan and `agents-commands` (PR #151) add to these same files. Splitting them now, after those land and before `agents-checkpoint-deltas` and `agents-absorb-cost` change the checkpoint, transcript and model again, lets the last two changes work in files of one concern each.

## What Changes

No behaviour changes. Nothing on the wire, in a checkpoint, in a transcript, in the config, in the dump or on screen changes.

- **Moves, one commit per file, each showing only moved lines plus `mod` and `use` lines under `git diff --color-moved=dimmed-zebra`:**
  - `acp/reduce.rs` becomes `acp/reduce/{mod,lifecycle,turn,tools,permission,login,list,text}.rs`, its test harness `reduce/harness.rs`;
  - `acp/checkpoint.rs` becomes `acp/checkpoint/{mod,format,state,adopt}.rs`;
  - `transcript/src/lib.rs` keeps the `Transcript` type and the crate's re-exports, and gains `event.rs`, `fold.rs`, `page.rs` and `output.rs`;
  - `src/agent/model.rs` becomes `src/agent/model/{mod,phase,blocks,tool_rows,ledger,queue,prompt,login,recovery,choosing,paging,attention,dump}.rs`, its test harness `model/harness.rs`;
  - the agent restart lifecycle moves from `server/core/sessions.rs` to `core/agents.rs`;
  - the conversations palette moves from `workspace.rs` to `workspace/conversations.rs`, and the terminal-login zone code from `zone_view.rs` to `zone_view/login.rs`;
  - `ZoneId`, `ZoneTree`, `SplitAxis`, `Direction`, `UnitRect`, `split_host`, `split_cwd` and `split_target` move from `zone_view.rs` to a new whole pure module, `src/zone_tree.rs`, re-exported from `zone_view`.
  - Every public path stays through `pub use`.
- **Type changes, each its own commit with its tests first:**
  - a `ToolBook` owning the tool-call and open-output indexes;
  - a `Process` value holding the per-process state, which a respawn replaces whole (built on change 4's closing step);
  - `Output::then`, with one `Moment` precedence shared by `Step::mark` and adoption;
  - `Dialect::failure(&RpcError) -> forms::Failure` (B-6), the error codes moving into `acp/v1/`;
  - the `Agent` entity's `held` and `requests` become one `Hold` enum;
  - `ChatModel::leave_on_quit` and `ChatModel::replay_ended` (D-11), and `conversations.rs`'s `_ =>` arm spelled out (D-11, `railway R3`);
  - one exhaustive protocol-4 gate, `policy::agent_request(&ToCore)`, applied once in `handle_client`;
  - `Core::checkpoint` split into `checkpoint_screen` and `checkpoint_agent`.
- **Checks:** `tests/architecture.rs`'s `PURE_MODULES` names the new files, and a new test finds no ACP error code outside `acp/v1/`. `ports R14`'s enforcement text and `fcis`'s whole-pure-modules table change in the same commit.
- **Left out, with the reason in the design:**
  - the serde mirror of the checkpoint state (`b.md` observation 1), which risks the checkpoint's bytes;
  - moving `chose`, `lists` and `methods` into the phase and the greeting (`a.md` observation 5), which changes what a checkpoint records in some phases;
  - moving `TabId` (it already lives in the pure `workspace/tabs.rs`).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. No requirement changes, so `.openspec.yaml` sets `skip_specs: true`. These requirements hold as written, and their tests and e2e cases pass unchanged:
- `agent-sessions`: "An agent that doesn't speak ACP version 1 is closed with the reason", "Transcript event forms are fixed for each protocol version", "Attach replays the transcript", "Clients before protocol 4 never see agent sessions", "Agent sessions survive core restarts and upgrades", "A lost holder leaves agent sessions that can continue", "Continuing a conversation", "The agent's login methods are offered with a login demand", "How agent sessions are listed".
- `agent-zones`: "Tool call rows", "An agent that needs a login", "Quitting keeps agents running", "Replays raise nothing", "Continuing a conversation", "A terminal login runs beside the agent zone", "A terminal login zone is not kept".
- `terminal-server`: "Protocol versions", "Clients reconnect after a core restart".
- `zones`: "Splitting the active zone", "Which profile a split runs", "A terminal login's zone can't be split".

## Impact

- **`brindle-remote-agent`:** `src/acp/reduce.rs` → `src/acp/reduce/`; `src/acp/checkpoint.rs` → `src/acp/checkpoint/`; `src/acp/forms.rs` (`Failure`; the two error codes leave); `src/acp/dialect.rs` and `src/acp/v1/decode.rs` (`failure`, the codes); `src/acp/rpc.rs` unchanged; `src/server/core/{sessions,agents,clients,policy}.rs`.
- **`brindle-transcript`:** `src/lib.rs` split into `event.rs`, `fold.rs`, `page.rs` and `output.rs`. Its public API is unchanged.
- **`brindle`:** `src/agent/model.rs` → `src/agent/model/`; `src/agent/mod.rs` (`Hold`); `src/agent/conversations.rs`; `src/workspace.rs` → `src/workspace/conversations.rs`; `src/zone_view.rs` → `src/zone_view/login.rs` and `src/zone_tree.rs`; the `use` lines of `layout.rs`, `session.rs`, `close_confirm.rs`, `agent/close.rs` and `agent/conversations.rs`.
- **Tests:** tests move with their code into each file's own `mod tests` (`fcis R8`), and test code changes only in `use` lines and module paths. No transcript, checkpoint or protocol fixture changes. New unit tests cover the type changes. `tests/architecture.rs` gains one check.
- **Docs:** `docs/code-map.md`'s agent, reducer, checkpoint, transcript, server core, workspace and zone entries; `docs/architecture/functional-core-imperative-shell.md`'s whole-pure-modules table; `ports-and-adapters.md`'s R14 enforcement line; `illegal-states-unrepresentable.md`'s `ZoneId` example; `.claude/rules/remote-agent.md`.
- **No wire, checkpoint, transcript, config or state change** (`process R4`–`R6` not engaged).

## Overlap

- **Order:** after `agents-commands` (PR #151) and changes 1–4 (`agents-client-state-fixes`, `agents-zone-focus-and-scroll`, `agents-event-bounds`, `agents-process-lifecycle`); before `agents-checkpoint-deltas` and `agents-absorb-cost`, which are written against this change's files, and before `agents-client-request-marks` (8), which edits `clients.rs`'s `process_client_frames` and `flush_client` and adds `policy::takes_frames` beside this change's gate.
- **What the earlier changes add, and where it goes:** the design's assignment tables include them: change 1's `NotSent`, `ReplyError`, `SessionHandle`, `FirstState`, `Choose` and paging state; change 2's working-row splice, `item_count` and `permission_block`; change 3's `acp/limits.rs` (kept) and `cleaned_len`; change 4's closing step, `Moment::Answered`, `Life::Restarting` and `policy::lost_fate`; and `agents-commands`' option and command handling.
- **Changes in flight that edit these files**, and so rebase over the moves: `agents-prompt-context` (`reduce.rs`, `model.rs`, `forms.rs`, `workspace.rs`, `transcript/src/lib.rs`), `agents-plans` (`reduce.rs`, `model.rs`, `forms.rs`, `transcript/src/lib.rs`), `agents-prompt-images` (`model.rs`, `forms.rs`), `agents-acp-v2` (`forms.rs`, and `Dialect::failure`'s v2 arm), `agents-chat-selection` (`model.rs`). Each replays its edits in the module that now holds the code; the design names the module for each kind of edit.
