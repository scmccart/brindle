# Proposal

## Why

The 2026-11-26 agent code review (`docs/architecture/audits/2026-11-26-agents.md`, change 6 of `2026-11-26-agents-remediation.md`) found that an agent session's checkpoint and the paths around it cost more than `agents-log-paging` set out to pay. All are performance faults on the terminal server core's one loop thread, which also serves every terminal session.

- **B-3, C-2 (medium):** each format 2 segment rewrites the whole non-log state. Its `tools` map holds every tool call the log keeps, about 40 bytes each, and a permission-gated tool call brings two segments. At 5,000 tool calls each segment is about 200 KiB, and the segments outgrow the base every few calls. The session then writes roughly 80·T² bytes, about 2 GB at T = 5,000, for a log that grew by about 2 MB. This breaks `agents-log-paging` decision 9's claim of "a small constant" per byte of log growth, which counted only the events. `methods` (up to 256 KiB) and `outputs` (up to 64 open outputs, each with a line of up to 64 KiB) are written whole in every segment too.
- **B-4, C-2 (low):** a line that is still arriving is checkpointed every 512 KiB of stdout, and each segment carries the whole partial line so far. That is about 68 MiB written for one 8 MiB line, plus two or three whole re-encodes of the log.
- **`c.md` observation 6:** a whole checkpoint makes about six transient copies of the log: `replay()`'s cloned events, a `Value` tree, `to_string`, the record buffer, the chunk `Vec`s and the frames. That is about 100 MiB for a 16 MiB log, at every trim, compaction, restart, adoption and step-down. Each `TRANSCRIPT` event is serialized twice, once to measure it in `pack_events` and once to encode it.
- **A-4 (low):** once a command's output passes 64 KiB, each output delta memmoves the kept 64 KiB, in the core and again in every client. That is about 2 GiB copied per side for 1 MiB of output arriving in 32-byte pieces.
- **A-5 (low):** a tool call's diff is recomputed (Myers, plus every kept line allocated) for each message that repeats it. Claude's adapter sends the same diff in the update, in the permission request and in the completing update. A permission's whole `toolCall` is also cloned to be applied.
- **B-5 (low):** the v1 decoder deep-clones whole updates, file texts and images included, through `&Value` signatures, only to drop most of what it cloned.

Format 2 and client protocol 4 have not shipped (v0.3.0 predates both), so the fix amends format 2 in place (choice C4 of the plan).

## What Changes

- **Segments carry deltas** (B-3, B-4). This amends checkpoint format 2 in place; the framing (header, `B`/`S` records, sequence numbers) is unchanged.
  - `tools` is no longer written. The reducer records where the running agent's tool entries start (`tool_epoch`), and the decoder rebuilds the index from the log.
  - The partial line is written as the bytes added since the previous record, with their offset (`lines.from`).
  - `methods` is written only when it changed, and `outputs` as a patch of the terminals that changed or closed.
  - Every other field stays whole in every record: each is bounded and small, and the design lists their sizes.
  - A checkpoint written before this change (format 1, or format 2's earlier shape) is not read: its session is adopted as not restored. Only development builds wrote one, and the one-time loss of the conversations running at that reinstall was accepted on 2026-11-27.
- **The cost bound is a test.** A pinned synthetic conversation of 5,000 permission-gated tool calls and one 8 MiB line arriving in 64 KiB reads asserts that the bytes written stay within 4× what the session added, plus a fixed allowance per record and one base per trim. It also asserts that no segment grows with the number of tool calls.
- **One buffer per checkpoint** (observation 6). The log and a segment's events are serialized straight into the record buffer, the whole buffer refusing past the limit as it is written. Holder frames are queued from slices of that buffer. A transcript event is serialized once, and `brindle-remote-protocol` packs the encoded objects without knowing what they are.
- **Output appends in amortized O(delta)** (A-4). An output's text keeps a start offset and compacts once the dropped prefix passes the limit. Its equality, encoding, `encoded_len` and fold results are unchanged.
- **Repeated diffs are reused** (A-5). The session keeps a digest of each diff item's path, old text and new text; the checkpoint and the log do not. An item whose digest matches is taken from the entry instead of being diffed again. A permission's tool call is moved, not cloned.
- **`Dialect` takes owned values** (B-5). The `Dialect` decoders that clone take `params` or `result` by value, and v1 moves fields out of them. `ports R14` is unchanged.
- **Numbers before and after:** `agent-attach-long`, `agent-tool-output` (with a new run of 32-byte deltas) and a new `agent-tool-heavy` perf case report them (`testing R14`).

No wire message changes, and no protocol version moves: the H1 frames and the `TRANSCRIPT`/`AGENT_PAGE` frames are byte-for-byte what they were. A user sees nothing new. Within the server, checkpoints cost what they add.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: "Checkpoints append what changed" gains scenarios for a tool-heavy conversation, a long line arriving in pieces, and a core killed mid-line. Its text says that a segment's cost doesn't grow with the conversation. The other requirements hold as written: "Agent sessions survive core restarts and upgrades", "The log is bounded", "Tool calls keep their content" and "Transcript event forms are fixed for each protocol version".

## Impact

- **Code:**
  - `remote-agent/src/acp/checkpoint.rs`: the record shapes, `Written`, `encode`, `encode_segment`, `decode`, and a new `Unreadable::PartialGap`.
  - `remote-agent/src/acp/reduce.rs`: `tool_epoch`, the diff memo, and the permission's call taken by value.
  - `remote-agent/src/acp/dialect.rs` and `v1/decode.rs`: owned arguments.
  - `remote-agent/src/server/core/policy.rs`: `Stored::Known` carries `Written`.
  - `remote-agent/src/server/core/agents.rs` and `sessions.rs`: encoding events once, and queueing sliced frames.
  - `remote-agent/src/server/core/holders.rs` and `server/conn.rs`: `Conn::queue_sliced`.
  - `remote-protocol/src/holder.rs`: `checkpoint_frames`.
  - `remote-protocol/src/client.rs`: `JsonObject`, `transcript_frames` and `page_frames`.
  - `transcript/src/lib.rs`: `OutputText`, `Output::append`, and the borrowed `Transcript::replay_ref`.
  - `src/agent/tool.rs` and `src/agent/model.rs`: they read `Output::text` through its new type.
  - `tests/perf/agent.rs` and `tests/perf/main.rs`: the new runs and the new case.
- **Wire and files:** checkpoint format 2's state is amended (unreleased; choice C4). The H1 3 and client protocol 4 encodings are unchanged, and the existing transcripts and fixtures pin that. No fixture is edited.
- **Dependencies:** none. serde_json's `raw_value` feature is not used: the helper builds without gpui, which is what enables it in a workspace build.
- **Overlap with the sibling changes** (the plan's order is 1 to 7, so changes 3, 4 and 5 land before this one):
  - **`agents-event-bounds` (3)** rewrites `v1/decode.rs`'s `raw` with a bounded writer (B-2) and makes the fold enforce tool content bounds (B-7). B-5's owned arguments apply to whichever `raw` exists. The fold's `Output` arm is where A-4's new `append` lands.
  - **`agents-process-lifecycle` (4)** forgets the stored checkpoint after a caught panic (C-3) and adds one "agent gone" step used by `respawn`. `Written` is forgotten with `Stored`, and `tool_epoch` and the diff memo are reset in that step.
  - **`agents-module-structure` (5)** moves `checkpoint.rs` into `checkpoint/{format,state,adopt}.rs` and `transcript/src/lib.rs` into `event.rs`, `fold.rs`, `page.rs` and `output.rs`. It gives the tool index to a `ToolBook` and moves `methods` into the greeting. This change works on the moved files: `tool_epoch` and the diff memo join `ToolBook`, and `methods` stays a top-level record field wherever its type lives.
  - **`agents-absorb-cost` (7)** changes the client's copies of tool output; it reads `OutputText` as this change leaves it.
  - **Changes 1 and 2** don't touch these files.
- **Overlap with open changes:**
  - **`agents-commands` (PR #151, merges first)** adds `Commands`/`Options` events, which segments carry as they carry every event. It also adds state fields (option keys, the `Busy` mark, kept mode values; at most 32 options), which this change classifies as small and keeps whole in every record. It touches `acp/reduce.rs` and `src/agent/model.rs`, so the first task re-opens the cited sites on `main`.
  - **`agents-plans`** (proposal) keeps `dismissed` plans, up to `PLANS_LIMIT` (4 MiB), "written whole in every record". Under this change's rule (decision 5) that field must be written only when it changed. Whichever change lands second applies the rule.
  - **`agents-session-info`**, **`agents-prompt-context`** and **`agents-prompt-images`** add small state fields: a usage record, a store name, and `PromptSupport`. They stay whole. `agents-prompt-context`'s design says the checkpoint "is rewritten whole", which this change and `agents-log-paging` make out of date.
  - **`agents-client-terminals`** adds terminals an agent runs through Brindle. Their open outputs go through the `outputs` patch.
  - **`agents-acp-v2`** adds a v2 decoder. It takes the owned `Dialect` arguments this change introduces.
