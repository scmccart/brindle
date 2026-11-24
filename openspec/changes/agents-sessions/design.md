# Design

## Context

See proposal.md for why. Vocabulary: "ACP agent" and "agent session" are the new thing; "the helper" and "the terminal server" are `brindle-remote`; "the core" is the `serve` role, never pure code. Every change of this work is named `agents-…`; the deferred ones (`agents-log-paging`, `agents-login`, `agents-markdown`, `agents-session-recovery`, `agents-acp-v2`) are proposal-only and don't exist yet.

Facts from the code and the ACP spec that shape the design:

- **The holder is PTY-only.** H1's `SPAWN` (`remote-protocol/src/holder.rs`) requires `cols` and `rows`, and the holder's sessions are a `Pty` and a `Program` from `server/platform/{unix,windows}/pty.rs`. H1 is at version 2 (`holder::PROTOCOLS` 1–2); unknown JSON fields are ignored, so a field that changed what `SPAWN` does would be silently dropped by a holder that doesn't know it: a new spawn mode must be a new message, hence a new version (`process R5`).
- **What a holder keeps per session is already the right shape.** A checkpoint (opaque, ≤ `MAX_CHECKPOINT_LEN`, 64 MiB) and the raw output since (`TAIL_LIMIT`, 1 MiB, then `truncated`). `CHECKPOINT_END.covers` lets the core say how much tail its checkpoint covers. An agent session uses exactly this: the checkpoint is the core's agent state, the tail is the agent's stdout since.
- **Spawns only go to holders of the core's own build** (`core/policy.rs` `placement`, `takes_spawns`); holders of another build are drained (`adoption`). So a core never asks an older-build holder to spawn anything.
- **The client protocol is at 1–3** (`client::PROTOCOLS`), and Brindle offers that same constant (`src/remote/local.rs`). The CLI (`server/cli.rs`) and the relay's own connections offer it too. The relay copies client frames by header and drops only its own `0xF0`–`0xF2` types (`relay.rs`), so new client message types pass through it unchanged.
- **Layouts, the listed directory and the size are generic today.** `client_detach`, `client_set_layout` and `client_attach` (`core/sessions.rs`) store, drop and list `SessionMeta.layout` without looking at the session's kind; `Session::info` lists `meta.cwd` (the spawn's `cwd` as the client sent it) and the session's `cols`/`rows`.
- **Program status today rides in the terminal's bytes** (OSC 7501, parsed by `brindle-term-core`'s `status` into `Records`, replayed by `replay::render`). An agent session has no terminal bytes, so its status has to travel another way.
- **The workspace is five crates, and that number is written down in many places** (checked 2026-11-24): `Cargo.toml` `[workspace] members` and `default-members`; `tests/architecture.rs` `CRATES` (5), `ALLOWED` (4 closed lists), `ROOT_LINTS` (5), `the_workspace_is_the_five_crates_these_rules_cover`, `the_app_and_the_helper_build_on_the_shared_crates`, `no_crate_names_a_forbidden_dependency`'s message; `process R21` ("The five workspace crates MUST share one version"); `ports-and-adapters.md` (its intro's "three shared crates", R1, R3's lists, its Enforcement's "five manifests"); `docs/architecture/README.md` (the dependency diagram, "all three shared crates", "The shared crates never reference each other", the Enforcement paragraph's "all five crates"); `railway-oriented-programming.md` (R2's list of crates, its Enforcement's "all five crates"); `functional-core-imperative-shell.md` (R4, `brindle-term-core` pure as a crate); `CLAUDE.md` (the CI `test` and `clippy` descriptions, the release-please paragraph, the "Architecture rules" dependency rule); `.github/workflows/ci.yml` (the version check's comment; the check itself, `cargo metadata --no-deps` over every package, counts any number of crates); `tests/common/server.rs` `check_helper` (the source directories a fresh helper must be newer than: `remote-agent/src`, `remote-protocol/src`, `term-core/src`). `release-please-config.json` has one package, `"."`, of release type `rust`, and `.release-please-manifest.json` one entry; neither names the member crates, which the `rust` strategy bumps from the root's `[workspace] members` (how the five move today, per `CLAUDE.md`).
- **`remote-agent/build.rs` embeds `brindle-remote.rc`** (the version resource) with `embed_resource::compile`, which links it into every binary of the package.
- **Packaging names its files.** `scripts/package-linux.sh` installs `target/release/brindle` and the musl `brindle-remote`; `scripts/package-windows.ps1` copies `brindle.exe` and `brindle-remote.exe`; `scripts/winget-manifest.ps1` names the two commands. Nothing globs `target/`.
- **The e2e harness's helper check** (`tests/common/server.rs`: `helper`, `freshness`, `check_helper`) finds the helper by a variable, else beside the binary, and requires it to be newer than its sources (this build's) or the same release (an archive's).
- **ACP, verified against the spec (fetched 2026-11-24, `protocol/v1/*`):** JSON-RPC 2.0, UTF-8, one message per line, no embedded newlines; stderr is free-form logging; stdio is the only transport in v1 and in the v2 Preview. `initialize { protocolVersion, clientCapabilities, clientInfo }`; the agent answers with its version (the requested one if it supports it, else its latest), `agentCapabilities`, `agentInfo`, `authMethods`. `session/new { cwd, mcpServers }` may fail with `auth_required` (`-32000`). `session/prompt` is answered, in v1, at the end of the turn with `stopReason` (`end_turn`, `max_tokens`, `max_turn_requests`, `refusal`, `cancelled`). `session/update` carries `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`, `usage_update`, `available_commands_update`, `current_mode_update`, `config_option_update`, `session_info_update` and more. `session/cancel` is a notification; after sending it the client MUST answer every pending `session/request_permission` with `{"outcome":{"outcome":"cancelled"}}`. A permission request is `{ sessionId, toolCall: ToolCallUpdate, options: [{ optionId, name, kind }] }` with `kind` one of `allow_once`, `allow_always`, `reject_once`, `reject_always`; the answer is `{"outcome":{"outcome":"selected","optionId":…}}`. Unknown notifications SHOULD be ignored; a request for a method not offered gets `-32601`. `$/cancel_request` may cancel a pending request. In **v2** (Preview since 2026-11-20) the `session/prompt` response only acknowledges the prompt with a `messageId`; a turn ends with an idle `state_update`; messages, tool calls and terminals are patched by id; a permission request has a required `title` and an optional `subject` instead of a required tool call; enum-like values accept `_`-prefixed unknown variants; an agent can't reject a prompt because it is busy, it holds it.

## Goals / Non-Goals

**Goals:**
- An ACP agent runs in the terminal server and outlives Brindle, core crashes and upgrades, with its conversation, a turn in flight and a queued or held prompt.
- The core is the only ACP speaker. Clients see Brindle's own transcript events and never parse ACP, so v1/v2 differences stay in one place.
- Everything is observable and testable through the helper's protocols and management commands, against a fake ACP agent, with no GUI, on Linux and Windows.
- The holder's addition is as small as a second spawn mode can be.

**Non-Goals:**
- Brindle's GUI, its client for protocol 4, the profile kind, closing with confirmation (all `agents-profiles`).
- ACP v2 (`agents-acp-v2`); in-zone login and `authenticate` (`agents-login`); `session/load`/`session/resume` after an exit (`agents-session-recovery`); paging the log (`agents-log-paging`); markdown (`agents-markdown`, a client matter: the server stores plain text).
- `fs`, `terminal`, elicitation, modes, slash commands, MCP servers, plans, usage and tool call content (diffs, terminal output) in the transcript.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| Transcript events, entries, `Transcript` with `apply`, `replay`, `activity`, `trim_to`; their JSON encoding | pure (types + fold) | **`brindle-transcript`** (`transcript/`), the whole crate |
| Newline framing with its bound (`Lines`) | pure | `brindle-remote-agent` `acp/lines.rs` |
| JSON-RPC envelope and bookkeeping (`Message`, `Rpc`) | pure | `brindle-remote-agent` `acp/rpc.rs` |
| ACP types the reducer reads (schema crate re-export or hand subset) | pure types | `brindle-remote-agent` `acp/wire.rs` |
| The ACP reducer (`step`) and the agent session's checkpoint encoding | pure, takes `now` | `brindle-remote-agent` `acp/reduce.rs`, `acp/checkpoint.rs` |
| Client protocol 4 messages (events carried as opaque JSON), `SessionInfo.agent` with its own `AgentActivity` | codec | `brindle-remote-protocol` `client.rs` |
| H1 3 messages (`SPAWN_PIPES`, `STDERR_TAIL`), `HeldSession.mode` | codec | `brindle-remote-protocol` `holder.rs` |
| Pipe spawn decisions (Windows flags and handle list, hang-up steps, program resolution and the batch-file hint through `conpty::cmdline`) | pure | `brindle-remote-agent`, new `server/platform/piped.rs` (neutral, beside the seams) |
| Pipe spawn: `Piped`, `spawn_piped`, the program's group or job | platform seam | `server/platform/{unix,windows}/piped.rs`, same item names |
| Holder: `SPAWN_PIPES`, three sources per session, the stderr ring, hiding pipe sessions from H1 < 3 cores | shell (loop handler) | `server/holder.rs` |
| Core: agent sessions beside terminal sessions, feeding `step` under `catch_unwind`, applying its output, checkpointing, mapping `Activity` to the wire's `AgentActivity` | shell (loop handler) | `server/core/agents.rs` (new), `core/sessions.rs`, `core/clients.rs`, `core/holders.rs`, `core/manage.rs` |
| Which spawns go where, checkpoint cadence, refusals below protocol 4 | pure | `server/core/policy.rs` |
| `list` shows agent sessions | shell (CLI) | `server/cli.rs` (its `list_line` stays a pure function) |
| The fake ACP agent | test binary, never packaged | `brindle-remote-agent`'s second `[[bin]]`, `brindle-fake-acp-agent` (`remote-agent/src/fake_acp/main.rs`; its script parser pure in `fake_acp/script.rs`) |
| Brindle's offer pinned at 3 | shell | `src/remote/local.rs` |

References still point one way (`ports R1`): `brindle` and `brindle-remote-agent` → `brindle-transcript`, no shared crate names another (decision 6), and only `brindle-remote-agent` names the schema crate. The reducer is the core's alone; the fold is shared so Brindle and the core fold the same way.

## Decisions

### 1. Hosted by the terminal server (confirmed by the user)

The ACP agent runs in the terminal server, not as Brindle's child, so it keeps working when Brindle quits, and an agent profile with a `remote` target runs its agent on that endpoint's server through the existing relay. ACP's stdio transport can't run on a PTY: the line discipline rewrites `\n` and echoes input on Linux, and ConPTY re-renders output as a screen, either of which corrupts JSON. So the holder gets a second spawn mode with plain pipes (decision 2). One agent session runs one ACP agent process with one ACP session (confirmed: one agent per zone; a split starts another agent session in the split zone's directory).

### 2. The holder's addition: one spawn mode, H1 version 3 (`process R1`, `R4`, `R5`)

H1 version 3 (`holder::PROTOCOLS` 1–3) adds two messages and one field, nothing else:

| type | name | direction | payload |
| ---: | --- | --- | --- |
| 15 | `SPAWN_PIPES` | core → holder | `session, command, args, cwd, env, meta` (no grid) |
| 41 | `STDERR_TAIL` | holder → core | session, bytes (binary, ≤ 16 KiB) |
| — | `HeldSession.mode` | in `WELCOME` | `"pipes"` for a pipe session, omitted for a PTY (additive) |

What the holder does with a pipe session, and what it doesn't:
- **Spawn:** the program with stdin, stdout and stderr on pipes and no terminal; answered with `SPAWNED` or `SPAWN_FAILED` as today. The command must be non-empty (no login-shell default); the core refuses an empty one before it gets here.
- **I/O:** `INPUT` is written to stdin, under the same 4 MiB per-session cap. stdout is the session's output: sent as `OUTPUT`, kept as the tail, paused by `PAUSE`, covered by `CHECKPOINT_END`, exactly as a PTY's. The holder doesn't look at the bytes.
- **stderr** is read always (so the agent never blocks on it, even while stdout is paused) into a ring of its newest 16 KiB, which the holder sends once as `STDERR_TAIL` just before `EXITED`. For a session that exited while no core was connected, it follows that session's `TAIL_END` in the adoption. Nothing is sent live: a running agent's diagnostics reach the transcript through ACP errors, and its stderr is for explaining an exit.
- **`RESIZE` and `REPARENT`** for a pipe session get an `ERROR` for that session (a second guard: the core never sends them, decision 9).
- **Hang-up** (`HANGUP`, `STOP`, a `kill` owner gone): close stdin (ACP's polite end: the spec's sequence is "close stdin, terminate subprocess"), then after `pty::GRACEFUL_SHUTDOWN` (1 s) end the program's process group (Linux: `SIGHUP`, then `SIGKILL` while members remain, as `Group` does today) or job (Windows: `TerminateJobObject`).
- **A core that negotiated H1 below 3** (only an older core adopting a newer holder: a downgrade or a wrong winner, `process R12`) is not shown pipe sessions: `WELCOME` leaves them out and their output only grows the tail, which truncates at `TAIL_LIMIT`. They keep running and come back when a core that speaks 3 adopts the holder. Such a core never spawns on this holder anyway (it drains holders of another build), so it can't reuse a hidden session's id.

That is the whole addition: one spawn path, one more readable source per session, a fixed-size ring, and one filter in `WELCOME`. The holder still parses nothing and holds no policy.

**A holder of an older build** can't run pipe sessions, and is never asked to: `placement` sends spawns only to holders of the core's own build, which speak H1 3, and starts one when there is none. The only way a client meets a server that can't host agents is a core older than protocol 4 (decision 9), which a newer Brindle upgrades first (`process R12`). When that upgrade is refused (the `check` fails because a running holder's H1 is too old for the new binary), the client stays on protocol ≤ 3, and `agents-profiles` shows that the terminal server is too old for agent sessions.

### 3. The pipe spawn seam on each platform (`ports R4`, `R5`, `R7`)

`server/platform/{unix,windows}/piped.rs` export the same items: `Piped` (stdin, stdout and stderr handles, each giving a `Source`, read and written through the owner as `process R8` requires), `spawn_piped(&PipeCommand) -> io::Result<(Piped, Program)>`, and the existing `Program`/`Group` for exit and hang-up. The holder names none of their platform types. Decisions are in the neutral `server/platform/piped.rs`, pure and tested on both platforms.

- **Linux:** three `pipe2(O_CLOEXEC)` pairs, the holder's ends non-blocking; the child calls `setsid` (its own session and process group, no controlling terminal) and resets signal dispositions and mask in `pre_exec` as PTY programs do (`Programs start with default signal handling`). The directory is chosen by the pure `pty::program::start_dir` as for PTYs.
- **Windows:** no pseudoconsole. Three pipes named like PTY pipes (`server_dir::pty_pipe`, first instance, the server pipes' security), the holder's ends overlapped so they are `Source`s on the IOCP loop, the child's ends synchronous and inheritable, passed with `STARTF_USESTDHANDLES` and a `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` naming only them. Flags: `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT`. The program is resolved by `conpty::cmdline::resolve_program` and quoted by `launch_line`, the environment passed as given, as for ConPTY sessions. Each pipe session gets a job object (nested jobs work since Windows 8), so a hang-up ends the processes the agent started; `KILL_ON_JOB_CLOSE` is set, so an agent dies with its holder, as its sessions do anyway. Exit is watched with `Waker::wake_on_exit`. The `windows` crate gains the `Win32_System_JobObjects` feature in the agent's non-dev dependencies (a feature of an allowed crate, not a new dependency).
- **Batch files on Windows (confirmed by the user).** A bare name that is a batch file (`npx` is `npx.cmd`) isn't found, as for terminal sessions (`Bare program names resolve through PATH on Windows`): an agent profile names `cmd /c npx …`. When resolution fails and a `.cmd` or `.bat` of that name is on `PATH`, the spawn's error says so and shows the form to use: "npx is a batch file (C:\…\npx.cmd); run it as `cmd /c npx …`". The pure `piped::batch_hint(name, args, path_var, exists) -> Option<String>` decides it, beside `resolve_program`; the error reaches the client as the `SPAWN_FAILED` message, which `agents-profiles` shows in the zone. *Rejected:* resolving batch files and launching them through `cmd /c` implicitly, which brings cmd's quoting rules into every agent's arguments without the user seeing it.

### 4. The core is the ACP client (confirmed by the user), and who answers what

The core does `initialize` and `session/new`, keeps the state and answers the agent; Brindle is a view. The analogue of `process R19` ("the core never answers a terminal's queries"):

| The agent sends | Answered by | How |
| --- | --- | --- |
| responses to `initialize`, `session/new`, `session/prompt` | the core | matched by id in `Rpc` |
| `session/update` | the core | reduced into transcript events |
| `session/request_permission` | the attached client's choice, relayed by the core | the core holds the request until a client's `AGENT_PERMISSION` or a cancel; never on its own |
| `$/cancel_request` for a pending permission request | the core | records outcome `cancelled`, answers `cancelled` |
| `fs/*`, `terminal/*`, `elicitation/create`, `_…` and any other request | the core | `-32601`, since none was offered |
| any other notification | nobody | ignored |

`initialize` asks for version 1 with `clientCapabilities: {}` (no `fs`, no `terminal`, no `auth.terminal`, no elicitation) and `clientInfo { name: "brindle", version }`. `session/new` gets `cwd` resolved to an absolute path by the same rule as a PTY session's directory (`~` expanded with the spawn environment's `HOME`; a relative or missing one is the home directory), and `mcpServers: []`.

**An agent that doesn't speak ACP version 1.** ACP's rules (checked 2026-11-24 against the spec's `llms-full.txt`): in v1 an agent that doesn't support the requested version "MUST respond with the latest version it supports", and the client "SHOULD close the connection and inform the user"; in v2 an agent MUST choose the highest version at or below the requested one, and if none exists MUST reject `initialize` with `-32022` (Unsupported protocol version) and `data { requested, supported }` instead of answering with a higher version. So two answers reach the core:
- the well-behaved refusal of a v2-only agent: an `initialize` error `-32022`, whose message the core builds from `data.supported` ("the agent supports ACP versions 2; Brindle speaks 1"; with `data` missing or malformed, "the agent supports no ACP version Brindle speaks (1)");
- a success whose `protocolVersion` isn't 1 (correct for a v1-only agent newer than Brindle, a violation for a v2 agent): "the agent speaks ACP version N; Brindle speaks 1".

Either way, and for any other `initialize` error (the agent's own message), the result is the same and is stated once here: the agent becomes `AgentState::Failed { message }` with that message, the core sends nothing more and hangs the agent up (closes its stdin, then ends it after the grace, decision 2), and the session ends when the agent has exited, as every exit does. **The version message is what the user is left with:** once the agent is `Failed`, its exit sends no `Exited` event that would replace it; a non-empty stderr tail is added as a `Notice` entry, then the session's `EXIT` follows. `agents-profiles` keeps the zone open showing the last agent state, so it shows the version message. (A `Failed` from a fault in the core's own ACP handling, decision 13, is different: the agent wasn't at fault, so it is left running until the session is closed.) Each case has a scenario and a fixture (`initialize-32022.jsonl`, `initialize-v2-reply.jsonl` under `remote-agent/tests/fixtures/acp/`). *Rejected:* keeping a version-mismatched agent running in an open session, which ACP says to close and which can do nothing.

**Permission requests wait with no timeout (confirmed by the user).** With no client attached, a request waits for as long as it takes: the agent is blocked, the session is listed as `blocked`, and the request is in the log for the next client to answer. Nothing is auto-allowed and nothing is rejected silently (user decision 1). Pending requests are capped at 16 per session; one past the cap is answered `cancelled` at once with a notice in the log. *Rejected:* a timeout, which would answer for the user.

### 5. ACP wire types: the schema crate, decided by a spike (confirmed by the user)

ACP's types come from `agent-client-protocol-schema` (1.22.0 on 2026-11-23, Apache-2.0, MSRV 1.88; the toolchain is 1.99.1), pinned with `=1.22.0`, moved by hand, as a dependency of `brindle-remote-agent` alone (decision 6). Not the runtime crate `agent-client-protocol` (async, drives its own IO, needs `futures` 0.3.32 against the lock's 0.3.31), and no JSON-RPC crate: newline framing (`lines.rs`) and request-id bookkeeping (`rpc.rs`) are Brindle's own pure code. The JSON-RPC envelope is decoded by hand into `Message::{Request, Response, Error, Notification}` with `params` kept as `serde_json::Value`, and each known method's params are then decoded into the schema crate's type. That two-step decode is what makes an unknown method, an unknown update kind or a malformed message one skipped item, not a lost connection.

The spike (task group 1) decides between the crate and a hand-written subset of the types:
1. decode captured stdout of two or three real ACP agents with the pinned crate, plus a message with an unknown `sessionUpdate` kind and an unknown enum value inside a known one, to learn whether the crate rejects a whole message when an agent is newer than the pin;
2. build the static musl helper with it and compare size and build time against `main`;
3. check that the `Cargo.lock` diff only adds packages (the crate brings `serde_with` ^3.20, `strum` 0.28 and `derive_more` 2; the lock has `strum` 0.24/0.26/0.27 and `derive_more` 0.99, and no `serde_with`), with no version of an existing package moved (`CLAUDE.md`'s `Cargo.lock` rule).

If any check fails, `wire.rs` holds a hand-written subset instead (the `initialize`, `session/new`, `session/prompt`, `session/cancel`, `session/update` kinds the reducer reads, and `session/request_permission`, with `#[serde(other)]` catch-alls), and `brindle-remote-agent`'s closed list doesn't gain the schema crate. The rest of the design is unchanged, because the reducer reads `wire.rs`'s names. A whole-message rejection found in check 1 is not a failure by itself if the two-step decode confines it to the one update.

### 6. A sixth crate, `brindle-transcript`, and the ACP code in the helper (confirmed by the user)

**What it is.** `brindle-transcript`, directory `transcript/`, `brindle_transcript` in code: a shared crate below the app like the other three, holding only the transcript event types and their fold, which Brindle and the core share. It has no cargo feature. It is pure as a crate (no filesystem, network, process, environment, thread or clock call, as `fcis R4` holds `brindle-term-core`), returns its own error types (`railway R2`), and its root switches on `clippy::wildcard_enum_match_arm`, `clippy::unwrap_used` and `clippy::expect_used`, since it decodes what the wire sends.

**What goes in it:** `src/lib.rs` (the types of decision 7, `Transcript::{apply, slot, replay, activity, trim_to}`, `FoldError`, `Event::{to_value, from_value}` and `DecodeError`), `src/fuzz.rs` (the event decoder and the fold over arbitrary input), and fixture tests pinning each event's encoding (`transcript/tests/events.rs`, `transcript/tests/fixtures/v4/`, written only when missing, never edited, as `remote-protocol`'s are).

**Its closed list (`ports R3`):** `serde`, `serde_json`. `brindle-remote-agent`'s list gains `brindle-transcript` and, if the spike keeps it, `agent-client-protocol-schema`; the app has no closed list and names `brindle-transcript` only.

**The ACP code lives in the helper, its only user.** `remote-agent/src/acp/` holds `wire.rs`, `lines.rs`, `rpc.rs`, `reduce.rs` and `checkpoint.rs` (decisions 5, 8, 11), pure modules of the helper like `server/core/policy.rs`, each listed in `PURE_MODULES` and `fcis`'s pure-module table, so `the_pure_modules_read_nothing_ambient` holds them pure. Only these modules name the schema crate (a new architecture test, below), so the core's shell reaches ACP through `acp::reduce` and never decodes ACP itself. Because Brindle doesn't depend on any crate that names the schema crate, no build, whole-workspace or not, puts it in Brindle's build graph; a feature could not have done that, since cargo unifies a dependency's features across the members of one build. *Rejected:* a `brindle-acp` crate with the reducer and the schema crate behind a `reducer` feature (round 2), for that reason.

**How client protocol 4 carries transcript events without a shared crate naming another's types.** `brindle-remote-protocol` keeps owning frames, messages and versions, and carries the events as data it doesn't interpret, as it carries `layout` and `meta`: `TRANSCRIPT { session, events: Vec<serde_json::Value> }`, each value a JSON object. The protocol crate bounds the frame (`process R7`) and checks that `events` is an array of objects; `brindle_transcript::Event::from_value` gives them meaning, on both sides. The core encodes with `Event::to_value`; Brindle decodes with `from_value` and folds. The two crates agree through the JSON shape, which `brindle-transcript`'s fixtures pin and the protocol's `TRANSCRIPT` fixtures carry. The listing's activity is the protocol's own closed enum, `client::AgentActivity { Idle, Working, Blocked, Done, Error }` (wire names `idle`…`error`), not `brindle_transcript::Activity`: the core maps one onto the other with an exhaustive `match` in `core/agents.rs`, the only code that names both. `AGENT_PERMISSION`'s `entry` is a plain `u64` (`types R4`: ids inside protocol structs stay the integers they are); `brindle_transcript::EntryId` wraps it on each side. *Rejected:* `brindle-remote-protocol` depending on `brindle-transcript` (or the reverse), which `ports R1` forbids; and copying the event types into the protocol crate, which would make two definitions to keep equal.

**Event forms are frozen per client protocol version (`process R4`–`R6`).** A transcript event is part of client protocol 4's format, so `process R5` applies to it as to a message: the event kinds, and the values of every closed enum inside an event (`Entry`'s kinds, `ToolKind`, `ToolStatus`, `NoticeLevel`, `TurnState`, `StopReason`, `AgentState`, `Slot`, `HoldReason`, `PermissionOutcome`), are fixed for protocol 4. Within 4 an event may only gain optional fields, defaulted and omitted when unset, and `from_value` ignores fields it doesn't know. A new event kind or enum value is a new client protocol version, and the core sends only forms of the version a connection negotiated (a later change that adds one says what the core sends a protocol 4 client instead). So a client never meets an event it can't decode from a correct core, and a decode error stays what Brindle treats it as: a broken session. Values that come from the agent and can't be listed in advance already have their form (`StopReason::Other { name }`, a tool's free-text title). *Rejected:* a skippable "unknown event" form, which would let a fold skip an event whose effect later events assume (an entry it never added), turning a version problem into a silent `FoldError` later. Pinned by: the `v4/` fixtures (one per kind and value, never edited), a test that each v4 fixture with an extra unknown field decodes to the same event, and a test that an unknown kind or value is a `DecodeError`.

**What the sixth crate changes,** each in a task of group 2:

| File or rule | Today | Change |
| --- | --- | --- |
| `Cargo.toml` | `members` and `default-members` list five | add `"transcript"` to both; the root package depends on `brindle-transcript` |
| `process R21` (`process-architecture.md`) | "The five workspace crates MUST share one version" | "six"; the process table's core row names `brindle-transcript` beside the protocol crate |
| `docs/architecture/README.md` | diagram of three shared crates; "all three shared crates"; Enforcement's "all five crates" and the restriction-lint list | `brindle-transcript` in the diagram with `serde`; its bullet (pure, the fold Brindle and the core share, closed list); "all four shared crates"; "all six crates"; `brindle-transcript` among the `unwrap_used`/`expect_used` crates; the "Read by layer" table gains `transcript/**` (fcis, types, railway, process) and `remote-agent/src/acp/**` (fcis, railway) |
| `ports-and-adapters.md` | intro's "three shared crates"; R1; R3's lists; Enforcement's "five manifests" | "four shared crates"; R1 names `brindle-transcript`; R3 adds `transcript`'s list, and `brindle-transcript` and `agent-client-protocol-schema` to the agent's; "six manifests" |
| `functional-core-imperative-shell.md` | R4 makes `brindle-term-core` pure as a crate; the pure-module table | R4 also binds `brindle-transcript`; the table gains `remote-agent/src/acp/{wire,lines,rpc,reduce,checkpoint}.rs` |
| `railway-oriented-programming.md` | R2 lists four crates; Enforcement's "all five crates" and the two lint crates | R2 adds `brindle-transcript`; "all six crates"; `brindle-transcript` among the lint crates |
| `tests/architecture.rs` | `CRATES` 5, `ALLOWED` 4, `ROOT_LINTS` 5, `term_core_does_no_io_and_reads_no_clock`, `PURE_MODULES` 42, `the_workspace_is_the_five_crates_these_rules_cover`, `the_app_and_the_helper_build_on_the_shared_crates`, `no_crate_names_a_forbidden_dependency`'s message | `CRATES` 6 (`("transcript", "brindle-transcript")`), `ALLOWED` 5 (`transcript`: `serde`, `serde_json`; the agent's gains `brindle-transcript` and `agent-client-protocol-schema`), `ROOT_LINTS` 6, `PURE_MODULES` gains the five `acp/` modules (and `piped.rs`, `fake_acp/script.rs`), the test renamed `…_six_crates_…`, both binaries must depend on `brindle-transcript`, the message names it; new: `transcript_does_no_io_and_reads_no_clock` (the whole crate, as for term-core), `only_the_acp_modules_name_the_schema_crate` (no file outside `remote-agent/src/acp/` names `agent_client_protocol_schema`, and no manifest but the helper's lists it) |
| `.github/workflows/ci.yml` | the version check counts every package of `cargo metadata`; its comment says "all five crates" | comment only |
| `release-please-config.json`, `.release-please-manifest.json` | one root package, `rust` type; the strategy bumps `[workspace] members` | no change: adding the member is enough; the version check on the release PR catches a crate left behind |
| `CLAUDE.md` | "five workspace crates" (CI `test`), "all five crates" and the two lint crates (CI `clippy`), "bumps all five crates" (release-please), the "Architecture rules" dependency rule | "six"; `brindle-transcript` among the lint crates; the dependency rule names it |
| `tests/common/server.rs` `check_helper` | a helper must be newer than `remote-agent/src`, `remote-protocol/src`, `term-core/src` | add `transcript/src`, since the helper is built from it |
| `.claude/rules/` | per-crate reminders for the others | a new `transcript.md` (pure, closed list, event forms frozen per protocol version, fixtures never edited); `remote-agent.md`'s closed list gains `brindle-transcript` and the schema crate, and says ACP is decoded only in `src/acp/` |
| `docs/code-map.md` | no `brindle-transcript` | an entry for it, and the helper's `acp/` modules |

### 7. The transcript: Brindle's own events, folded the same way on both sides (confirmed by the user)

The log holds Brindle's transcript events, not ACP lines (user decisions 4 and 5). In `brindle_transcript`:

```rust
pub struct EntryId(pub u64);              // per session, from 1, never reused (types R3)

pub enum Entry {
    User { text: String },
    Agent { text: String },
    Thought { text: String },
    Tool { call: String, title: String, kind: ToolKind, status: ToolStatus },
    Permission { call: Option<String>, title: String, options: Vec<PermissionOption>,
                 outcome: Option<PermissionOutcome> },
    Notice { level: NoticeLevel, text: String },
}
pub enum Event {
    Added { id: EntryId, entry: Entry },
    Appended { id: EntryId, text: String },          // User, Agent, Thought
    ToolChanged { id: EntryId, title: Option<String>, kind: Option<ToolKind>, status: Option<ToolStatus> },
    Answered { id: EntryId, outcome: PermissionOutcome },
    Queue { slot: Slot },                            // the whole slot, set as given (decision 10)
    Turn { state: TurnState },
    Agent { state: AgentState },
    Trimmed { through: EntryId, dropped: u64 },
}
pub enum Slot { Empty, Waiting { text: String }, Held { text: String, reason: HoldReason } }
pub enum HoldReason { Cancelled, Failed, Stopped { stop: StopReason }, LoginRequired, AgentStopped, OutputLost }
pub enum TurnState { Idle, Working, Blocked, Cancelling, Ended { stop: StopReason }, Failed { message: String } }
pub enum StopReason { EndTurn, MaxTokens, MaxTurnRequests, Refusal, Cancelled, Other { name: String } }
pub enum AgentState { Starting, Ready { name: Option<String>, version: Option<String> },
                      LoginRequired { message: String }, Failed { message: String },
                      Exited { code: Option<i32>, stderr: String } }
pub enum PermissionOutcome { Selected { option: String }, Cancelled }
pub enum Activity { Idle, Working, Blocked, Done, Error }   // program status's names
pub struct Transcript { /* entries in id order, next id, the queue's slot, turn, agent, dropped */ }
```

- **Updates are by id, the end of a turn is a state event** (`Turn`), never inferred from a response: that is the shape ACP v2 has (patched by id; a prompt's response only acknowledges it; an idle `state_update` ends the work), so `agents-acp-v2` changes only the reducer (confirmed: ACP v1 only, shaped so v2 fits).
- **Chunks merge:** the reducer turns the first chunk of a run into `Added` and the rest into `Appended` on that entry; a chunk of another kind, a tool call or a turn's end ends the run. The fold appends; the log keeps one entry.
- **`apply` is the one fold.** The core applies each event to its log before sending it; Brindle (`agents-profiles`) applies the same events to its copy. `replay()` returns the events that rebuild the log (`Trimmed` first when entries were dropped, one `Added` per entry with its text split into `Appended`s of at most 64 KiB, the outcomes, `Queue { slot }` when the slot isn't empty, then `Turn` and `Agent`), so a client's fold of a replay equals the server's log: a property test pins `fold(replay(t)) == t`.
- **`FoldError`** (an id that doesn't exist, `Appended` to a tool entry, an id out of order; `Queue` never fails, since it sets the whole slot) is an outcome, not a panic (`railway R1`, `R5`): the core treats it as a bug in its reducer and breaks only that session (decision 13). What a client does with one is the client's decision; the server guarantees only that a later `ATTACH` replays the whole log from its first kept entry, whatever that client received before. (`agents-profiles` detaches the session and leaves the reattach to the user, as for a broken terminal.)
- **Invalid states ruled out by the types:** a permission's outcome exists only on a permission entry; a stop reason only on an ended turn; an exit status only on an exited agent; "ready" and "login required" can't both hold; the queue holds at most one prompt, and a held one always has its reason (`Slot`); an `EntryId` is never an index. `Other { name }` keeps a stop reason a newer agent sends without a sentinel (`types R7`).
- **Bounds:** each text field and each `Appended` carries at most 64 KiB (longer text is split), a queued prompt at most 256 KiB, and a `TRANSCRIPT` frame holds whole events up to the 1 MiB frame limit (`process R7`).

### 8. The ACP reducer is pure (`fcis R1`–`R3`, `R9`)

`acp::reduce::step(&mut AgentSession, Input, now) -> Output`, where `Input` is one of: a line from the agent (`Lines` has already split and bounded it), a client command (prompt, unqueue, cancel, permission answer), a holder event (spawned, exited with its stderr tail), and `Output` is data: transcript events to apply and send, bytes to write to the agent's stdin, replies to the client's request, and whether to checkpoint now or hang the agent up. The core's handlers gather, call `step` and apply the output; they decide nothing. `AgentSession` holds the `Transcript`, `Rpc` and `Lines`. Text from the agent is cleaned here: control characters other than `\n` and `\t` become U+FFFD, so no client ever renders an escape sequence from an agent.

Cancel, per the spec: a waiting prompt is held first (`Queue { Held { reason: Cancelled } }`, decision 10), then `session/cancel` goes to the agent, then `cancelled` for each pending permission request with `Answered` events, and the turn becomes `Cancelling`; it ends (`Ended { Cancelled }` or whatever stop reason the agent gives) when the prompt's response arrives. A cancel with no turn running produces nothing.

### 9. Client protocol 4 (`process R4`–`R6`)

`client::PROTOCOLS` becomes 1–4. Protocol 4 adds these messages, sent only on connections that negotiated 4:

| type | name | direction | payload | reply |
| ---: | --- | --- | --- | --- |
| 15 | `SPAWN_AGENT` | client → core | `req, command, args, cwd, env, policy, profile?` | `SPAWNED` or `ERROR` |
| 16 | `AGENT_PROMPT` | client → core | `req, session, text` | `OK` (sent or queued) or `ERROR` |
| 17 | `AGENT_CANCEL` | client → core | `session` | none; a refused one gets `ERROR` |
| 18 | `AGENT_PERMISSION` | client → core | `req, session, entry, option?` (none: cancelled) | `OK` or `ERROR` |
| 19 | `AGENT_UNQUEUE` | client → core | `req, session` (discards the slot's prompt, waiting or held) | `OK` or `ERROR` |
| 20 | `AGENT_RESEND` | client → core | `req, session` (sends a held prompt again) | `OK` (sent or waiting) or `ERROR` |
| 42 | `TRANSCRIPT` | core → client | `session, events: [object]` (decision 6) | — |

and one additive field: `SessionInfo.agent?: { activity: AgentActivity, name? }`, set only for agent sessions (which only protocol 4 lists).

- **Attach and replay reuse the existing exchange:** `ATTACH` → `REPLAY_START`, `TRANSCRIPT`…, `REPLAY_END`, then live `TRANSCRIPT`s. The replay is `Transcript::replay()`, ordered as a terminal's (`Session::storing`: it waits for the session's last `SET_META` and checkpoint to be in its holder's connection).
- **Prompts, unqueues, resends, cancels and permission answers** are accepted only from the client that has the session attached (as `SET_LAYOUT` is), else `ERROR` naming the session. Whether a prompt was sent or queued, the client learns from the events (`Added { User }` or `Queue`), not the reply.
- **Layouts work as for terminals.** `client_detach`, `client_set_layout` and `client_attach` store, drop and list the layout whatever the session's kind, so agent sessions get `DETACH { layout }`, `SET_LAYOUT` (protocol 3 and up, so every client that can attach one), `MAX_LAYOUT` and the survival through `SET_META` with no new code; this change pins it with tests, since Brindle's restore and sweep group agent tabs by it.
- **The listed directory is the resolved one.** For an agent session the core stores the absolute directory it resolved for `session/new` (decision 4) as `SessionMeta.cwd`, so `SessionInfo.cwd` is what the agent works in and a client can start a split or a new tab there. (A terminal's stays the spawn's `cwd` as sent.)
- **No size.** An agent session has no grid: it is listed with `cols` and `rows` 0, `ATTACH`'s `cols`/`rows` are ignored for it, and a `RESIZE` for it gets an `ERROR` naming the session from the core, so it never reaches the holder.
- **What an older peer sees.** A client on 1–3: agent sessions are left out of `SESSIONS`, an `ATTACH` of one gets "session N is an agent session, which needs client protocol 4", a reconnect never gives one back, and `KILL`/`STOP` end it as any session. A core on 1–3 meeting a client that offers 4: they agree on 3; the client sends none of the new messages. A v4 core never sends `TRANSCRIPT` below 4.
- **Brindle's offer stays at 1–3 in this change** (a `BRINDLE_PROTOCOLS` constant in `src/remote/local.rs` in place of `client::PROTOCOLS`), so a Brindle of this release never meets an agent session; `agents-profiles` raises it with the client code. Brindle's exhaustive `match`es on `ToClient` get the `Transcript` arm (logged and dropped: never sent below 4).
- **Relay:** nothing changes. It copies frames by header, its own connections' `HELLO` offers `client::PROTOCOLS` (it decodes only `WELCOME` and status replies there), and a Brindle's `HELLO` passes through to the endpoint's core.
- **Event forms are frozen for protocol 4** (decision 6): no new event kind or enum value within it; a later version adds them, and the core sends a protocol 4 connection only protocol 4's forms.
- **Transcripts that pin it** (`remote-protocol/tests/transcripts.rs`, fixtures added, none edited, `BRINDLE_REGEN_FIXTURES=1` writes only missing ones): `client-agent-session` (spawn, a prompt, a queued prompt and an unqueue, a held prompt and a resend, a permission answer, cancel, `TRANSCRIPT` frames with sample event objects), `client-agent-attach` (replay then live), `client-agent-old-client` (a protocol 3 client: list without agent sessions, refused attach), `holder-spawn-pipes` (spawn, I/O, stderr tail, exit), `holder-pipes-adopt` (adoption with `mode: "pipes"`, a checkpoint and a tail), `holder-pipes-old-core` (an H1 2 core: the pipe session not in `WELCOME`). `every_message_variant_and_form_appears_in_a_transcript` gains every new variant and `AgentActivity` form. The event objects themselves are pinned in `brindle-transcript`'s `v4/` fixtures (every `Event`, `Entry`, `TurnState`, `AgentState`, `StopReason`, `Slot`, `HoldReason` and `PermissionOutcome` form, with the unknown-field and unknown-kind tests of decision 6). `remote-protocol`'s `fuzz.rs` covers the new decoders; `brindle-transcript`'s covers `Event::from_value` and `apply`; the helper's covers `Lines`, the envelope decoder and checkpoint decoding.

### 10. A queue of one prompt (confirmed by the user)

A prompt sent while a turn runs waits in the session's one queue slot and is sent only when that turn ends normally; a prompt that isn't sent stays in the slot, held, until the user sends it again or discards it. Confirmed by the user: the queue itself, a third prompt refused, sending only after an `EndTurn` ending, and keeping an unsent prompt apart from the draft. That a held prompt must survive a detach (so it lives in the transcript, the checkpoint and the replay, not only in a client) is the coordinator's decision, drawn from the user's choice that an unsent prompt is never lost into or with the draft.

**The slot** is `Transcript.slot: Slot`, a closed type (decision 7):
- `Empty`;
- `Waiting { text }`: queued, waiting for the running turn to end;
- `Held { text, reason }`: not sent, waiting for the user. `HoldReason` is `Cancelled` (the user stopped the turn), `Failed` (the agent answered the prompt with an error), `Stopped { stop }` (the turn ended with a stop reason other than `EndTurn` and `Cancelled`: `MaxTokens`, `MaxTurnRequests`, `Refusal`, `Other`), `LoginRequired` (the agent answered the running prompt with `-32000`, decision 17), `AgentStopped` (the agent stopped being ready: ACP handling failed, or it exited), or `OutputLost` (a truncated restart failed the turn).

**Its one event** is `Queue { slot }`, which sets the whole slot. The fold is total for it (any slot follows any slot), a client needs nothing but the event to show the slot, and `replay()` emits `Queue { slot }` whenever the slot isn't `Empty`, so a held prompt reaches a client that attaches later. The prompt's text (≤ 256 KiB) travels in the event; a `TRANSCRIPT` frame still holds whole events within 1 MiB.

**Transitions** (all in the reducer, each followed by a checkpoint):

| From | On | To, and what else happens |
| --- | --- | --- |
| `Empty` | `AGENT_PROMPT` while a turn runs | `Waiting { text }` |
| `Waiting` | the turn ends `Ended { EndTurn }` with the agent `Ready` | `Empty`, then `Added { User }`, the prompt is written and the turn is `Working` |
| `Waiting` | a cancel (before `session/cancel` is sent) | `Held { Cancelled }` |
| `Waiting` | the turn ends `Failed` | `Held { Failed }` |
| `Waiting` | the turn ends `Ended` with any stop reason but `EndTurn` and `Cancelled` | `Held { Stopped { stop } }` (a `Cancelled` ending follows a cancel, which already held it) |
| `Waiting` | the running prompt is answered `-32000` | `Held { LoginRequired }`, emitted before the `Turn` and `Agent` events |
| `Waiting` | the agent becomes `Failed` or `Exited` | `Held { AgentStopped }`, emitted before the `Agent` event |
| `Waiting` | adoption after a truncated tail fails the turn | `Held { OutputLost }` |
| `Waiting` or `Held` | `AGENT_UNQUEUE` | `Empty`; the prompt is discarded |
| `Held` | `AGENT_RESEND`, the agent `Ready` and no turn running | `Empty`, then `Added { User }` and the prompt is sent as a new turn |
| `Held` | `AGENT_RESEND` while a turn runs | `Waiting { text }` (queued again) |
| `Held` | `AGENT_RESEND` while the agent isn't `Ready` | unchanged; `ERROR` "the agent isn't ready" |
| `Held` | `AGENT_PROMPT` with no turn running | unchanged; the new prompt is sent ahead of it (see below) |
| `Waiting` or `Held` | `AGENT_PROMPT` while a turn runs | unchanged; `ERROR` "a prompt is already queued" |
| `Empty` | `AGENT_UNQUEUE` or `AGENT_RESEND` | unchanged; `ERROR` "nothing is queued" or "no prompt is held" |
| `Waiting` | `AGENT_RESEND` | unchanged; `ERROR` "no prompt is held" |

**A new prompt while one is held** (confirmed by the user): with no turn running it is sent as usual and the held prompt stays held, untouched, so the user can still send or discard it; while a turn runs it is refused, since the slot is taken. The held prompt waits for the user's decision, not for a turn, so it shouldn't block a new prompt, and nothing replaces or reorders it without the user asking. *Rejected:* refusing every prompt until the held one is dealt with (blocks the input on a row the user may not have noticed); replacing it (loses text silently).

**What survives:** the slot is part of the `Transcript`, so the checkpoint keeps it. A core restart or upgrade keeps `Waiting` (sent when the turn ends normally) and `Held` (still held). A Brindle crash, quit or detach keeps both; the next attach replays them. An agent exit holds a waiting prompt (`AgentStopped`) and then ends the session, as every exit does; the attached client keeps the conversation it has, held prompt included (`agents-profiles` keeps the zone open), but the server keeps nothing after the session's `EXIT` until `agents-session-recovery`. `AgentState::Failed` from a fault in the core's ACP handling (decision 13), and `LoginRequired` (decision 17), keep the session and its held prompt, which can be discarded but not resent (the agent isn't ready). A `Failed` from a version mismatch (decision 4) ends the session once the agent has exited, as an exit does.

**Activity:** the slot doesn't change it (decision 15); the turn it waits for, or the ending that held it, does.

*Rejected:* refusing every prompt during a turn (round 1's proposal); a queue of more than one, which would need ordering, editing and a bound the zone has to show; sending a queued prompt after any ending but `EndTurn` (round 3); handing an unsent prompt back to the client in a `Returned` event (round 3), which replay couldn't carry, so a prompt held while no client was attached was lost.

### 11. What survives what (`process R2`, `R3`, `R12`)

**The checkpoint** of an agent session is the helper's `acp::checkpoint` encoding: a small header (a format version, `1`), the JSON state (the `Transcript` with its queue slot, the ACP session id, the negotiated ACP version, `Rpc`: the next outgoing id, the outgoing requests awaiting a response by id and what each was for, the agent's pending permission requests by entry id with their JSON-RPC ids), then the raw bytes of the partial line `Lines` holds. `covers` counts every stdout byte fed, partial line included. A newer core reads an older format (additive fields with defaults, `process R15`); a format it can't read is adopted as a log holding one notice ("the conversation could not be restored") with the agent kept running and `AgentState::Failed`, so prompts are refused rather than sent with lost ids.

**When the core checkpoints an agent session:** after the turn ends, after every change of the queue slot, after a permission request arrives and after it is answered, right before it sends a prompt (so the user's entry and the prompt's id reach the holder ahead of the prompt's bytes on the same connection), after 512 KiB of stdout (`CHECKPOINT_AFTER`, as terminals), before a step-down and on detach. The pure `policy::agent_checkpoint_due` decides.

| Event | What happens to an agent session |
| --- | --- |
| core crash or `SIGKILL` | The agent keeps running under its holder; its stdout piles up in the tail. The next core adopts: decodes the checkpoint, feeds the tail through `Lines` and `step` (responses match by the checkpointed ids, permission requests reappear in the log, a waiting prompt is kept and sent if the turn ends normally, a held one stays held), then replays to reconnecting clients. A turn in flight continues; pending permissions are still pending. |
| core crash with a truncated tail (> 1 MiB of stdout with no core) | The front of the tail is gone mid-line: `Lines` drops up to the first newline, the log gets a notice that output was lost, an outgoing prompt still awaiting its response is dropped from `Rpc` and the turn set to `Failed { "output was lost while the terminal server restarted" }`, since its end may have been in the lost bytes, and a waiting prompt is held (`OutputLost`). The agent keeps running and the next prompt works. |
| upgrade (`STEP_DOWN`) | The old core checkpoints every session, agent sessions included, then `GOODBYE { handover }`; the new core adopts as above, with an untruncated tail. Mid-turn, the queue slot (waiting or held) and pending permissions are kept. |
| Brindle crash | The session's policy applies as for a terminal: `kill` sessions end after the owner is gone (or linger if asked), `detach` sessions run on. A detached agent keeps working; a permission request waits; a waiting prompt is sent if the turn ends normally and held otherwise; the next attach replays everything, a held prompt included. |
| Brindle quit | `agents-profiles` detaches the zone's session as terminals are (confirmed: quitting never asks and leaves agents running). |
| a holder crash | Its sessions are lost, agent sessions included (`EXIT` with `lost`), as today. With `KILL_ON_JOB_CLOSE` on Windows, and the agent's pipes closing on Linux, the agent ends too. |

**Known gap:** a core killed between writing a prompt's checkpoint and the prompt's `INPUT` to the holder leaves a prompt in the log the agent never received; its turn shows working until the user cancels, which then ends it locally (`Cancelling` with no prompt in `Rpc` resolves to `Ended { Cancelled }`). Both messages go out in the same flush, so the window is one partial write.

### 12. Queues and their marks (`process R9`)

| Queue | Bound |
| --- | --- |
| agent stdout → core | the holder's existing `CORE_QUEUE_HIGH`/`LOW` (8 MiB / 2 MiB): it stops reading the agent's stdout, so the agent blocks on its write |
| core → client (`TRANSCRIPT`) | the client's existing queue marks (1 MiB / 256 KiB): past them the core `PAUSE`s the holder's reads of that session, as for a terminal |
| prompts in | one in flight and one in the slot (waiting or held) per session, 256 KiB each; a third is refused (decision 10), so the queue's high mark is one |
| permission answers in | at most 16 pending requests per session (decision 4), so at most 16 answers |
| core → agent stdin | the holder's 4 MiB input cap, never approached: the core writes one prompt at a time and small answers |
| agent stderr | a fixed 16 KiB ring in the holder |
| one agent line | 8 MiB (`MAX_LINE`), then dropped to its end with a notice |
| the log | 16 MiB encoded (decision 14) |

### 13. A broken or hostile agent is confined (`process R3`, `railway R5`)

- Every call into `step` for a session runs under `catch_unwind` (as the emulator's feed does in `screen.rs`): a panic, or a `FoldError` from applying its own output, marks that session's ACP handling broken (`AgentState::Failed { "ACP handling failed: …" }`), refuses its prompts, and leaves the agent running until the session is closed. Other sessions and the core carry on. `BRINDLE_TEST_ACP_PANIC=<marker>` (debug builds, `server/fault.rs`, listed by `--list-fault-switches`) injects it.
- Bytes are never trusted: `Lines` bounds a line at 8 MiB; a line that isn't JSON, isn't JSON-RPC 2.0, or has params of the wrong shape is skipped and counted (one notice per run of bad lines, with the count); invalid UTF-8 is replaced. Request ids from the agent are kept as JSON values only if under 1 KiB, else the request is answered with `-32600`.
- What an agent can make the core do is bounded: it can't make it read files or run commands (no `fs`, no `terminal`), every permission answer comes from a person, pending requests are capped, and the log is bounded.
- Text is cleaned of control characters before it enters the log (decision 8).

### 14. The log's bound (confirmed by the user)

The checkpoint is the whole state, and a holder accepts at most `MAX_CHECKPOINT_LEN` (64 MiB). The log is capped at 16 MiB encoded (`LOG_LIMIT`); past it the oldest entries are dropped whole and a `Trimmed` event records through which id and how many, so a client's fold drops the same entries and a replay starts with the marker. The cap leaves room in 64 MiB for an 8 MiB partial line, a 256 KiB queued prompt and `Rpc`, and keeps the checkpoint cost bounded: every checkpoint copies the whole log, so at 16 MiB a checkpoint per 512 KiB of output is a 32× copy, the most this design accepts. Tool call content isn't stored (tool rows are one line), so 16 MiB is a long conversation. Paging is `agents-log-paging`: an attach that sends a page of the newest entries and fetches older ones on demand, and checkpoints that append segments instead of rewriting the log, both additive (a later protocol version, a new checkpoint format version).

### 15. The session list and the status mapping (confirmed by the user)

`SessionInfo.agent = { activity, name }`, where `activity` is `Transcript::activity()` mapped onto `client::AgentActivity` (decision 6), and the names are program status's (`idle`, `working`, `blocked`, `done`, `error`) so `agents-profiles` maps them one-to-one onto `brindle_term_core::status::State` and the tab dots and notifications work unchanged:

| Transcript | Activity |
| --- | --- |
| agent `Starting`, turn `Working` or `Cancelling` | `working` |
| a permission request pending (turn `Blocked`) | `blocked` |
| turn `Ended { EndTurn }` | `done` |
| turn `Idle` or `Ended { Cancelled }` | `idle` |
| turn `Ended` with `MaxTokens`, `MaxTurnRequests`, `Refusal` or `Other`, turn `Failed`, agent `LoginRequired`, `Failed` or `Exited` | `error` |

The queue slot doesn't change the activity, waiting or held: the turn it waits for, or the ending that held it, does. Clearing `done` and `error` once the user types into the zone is the zone's view (`agents-profiles`); the server's activity is unchanged by it. `brindle-remote list` shows `agent: <activity>` in the title column, and `--json` the `agent` object. Agent sessions have no terminal title and get no `PROCESS` messages.

### 16. The fake ACP agent is a separate test binary (confirmed by the user)

So the agent cases run against release builds and packaged archives too, the fake ACP agent is not part of the helper.

- **Where it lives and how cargo builds it:** a second `[[bin]]` of `brindle-remote-agent`, `name = "brindle-fake-acp-agent"`, `path = "src/fake_acp/main.rs"`, a thin loop over the pure script parser in `src/fake_acp/script.rs` (in `PURE_MODULES`). Being a bin of a default member, every `cargo build` and `cargo build --release` puts it in `target/<profile>/` (and the README's musl build of the helper's package puts a static one beside the static helper). It names no dependency beyond the package's list (it reads and writes JSON lines through `serde_json`). `remote-agent/build.rs` switches from `embed_resource::compile` to `compile_for(…, ["brindle-remote"], …)`, so the helper's version resource isn't linked into the test binary.
- **Never packaged:** `scripts/package-linux.sh`, `scripts/package-windows.ps1` and `scripts/winget-manifest.ps1` name their files and stay unchanged; CI's archive steps gain a listing check that each archive holds exactly its files.
- **How `remote-agent/tests/` finds it:** `env!("CARGO_BIN_EXE_brindle-fake-acp-agent")` in `tests/support/mod.rs`, beside `BIN`; cargo builds a package's binaries for its integration tests.
- **What `agents-profiles`' e2e suite needs from it:** the fake agent speaks ACP and nothing of Brindle's, so any build of it works with any Brindle and helper of this or a later release: it has no version to match. The suite finds it through `BRINDLE_FAKE_ACP_AGENT`, else beside its own build (`target/<profile>/brindle-fake-acp-agent[.exe]`, next to `CARGO_BIN_EXE_brindle`), never beside a `BRINDLE_E2E_BIN` (an archive doesn't carry it). When a selected case needs it, the suite checks it as `check_helper` checks the helper: missing, or older than `remote-agent/src/fake_acp/`, stops the run with the build command (`cargo build -p brindle-remote-agent --bin brindle-fake-acp-agent`, `--release` to match the suite's build). The archive jobs (`e2e-archive`, `windows-e2e-archive`) build it, or set the variable, before running agent cases. A `remote-*` agent case needs the static musl build copied into its container.
- **The script:** lines of steps: `expect <method>` (read until a message with that method, recording it to a file named in the script), `send <json>`, `send-raw <text>` (malformed lines, unknown variants), `respond <json-result>` (to the last request read), `respond-error <code> <message> [<data-json>]` (to the last request read; the optional JSON is the error's `data`, so a `-32022` can carry `{ requested, supported }`), `stderr <text>`, `wait-file <path>` (a gate the test creates, never a sleep), `exit <code>`.

*Rejected:* a debug-only `fake-acp-agent` role of the helper with a `--list-test-roles` probe (round 1), which kept agent cases off release builds and archives.

### 17. Login, exits and closing (confirmed by the user)

- **Login:** `-32000` (Authentication required) gives `AgentState::LoginRequired { message }` with the agent's message, whichever request it answers. ACP allows it on any request but `initialize` (v1 names `session/new`; v2 adds that a prompt rejected before its message was inserted gets it, e.g. after a logout elsewhere). The core's requests after `initialize` are `session/new` and `session/prompt`, so:
  - from `session/new`: the session never becomes `Ready`;
  - from `session/prompt`: the turn ends `Failed { message }` (the user entry stays in the log; the core never resubmits it, as ACP warns that resubmitting can repeat tool side effects), a prompt waiting in the slot is held (`Held { LoginRequired }`), then the agent becomes `LoginRequired`, in that order.
  Either way the core sends no `authenticate`, the listing shows `error`, every later prompt and `AGENT_RESEND` is refused saying the agent needs a login through its own command line (`AGENT_UNQUEUE` still discards a held prompt), and the agent is left running until the session is closed: nothing can be sent without starting the agent again, which is opening the profile again (`agents-profiles`). The core never leaves `LoginRequired` on its own. In-zone login, and what happens after a login to a prompt that got `-32000`, are `agents-login`'s.
- **Exit:** an exit gives `AgentState::Exited { code, stderr }` from `EXITED` and `STDERR_TAIL` (lossy UTF-8, cleaned), then the session's `EXIT`; except after a version mismatch (decision 4), where the agent stays `Failed` with the version message, its stderr tail becomes a `Notice`, and the `EXIT` follows. No restart; `session/load`/`session/resume` recovery is `agents-session-recovery`.
- **Closing:** a `CLOSE` ends an agent session as it ends a terminal's, at once (it hangs the agent up and the pending permissions with it); a `KILL` or `STOP` from the management commands never asks. Asking before closing an agent that is mid-turn is Brindle's (`agents-profiles`), decided from the activity the server lists and sends.

## Boundary with `agents-profiles`

This change delivers, and `agents-profiles` consumes:
- **Client protocol 4:** `SPAWN_AGENT`, `AGENT_PROMPT`, `AGENT_UNQUEUE` (19: discard the slot's prompt, waiting or held), `AGENT_RESEND` (20: send a held prompt again), `AGENT_CANCEL`, `AGENT_PERMISSION`, `TRANSCRIPT` (events as JSON objects), attach and replay through `ATTACH`/`REPLAY_*`, `OK`/`ERROR` replies keyed by `req`, and the refusal texts: the agent isn't ready, a prompt is already queued, the prompt is over 256 KiB, the agent needs a login, ACP handling failed, nothing is queued, no prompt is held, the request was already answered or doesn't offer that option, the session needs client protocol 4, an agent session has no size (`RESIZE`), and the spawn's errors (the program and the OS error; on Windows, the batch-file hint with its `cmd /c` form).
- **The sixth crate `brindle-transcript`** (directory `transcript/`, `serde` and `serde_json` only, no feature), exporting at its root `brindle_transcript::{Event, Entry, EntryId, TurnState, StopReason, AgentState, Slot, HoldReason, PermissionOption, PermissionOutcome, ToolKind, ToolStatus, NoticeLevel, Activity, Transcript, FoldError, DecodeError}`, `Event::from_value`, `Transcript::{apply, replay, activity, slot}`. Brindle never depends on the schema crate or the reducer, which are the helper's (`remote-agent/src/acp/`). Brindle's chat model is a `Transcript` plus its view state; the queue row is `Transcript::slot()`, set by `Event::Queue { slot }` (live and in every replay while not `Empty`). There is no `Queued`, `Unqueued` or `Returned` event, and nothing from the slot ever goes into the prompt input.
- **The queue's rules (decision 10):** `Slot::{Empty, Waiting { text }, Held { text, reason: HoldReason }}`, `HoldReason::{Cancelled, Failed, Stopped { stop }, LoginRequired, AgentStopped, OutputLost}`. A prompt during a turn waits; it is sent only after `Ended { EndTurn }`; every other ending, a cancel, the agent stopping and a truncated restart hold it. `AGENT_RESEND` sends a held prompt (queued again if a turn runs; refused if the agent isn't ready); `AGENT_UNQUEUE` discards a waiting or held one. A prompt while one waits or is held and a turn runs is refused ("a prompt is already queued"); with no turn running, a new prompt is sent and a held one stays held. The slot survives detach, Brindle's crash or quit, core restarts and upgrades; an agent exit ends the session, so the server keeps it no longer than that. It never changes the listed activity.
- **Event forms frozen for protocol 4:** no new event kind or enum value within protocol 4, only optional fields, which `from_value` ignores when unknown; so from a correct core every event decodes, and a decode error is a broken session.
- **`SessionInfo.agent`** (`activity: client::AgentActivity`, `name`), the activity-to-status mapping of decision 15, and, for closing with confirmation, nothing more: Brindle asks when the zone's activity is `working` or `blocked`.
- **Listing and layouts as for terminals** (decision 9): `SessionInfo.layout` kept from `DETACH`/`SET_LAYOUT` and dropped on attach; `SessionInfo.cwd` the absolute directory `session/new` was given; size 0×0, `ATTACH`'s size ignored, `RESIZE` refused.
- **A later attach replays the whole log** (decision 7); the reaction to a `FoldError` is the client's.
- **The agent's states and what ends a session** (decisions 4, 13, 17): `LoginRequired { message }` from `-32000` on `session/new` or on a prompt (then the turn is `Failed`, a waiting prompt `Held { LoginRequired }`); the session stays, nothing can be sent, a held prompt can be discarded. `Failed { message }` from the core's own ACP fault: the session stays, likewise. `Failed { message }` from a version mismatch (`-32022`, or another `protocolVersion`): the agent is hung up and the session ends, with no `Exited` after it, so the last agent state a client has is that `Failed` and its message (plus a stderr `Notice`), then `EXIT`. `Exited { code, stderr }` otherwise, then `EXIT`.
- **The fake ACP agent** as the test binary `brindle-fake-acp-agent` (decision 16): built into `target/<profile>/` by `cargo build`, never packaged, found by the e2e suite through `BRINDLE_FAKE_ACP_AGENT` or beside its own build, checked for freshness against `remote-agent/src/fake_acp/`, and built by the archive jobs before agent cases. Its script steps are decision 16's list.
- **Brindle's offer pinned at 1–3** (`BRINDLE_PROTOCOLS`) with a logged-and-dropped `ToClient::Transcript` arm.

Left to `agents-profiles`: raising Brindle's offer to 4 and its client code for it (`Session` variants for agent sessions, the reader's transcript sink calling `Event::from_value` and `apply`), the profile kind and its config docs (an older Brindle rejecting such a config is that change's release note), `brindle agent …`, `ZoneContent::Agent` and every `as_terminal_view` caller, the chat model and components, the queued prompt's row, permission and tool rows, closing with confirmation (zone and Close Tab), mapping `Activity` onto `status::State` and notifications (`notifications.agent`), the "seen" rule for `done` and `error`, restore and sweep of agent sessions by kind and layout, the settings dialog, the dump lines, the e2e harness's fake-agent lookup and freshness check and the agent cases, the archive jobs' build step, README, `CLAUDE.md`'s manual checks.

## Risks / Trade-offs

- [The schema crate rejects a whole message when an agent is newer than the pin] → the two-step decode (envelope first, params per method) confines it to one skipped update; the spike measures it; the hand subset with `#[serde(other)]` is the fallback.
- [The schema crate's dependencies clash with the seeded `Cargo.lock`] → spike check 3; a clash means the hand subset, never a `cargo update`.
- [ACP decoding creeping out of `remote-agent/src/acp/` into the core's shell] → `only_the_acp_modules_name_the_schema_crate` and the modules' place in `PURE_MODULES`.
- [A later protocol needs a new event kind] → a new client protocol version, by decision 6's rule; the core then says what a protocol 4 client gets instead.
- [Whole-log checkpoints get expensive as conversations grow] → `LOG_LIMIT` 16 MiB caps the copy at 32× output; `agents-log-paging` is the next step.
- [A truncated tail loses a turn's end] → the turn is marked failed with the reason, a waiting prompt is held, the agent keeps running; truncation needs 1 MiB of agent stdout with no core, which a supervisor restart (3 s) makes rare.
- [The prompt/checkpoint write window] → one flush; cancel resolves the stuck turn (decision 11).
- [A permission request waits forever with nobody attached] → by design (user decision 1); the session is listed `blocked`, which `agents-profiles` turns into a status dot and a notification.
- [A held prompt lost when the agent exits] → the session ends with the exit; an attached client keeps what it shows, and `agents-session-recovery` is where the server would keep it.
- [A held prompt forgotten in its row] → it never blocks a new prompt with no turn running, and costs at most 256 KiB in the log's checkpoint.
- [The job object can't be created inside a job that forbids nesting or breakaway] → the spawn fails with `SPAWN_FAILED` naming the cause rather than running an agent whose tree can't be ended; the real-process tests run inside CI's job to catch it.
- [The fake agent binary in `target/` is mistaken for a shipped program] → the packaging scripts name their files, and the archive listing check fails on an extra one.

## Migration Plan

Additive throughout. A newer helper upgrades the running core (`process R12`), whose old holders drain; new sessions go to holders of the new build, which speak H1 3. Older Brindles keep working on protocols 1–3 and never see agent sessions. The sixth crate is internal: release-please bumps it with the others, and nothing ships it separately. Rollback: an older core adopting a newer holder sees its terminal sessions and not its agent sessions, which keep running until a core of the newer build comes back (decision 2). No config or state file changes in this change.

## Open Questions

None. Every question from the brief, and every one this design raised (where the reducer lives; a queued prompt after a failed turn; which endings are normal; a new prompt while one is held), is decided and written above as a decision. The last of them, for the record:

1. **A new prompt while one is held** (decision 10), confirmed by the user: with no turn running it is sent as usual and the held prompt stays held; while a turn runs it is refused because the slot is taken. The alternatives considered and rejected were refusing it until the held prompt is sent or discarded, or letting it replace the held prompt.
