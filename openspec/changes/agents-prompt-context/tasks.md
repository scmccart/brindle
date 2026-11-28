# Tasks

Before starting, check that the changes this one builds on are implemented and archived (the user's decisions are settled; design.md lists them):
- `agents-profiles` (and through it `agents-sessions`): `AGENT_PROMPT`, `Entry::User`, `Slot`, the reducer's prompt and queue handling, `src/agent/model.rs`, `brindle-ui`'s `PromptInput`, the dump's agent lines, the fake ACP agent.
- `agents-markdown`: `CodeBlock`. `agents-tool-content`: `acp/forms.rs`, `acp/dialect.rs`, `acp/v1/`, `Request`.
- When stacked on `agents-commands`: option commands and completion read the text only and leave the draft alone.
- Client protocol 4 is still unreleased. If a release has shipped it, stop and raise design decision 3.
- Refreshed on 2026-11-26 for the built stack (`agents-session-recovery`, `agents-login`, `agents-session-title`, `agents-zone-attention`, `agents-zone-theme-scope`, `agents-log-paging`, `remote-login-environment`): design decisions 3, 4 and 13 hold what they change, and the tasks below marked "(refresh)" carry it. The function and file names in the other tasks are from before those changes; check each against the code. Where a task names `session/new`, a continue's `session/load` or `session/resume` starts the session too.

## 1. Captures and shared contracts

- [x] 1.1 Capture each real ACP v1 agent we can run being sent a prompt with a `resource_link` to a text file outside its working directory, in a directory private to the user, once as its session is today and, for the agents that advertise `sessionCapabilities.additionalDirectories`, once with that directory named at `session/new`; add the logs to `remote-agent/tests/fixtures/acp/`; write down in design.md's "As built" whether each agent read the file, asked permission, or refused, and whether naming the directory changed that. Then apply design decision 6: name the session's attachment directory at `session/new` for the agents it helps.
  Done: 9bb0b03b (captured on Windows with the user: Claude, Codex, Copilot and Antigravity, 11 logs and `scripts/capture-attach-link.ps1` with `scripts/acp-log-wrapper.py`, fixed in 8954d365), read by the decode tests in 4c3b41ea. Every agent read the kept file without asking, and naming the directory changed nothing (As built flag 2); it is still named, at `session/new`, `session/load` and `session/resume`, for every agent that advertises `additionalDirectories` (decision D-cap-1, `reduce/lifecycle.rs` `directories`). The captures also found the silent Send Last Command Output to Agent fixed by flag 25. Names that moved since the change was written (flag 3): `reduce.rs` is `reduce/`, `checkpoint.rs` is `checkpoint/`, the transcript lib is `event.rs`/`fold.rs`/`page.rs`/`output.rs`, the client's `model.rs` is `model/`, and `store::plan` lives in `server/core/attachments.rs`.
- [x] 1.2 Add to `brindle-transcript`: `Attachment` (`Link`, and `Text` as a reference: id, name, size); `attachments` on `Entry::User` and on `Slot::Waiting` and `Slot::Held` (defaulted, omitted when empty); `MAX_ATTACHMENTS`, `ATTACHMENT_TEXT`; verify with unit tests that each form round-trips, `fold(replay(t)) == t` with attachments in entries and in the slot, the forms in `fuzz.rs`, and that every existing `tests/fixtures/v4/` file still passes unedited.
  Done: 63f4ba29, with `PROMPT_ADDED_EVENT` and `ATTACHMENTS_LEN` rows in `acp/limits.rs` so every event stays within `MAX_EVENT`.
- [x] 1.3 Write the new encodings as new files in `transcript/tests/fixtures/v4/` (a user entry with a link and a kept text; a slot with attachments); verify they are written once and read back, and no existing fixture changed.
  Done: 63f4ba29, three new fixtures; no existing one changed.
- [x] 1.4 Add to `brindle-remote-protocol`: `AGENT_ATTACH` (25), `AGENT_READ_ATTACHMENT` (26) and `ATTACHMENT` (44, from the core), the next free numbers after `agents-commands`, and `AGENT_PROMPT`'s optional `attachments` (omitted when empty); verify with fixtures for each message and for a prompt with each kind and with none (the old fixture unchanged), a 256 KiB text of line breaks fitting one frame in `AGENT_ATTACH` and in `ATTACHMENT`, decoding errors, and fuzzing.
  Done: 8f00c532, with `client-agent-attachments.hex` and `client-agent-attachments-refused.hex`; `ATTACHMENT` carries `session` (flag 4), and `SPAWN_AGENT`'s two fields landed here too (flag 5).
- [x] 1.5 Add to `acp/forms.rs`: `PromptSupport`, `PromptPart`, `Request::Prompt`'s parts; give `Dialect::initialize_result` the support and `Dialect::encode` the parts; verify `cargo test -p brindle --test architecture` passes.
  Done: 3955e478, `Initialized` also carries `directories` and the session requests their `additionalDirectories`.
- [x] 1.6 Add the pure `cache_dir(env, home, windows)` (`BRINDLE_CACHE_DIR`, then `XDG_CACHE_HOME`, then `~/.cache/brindle`; `%LOCALAPPDATA%\brindle\cache` on Windows) with unit tests for each rule on every platform, and add `BRINDLE_CACHE_DIR` to the e2e and perf harnesses' `isolate` (a directory per case, removed after it) and to `scripts/run-throwaway.sh` and `.ps1`; verify the harness self-tests and that a test run leaves the user's own cache directory untouched.
  Done: 091a383e, `cache_dir` in `server/core/attachments.rs`; the harnesses' directory is `<server dir>/cache` (flag 9), the real-role tests' `<dir>/cache`.
- [x] 1.6b Add the settings to `src/config.rs`: `[agent] attachments_dir` and `attachments_keep_days` (default 14, clamped to 0–3650 with the config line saying so), and `attachments_dir` on both kinds of `RemoteTarget`, dropped by `endpoint()` and not serialized into the session state, as `connect_at_launch` is; document them in `assets/default-config.toml`; verify with unit tests for the defaults, each value, the clamp and its message, two remotes differing only in `attachments_dir` having equal endpoints, the session state not holding the key, and that the shipped default config parses.
  Done: 15ce620b; the settings dialog's profile editor keeps a remote's `attachments_dir`, and a config save writes it back.
- [x] 1.6c Add the optional `attachments_dir` and `attachments_keep_days` to `SPAWN_AGENT` (omitted when unset), filled by `agent_spawn_request` from the profile: the remote's directory for a remote profile, `[agent] attachments_dir` for a local one, never the local one for a remote; verify with fixtures (the old one unchanged), fuzzing, and unit tests for a local profile, a remote with and without the key, and a container.
  Done: fields and fixtures in 8f00c532 (flag 5), filled by `Keeping::of` in `src/agent/spawn.rs` in 15ce620b; the keep time is always sent (flag 10).
- [x] 1.6a Add the store's platform items to `server/platform/{unix, windows}` under the same names: make the cache directory and a session's attachment directory private to the user with the server directory's checks (not a link, the user's own), write a text once and make it read-only, read one, remove one, remove a directory, list a directory; verify with tests on each platform for permissions, a second write to an id refused, and removal.
  Done: 091a383e, `server/platform/{unix,windows}/store.rs`, the listing type neutral in `platform/mod.rs` (flags 11, 13).

## 2. Pure code in the server

- [x] 2.1 Add `server/core/attachments.rs`: `store::plan(state, event, now) -> Vec<Step>` and the bounds (`ATTACHMENT_TEXT`, `STORE_LIMIT`, the default 14 days), with ids per session from 1 and never reused, the store's name given by the shell, and the session's directory and keep time from its spawn; verify with unit tests for an attach, a prompt taking uploads, a refused prompt, a discard, a trim, a connection closing with untaken uploads, adoption listing orphans, a missing file, the store at its limit, a directory that isn't absolute or isn't private refusing attaches, the session's end writing its marker (and removing the directory at once with 0 days), and the sweep (a directory past its marker's time removed, one before it kept, one with no marker judged by its newest file and 14 days, a live session's old directory kept, and which directories are swept at a core's start and at a session's start and end); add it to `PURE_MODULES` and `fcis`'s table.
  Done: 091a383e, with the uploads' bookkeeping (`Uploads`) there too and the sweep its own function (flag 12).
- [x] 2.2 Add `acp/uri.rs`: `file(path, windows) -> String`; verify with unit tests for a Unix path, a Windows drive path, a UNC path, spaces, `%`, `#`, `?`, non-ASCII and a path that is already odd, on every platform.
  Done: 3955e478, in `PURE_MODULES` and `fcis`'s table.
- [x] 2.3 Read `promptCapabilities` in `acp/v1/decode.rs` into `PromptSupport`, and write `PromptPart`s in `acp/v1/encode.rs` (`text`, `resource_link`, `resource` with `text/plain`); verify with unit tests over every captured `initialize` answer and pinning each line written.
  Done: 3955e478; v1's `resource` has no name, so an embedded text's name isn't sent (flag 7).
- [x] 2.4 Extend the reducer's prompt path: the count check before anything is logged or queued; cleaning; the user entry and the slot carrying references through waiting, held, sent again and discarded; at send time, resolving each link's path and building the parts in order, each kept text embedded (at most `EMBED_LIMIT`, with `PromptSupport.embedded`) or linked by its file's URI, and a missing text left out with its notice; verify with unit tests for each, including `~` and relative paths, a 64 KiB and a 64 KiB + 1 text, an agent without embedded context, and a queued prompt sent after the turn ends.
  Done: 3955e478, in the pure `reduce/attach.rs` (`PURE_MODULES`); a path past `MAX_PATH` is refused rather than cut.
- [x] 2.5 Extend `acp/checkpoint.rs` (format 2, amended) with the references in its entries and slot, and the store's name, the next attachment id and `PromptSupport` as state fields of every record, holding no attachment text; verify a held prompt with attachments round-trips as a base and as a base with segments, and a format 1 and a format 2 checkpoint from before this change decode with none.
  Done: 3955e478, `store` and `prompts` in `checkpoint/state.rs`, left out when unset; format 1 stays unreadable as before (`agents-log-paging`), a format 2 state from before reads as none.
- [x] 2.6 (refresh) A session whose agent stopped is still live to the store (design decision 4): verify with unit tests of `store::plan` that an agent's exit writes no marker and removes nothing, with 0 days too, that the session's record being dropped does, and that `PromptSupport` is taken again from a continued agent's `initialize`, a kept text embedded for one agent going as a link to the next.
  Done: `an_agents_exit_writes_no_marker_and_removes_nothing` (091a383e) and `reduce/attach.rs`'s `a_continued_agents_prompt_support_is_its_own`.

## 3. Server roles

- [x] 3.1 Handle `AGENT_ATTACH`, `AGENT_PROMPT`'s attachments and `AGENT_READ_ATTACHMENT` in the core: gather, call `store::plan` and the reducer, run the file steps through `server/platform/`, read the kept texts a prompt needs before the reducer builds its parts, answer `OK`, `ATTACHMENT` or `ERROR` with the refusal's text; verify with real-role tests through the fake ACP agent: a small text embedded and a large one as a link to a file holding it; an agent without embedded context getting a link; a read returning the text, also after a reattach; each refusal's text and no file left by a refused prompt.
  Done: 3e1f536b, the shell in `server/core/kept.rs` (flag 14); upload numbers are the connection's across its sessions, as the protocol says.
- [x] 3.2 Add the life-cycle cases: a discard and a trim remove what they should; untaken uploads go when the connection closes; a waiting prompt keeps its attachment across a core restart, and a held one across an upgrade; a text deleted from the directory is left out with its notice; a closed session's directory is still there with its marker, a session started with 0 days leaves none, a core started with a store directory whose marker's time has passed (written by the test) removes it, and a session started with its own attachments directory keeps its text there.
  Done: 4fc13216, in `remote-agent/tests/agents.rs` and (the upgrade) `upgrade.rs`; a detach drops its untaken uploads too.
- [x] 3.2a (refresh) Add the stopped-agent and paging cases: an agent's exit leaves the session's kept text in place and readable, with 0 days too; a held prompt with an attachment reaches the agent a Continue starts; closing that zone starts the keep time; a prompt refused for a login leaves no upload behind; and a text on an entry brought in by a fetch (`AGENT_FETCH`) can be read.
  Done: 4fc13216; with 0 days the close removes the store at once.
- [x] 3.3 Check the file's protection on Linux and on Windows in a real-role test (the directory the user's alone, the file not writable), and that every real-role test and case runs with its own `BRINDLE_CACHE_DIR`.
  Done: 4fc13216, `a_kept_text_is_the_users_alone_and_read_only` (mode 0700/0400; on Windows the user's and SYSTEM's ACL); the real-role support starts every role with `<dir>/cache`, and the harnesses' self-test checks `isolate` gives it.

## 4. Brindle's model

- [x] 4.1 Extend `src/agent/shared.rs`, `transcript.rs` and the client for the three messages and the references; verify with unit tests folding a user entry with attachments, a replay, and each message's encoding.
  Done: 80be6f37; `session.rs`'s `an_attached_text_and_its_read_go_out_and_their_answers_come_back` and `model/attach.rs`'s `a_user_message_draws_its_attachments_under_its_text` (flag 15).
- [x] 4.2 Add to `src/agent/model.rs`: the draft, `attach` with its refusals (`TooMany`, `TooLong { bytes, limit }`, never shortening), `send_plan`, `backspace_at_start`, clearing, and keeping text and draft on any refusal in the sequence; verify with unit tests for each, including the count at its edge, a text of exactly the limit accepted and one byte over refused with both numbers in its message, the plan's order, a refusal of the second attach, and backspace with a selection or mid-text leaving the draft alone.
  Done: 80be6f37, in `src/agent/model/attach.rs` (flag 16); `PromptSent` records the attachments and `taken` compares them.
- [x] 4.3 Add the pure `drop_plan(paths, host)` and `dropped_text(name, bytes)`; verify with unit tests for a local, an SSH and a container host, a directory, invalid UTF-8, an empty file, and a file one byte over the limit.
  Done: 80be6f37; `drop_plan(paths, local)` takes whether the agent runs here, since SSH and container hosts read alike, and `dropped_text` takes the file's size beside its first bytes.
- [x] 4.4 Add the conversation's attachment rows: a link's path; a text's row with `TextState::{Closed, Loading, Shown, Gone}`, the read request on opening, the text through `CodeBlock`, and the text dropped on closing; the queue row's count; verify with unit tests for each transition, an answer arriving after the row was closed, and a replayed message.
  Done: 80be6f37; each row a `BlockBody::Attached` part of the message's bubble (flag 17).
- [x] 4.5 Add `target_plan(agent_zones)` in `src/workspace.rs` (none, one, ask; most recently focused first); verify with unit tests for each and for zones in several tabs.
  Done: 80be6f37, in the pure `src/workspace/targets.rs` (`PURE_MODULES`), with `output_name`.
- [x] 4.6 Extend the dump (`describe`, `wait_text`) with `attachments=`, `--- attached`, `--- draft` and the queued count; verify with unit tests for each form, each row state and escaping.
  Done: 80be6f37; a gone row's line adds `why="…"`.

## 5. brindle-ui

- [x] 5.1 Add `PromptInput::set_attachments` and the row (icon, name with an ellipsis, detail, remove mark; no keyboard stop; a press on the mark asks the owner with the key); verify with component tests that `row_stops` is unchanged, a long name is cut, and the press reports its key; add it to the gallery.
  Done: c449d11d, with the gallery check `prompt-input-attachments-add-no-stops` and a listed difference in `ui/README.md`.

## 6. The zone and the terminal

- [x] 6.1 Wire the draft into `src/agent/view.rs`: the row from the model, backspace at the start, the remove mark, running `send_plan` on submit, refusals beside the input and on the notice line.
  Done: bb17064c; backspace is a capture listener on `TextInputBackspace`, and the remove mark names its item by a `DraftKey` (flag 18).
- [x] 6.2 Add `agent_attach_file` ("Attach File…", a palette prompt) and `agent_clear_attachments` ("Clear Attachments") to `ACTIONS`, handled only by `AgentView`, the second listed only with attachments; verify with unit tests for the palette's listing.
  Done: bb17064c, `PaletteFacts::agent_attachments`; the prompt is the window's (`AgentAsk::AttachFile`).
- [x] 6.3 Handle files dropped on an agent zone: `drop_plan`, links attached at once, files for a remote agent read on a background task (at most `ATTACHMENT_TEXT` + 1 bytes) and passed to `dropped_text`, each refusal on the notice line; the prompt input highlighted while files hover. Nothing is read on the UI thread (`process R13`).
  Done: bb17064c, `Agent::drop_paths`; the directory lookup is on the background task too (flag 19). A drop can't be driven, so it is a manual check (8.4).
- [x] 6.4 Add `send_last_output_to_agent` to `ACTIONS`, handled by `TerminalView` (listed after a prompt mark, as Copy Last Command Output is, and taking the same text with `\n` line ends), which hands the text and its name to the workspace; the workspace runs `target_plan`, opens the list when asked, attaches, activates the tab and focuses the prompt input; verify with unit tests for the listing rule, the names (`cargo test`, `output from TITLE`), and nothing attached when the copy command would copy nothing.
  Done: bb17064c, `Terminal::last_output_to_send` and `workspace/send_output.rs`; the list is a palette of its own (`PaletteRole::AgentTarget`, flag 20).
- [x] 6.5 Draw the conversation's attachment rows and the queue row's count.
  Done: 80be6f37 (`attached_element`, the queue row's `next · 2 attachments`).

## 7. End-to-end cases (fake ACP agent)

- [x] 7.1 `terminal-to-agent-output`: a command's output in a terminal tab becomes a draft in the agent tab, named by its command line, with focus in the prompt input and nothing sent; sending it shows the user message with its attachment's reference, and the fake agent received an embedded resource.
  Done: 791de70d, `tests/e2e/cases/attach.rs` (`output`); the kept file is read back from the URI.
- [x] 7.2 `terminal-to-agent-choose`: two agent tabs, the list in most-recently-focused order, choosing the second; and no agent tab giving the notice line.
  Done: 791de70d; it found the list ranked within each tab only, so the strip now ranks tabs by activation (flag 21).
- [x] 7.3 `agent-attach-file`: `agent_attach_file` with a relative path; the fake agent receives the resolved `file://` link; escape attaches nothing; backspace at the start and `agent_clear_attachments` remove.
  Done: 791de70d; each removal is checked by the index the next attachment takes.
- [x] 7.4 `agent-attach-bounds`: a ninth attachment refused on the notice line; output over 256 KiB refused with its size and the limit, the terminal keeping focus; output between 64 KiB and 256 KiB sent whole as a link to a kept file; blank text with an attachment sends nothing.
  Done: 791de70d; it found a refused output still showing the agent zone, now leaving focus in the terminal.
- [x] 7.5 `agent-attach-queue`: a prompt with an attachment sent during a turn waits with its count, and the agent receives both when the turn ends; a held one sent again keeps it; a discarded one leaves no file.
  Done: 791de70d; the discard is the waiting prompt's removal (`agent_unqueue`).
- [x] 7.6 `agent-attach-restore`: quit with a waiting prompt that has an attachment and a sent one in the conversation; the relaunched zone shows both references, and the session's attachment directory, under the case's `BRINDLE_CACHE_DIR`, still holds their texts.
  Done: 791de70d.
- [x] 7.6a `agent-attach-settings`: `[agent] attachments_dir` moves a local agent's kept text; `attachments_keep_days = 0` leaves no directory after the zone is closed; an out-of-range keep time shows the config line. And in `remote-agent-ssh` (Docker): a remote's `attachments_dir` is where the container keeps the text, and a remote with none uses the container user's cache directory.
  Done: 791de70d, and the Docker part as its own case, `remote-agent-attach` (ef9f0a22, flag 23), which found a remote's `attachments_dir` never sent (flag 22).
- [ ] 7.7 Run the cases on Windows (one `windows-agent-*` case with a drive-letter link and a kept text) and in the archive jobs.
  Written, not yet run: `windows-agent-attach` (a7e5b900), which the `windows-e2e` and `windows-e2e-archive` jobs' filters take, and the Linux archive job's filter now takes `terminal-to-agent-`. It wasn't compiled for Windows here (flag 24); the PR's CI runs it.

## 8. Docs and checks

- [x] 8.1 Update `docs/code-map.md` (the terminal server's directories and the store), `README.md` (the Agent tabs section, the terminal action, where attached text is kept and when it goes), and `CLAUDE.md`'s dump description, action notes and manual checks.
  Done: 509d62b7 and e79993f2 (the code map's kept attachments, the client's attachments and protocol 4's messages; README's "Attachments"; CLAUDE.md's dump lines, `BRINDLE_CACHE_DIR`, the new cases and two manual checks).
- [x] 8.2 Tell `agents-acp-v2` what it inherits (its v2 module writes the same three prompt parts and reads the same capability) and `agents-prompt-images` what it builds on (the store, the upload number and `Attachment`).
  Done: e79993f2 and the tick commit: the v2 proposal's inherited prompt parts and capability, the images design's "as built" names.
- [x] 8.3 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate agents-prompt-context --strict`.
  Done: at 4f1983cf `cargo test --workspace --locked` passes (brindle 1373 with 2 ignored, brindle-remote 773, agents 134, ui 320, the rest as before) with no known flake hit, clippy with `-D warnings` is clean, both self-tests pass and the change validates. The whole e2e suite under `scripts/with-xvfb` (in a user scope) passed at ce3580a0, 288 passed, 0 failed, 9 skipped (xclip, zsh and fish aren't installed here); the one case added since, `terminal-to-agent-nothing`, passed four times (once at `BRINDLE_E2E_SLOW=2`) beside the other `terminal-to-agent-` cases.
- [ ] 8.4 Manual checks for the user, with `scripts/run-throwaway.sh` and real agent profiles: dropping a file on a local agent's zone (a link) and on an SSH or container agent's zone (text kept, a binary refused), on Linux and on Windows; a pointer press on an item's remove mark, and on a text attachment's row in the conversation to open and close it; a real agent answering about a linked file, about a small attached output, and about one over 64 KiB that it must read from the kept file.
