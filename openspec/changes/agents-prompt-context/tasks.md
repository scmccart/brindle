# Tasks

Before starting, check that the changes this one builds on are implemented and archived, and that the user has settled design.md's open decisions:
- `agents-profiles` (and through it `agents-sessions`): `AGENT_PROMPT`, `Entry::User`, `Slot`, the reducer's prompt and queue handling, `src/agent/model.rs`, `brindle-ui`'s `PromptInput`, the dump's agent lines, the fake ACP agent.
- `agents-markdown`: `CodeBlock`. `agents-tool-content`: `acp/forms.rs`, `acp/dialect.rs`, `acp/v1/`, `Request`.
- When stacked on `agents-commands`: option commands and completion read the text only and leave the draft alone.
- Client protocol 4 is still unreleased. If a release has shipped it, stop and raise design decision 3.

## 1. Captures and shared contracts

- [ ] 1.1 Capture each real ACP v1 agent we can run being sent a prompt with a `resource_link` to a text file outside its working directory, in a directory private to the user, once as its session is today and, for the agents that advertise `sessionCapabilities.additionalDirectories`, once with that directory named at `session/new`; add the logs to `remote-agent/tests/fixtures/acp/`; write down in design.md's "As built" whether each agent read the file, asked permission, or refused, and whether naming the directory changed that. Then apply design decision 6: name the session's attachment directory at `session/new` for the agents it helps.
- [ ] 1.2 Add to `brindle-transcript`: `Attachment` (`Link`, and `Text` as a reference: id, name, size, cut) and `Cut`; `attachments` on `Entry::User` and on `Slot::Waiting` and `Slot::Held` (defaulted, omitted when empty); `MAX_ATTACHMENTS`, `ATTACHMENT_TEXT`; verify with unit tests that each form round-trips, `fold(replay(t)) == t` with attachments in entries and in the slot, the forms in `fuzz.rs`, and that every existing `tests/fixtures/v4/` file still passes unedited.
- [ ] 1.3 Write the new encodings as new files in `transcript/tests/fixtures/v4/` (a user entry with a link and a kept text, whole and cut; a slot with attachments); verify they are written once and read back, and no existing fixture changed.
- [ ] 1.4 Add to `brindle-remote-protocol`: `AGENT_ATTACH`, `AGENT_READ_ATTACHMENT` and `ATTACHMENT` (the next free numbers at the stack's tip), and `AGENT_PROMPT`'s optional `attachments` (omitted when empty); verify with fixtures for each message and for a prompt with each kind and with none (the old fixture unchanged), a 256 KiB text of line breaks fitting one frame in `AGENT_ATTACH` and in `ATTACHMENT`, decoding errors, and fuzzing.
- [ ] 1.5 Add to `acp/forms.rs`: `PromptSupport`, `PromptPart`, `Request::Prompt`'s parts; give `Dialect::initialize_result` the support and `Dialect::encode` the parts; verify `cargo test -p brindle --test architecture` passes.
- [ ] 1.6 Add the pure `cache_dir(env, home, windows)` (`BRINDLE_CACHE_DIR`, then `XDG_CACHE_HOME`, then `~/.cache/brindle`; `%LOCALAPPDATA%\brindle\cache` on Windows) with unit tests for each rule on every platform, and add `BRINDLE_CACHE_DIR` to the e2e and perf harnesses' `isolate` (a directory per case, removed after it) and to `scripts/run-throwaway.sh` and `.ps1`; verify the harness self-tests and that a test run leaves the user's own cache directory untouched.
- [ ] 1.6a Add the store's platform items to `server/platform/{unix, windows}` under the same names: make the cache directory and a session's attachment directory private to the user with the server directory's checks (not a link, the user's own), write a text once and make it read-only, read one, remove one, remove a directory, list a directory; verify with tests on each platform for permissions, a second write to an id refused, and removal.

## 2. Pure code in the server

- [ ] 2.1 Add `server/core/attachments.rs`: `store::plan(state, event, now) -> Vec<Step>` and the bounds (`ATTACHMENT_TEXT`, `STORE_LIMIT`, `RETENTION`), with ids per session from 1 and never reused and the store's name given by the shell; verify with unit tests for an attach, a prompt taking uploads, a refused prompt, a discard, a trim, a connection closing with untaken uploads, adoption listing orphans, a missing file, the store at its limit, the session's end leaving its directory, and the sweep (a directory a week past its session removed, a younger one kept, a live session's old directory kept); add it to `PURE_MODULES` and `fcis`'s table.
- [ ] 2.2 Add `acp/uri.rs`: `file(path, windows) -> String`; verify with unit tests for a Unix path, a Windows drive path, a UNC path, spaces, `%`, `#`, `?`, non-ASCII and a path that is already odd, on every platform.
- [ ] 2.3 Read `promptCapabilities` in `acp/v1/decode.rs` into `PromptSupport`, and write `PromptPart`s in `acp/v1/encode.rs` (`text`, `resource_link`, `resource` with `text/plain`); verify with unit tests over every captured `initialize` answer and pinning each line written.
- [ ] 2.4 Extend the reducer's prompt path: the count check before anything is logged or queued; cleaning; the user entry and the slot carrying references through waiting, held, sent again and discarded; at send time, resolving each link's path and building the parts in order, each kept text embedded (at most `EMBED_LIMIT`, with `PromptSupport.embedded`) or linked by its file's URI, the cut note, and a missing text left out with its notice; verify with unit tests for each, including `~` and relative paths, a 64 KiB and a 64 KiB + 1 text, an agent without embedded context, and a queued prompt sent after the turn ends.
- [ ] 2.5 Extend `acp/checkpoint.rs` with the references, the next attachment id and `PromptSupport`, format 1 amended and holding no attachment text; verify a held prompt with attachments round-trips, and a checkpoint from before this change decodes with none.

## 3. Server roles

- [ ] 3.1 Handle `AGENT_ATTACH`, `AGENT_PROMPT`'s attachments and `AGENT_READ_ATTACHMENT` in the core: gather, call `store::plan` and the reducer, run the file steps through `server/platform/`, read the kept texts a prompt needs before the reducer builds its parts, answer `OK`, `ATTACHMENT` or `ERROR` with the refusal's text; verify with real-role tests through the fake ACP agent: a small text embedded and a large one as a link to a file holding it; an agent without embedded context getting a link; a read returning the text, also after a reattach; each refusal's text and no file left by a refused prompt.
- [ ] 3.2 Add the life-cycle cases: a discard and a trim remove what they should; untaken uploads go when the connection closes; a waiting prompt keeps its attachment across a core restart, and a held one across an upgrade; a text deleted from the directory is left out with its notice; a closed session's directory is still there, and a core started with a store directory whose files are 8 days old (their times set by the test) and no session removes it.
- [ ] 3.3 Check the file's protection on Linux and on Windows in a real-role test (the directory the user's alone, the file not writable), and that every real-role test and case runs with its own `BRINDLE_CACHE_DIR`.

## 4. Brindle's model

- [ ] 4.1 Extend `src/agent/shared.rs`, `transcript.rs` and the client for the three messages and the references; verify with unit tests folding a user entry with attachments, a replay, and each message's encoding.
- [ ] 4.2 Add to `src/agent/model.rs`: the draft, `attach` with its refusal, `cut_text`, `send_plan`, `backspace_at_start`, clearing, and keeping text and draft on any refusal in the sequence; verify with unit tests for each, including the count at its edge, a cut at a line break and inside a multi-byte character, the plan's order, a refusal of the second attach, and backspace with a selection or mid-text leaving the draft alone.
- [ ] 4.3 Add the pure `drop_plan(paths, host)` and `dropped_text(name, bytes)`; verify with unit tests for a local, an SSH and a container host, a directory, invalid UTF-8, an empty file, and a file one byte over the limit.
- [ ] 4.4 Add the conversation's attachment rows: a link's path; a text's row with `TextState::{Closed, Loading, Shown, Gone}`, the read request on opening, the text through `CodeBlock` with its cut line, and the text dropped on closing; the queue row's count; verify with unit tests for each transition, an answer arriving after the row was closed, and a replayed message.
- [ ] 4.5 Add `target_plan(agent_zones)` in `src/workspace.rs` (none, one, ask; most recently focused first); verify with unit tests for each and for zones in several tabs.
- [ ] 4.6 Extend the dump (`describe`, `wait_text`) with `attachments=`, `--- attached`, `--- draft` and the queued count; verify with unit tests for each form, each row state and escaping.

## 5. brindle-ui

- [ ] 5.1 Add `PromptInput::set_attachments` and the row (icon, name with an ellipsis, detail, remove mark; no keyboard stop; a press on the mark asks the owner with the key); verify with component tests that `row_stops` is unchanged, a long name is cut, and the press reports its key; add it to the gallery.

## 6. The zone and the terminal

- [ ] 6.1 Wire the draft into `src/agent/view.rs`: the row from the model, backspace at the start, the remove mark, running `send_plan` on submit, refusals beside the input and on the notice line.
- [ ] 6.2 Add `agent_attach_file` ("Attach File…", a palette prompt) and `agent_clear_attachments` ("Clear Attachments") to `ACTIONS`, handled only by `AgentView`, the second listed only with attachments; verify with unit tests for the palette's listing.
- [ ] 6.3 Handle files dropped on an agent zone: `drop_plan`, links attached at once, files for a remote agent read on a background task (at most `ATTACHMENT_TEXT` + 1 bytes) and passed to `dropped_text`, each refusal on the notice line; the prompt input highlighted while files hover. Nothing is read on the UI thread (`process R13`).
- [ ] 6.4 Add `send_last_output_to_agent` to `ACTIONS`, handled by `TerminalView` (listed after a prompt mark, as Copy Last Command Output is, and taking the same text with `\n` line ends), which hands the text and its name to the workspace; the workspace runs `target_plan`, opens the list when asked, attaches, activates the tab and focuses the prompt input; verify with unit tests for the listing rule, the names (`cargo test`, `output from TITLE`), and nothing attached when the copy command would copy nothing.
- [ ] 6.5 Draw the conversation's attachment rows and the queue row's count.

## 7. End-to-end cases (fake ACP agent)

- [ ] 7.1 `terminal-to-agent-output`: a command's output in a terminal tab becomes a draft in the agent tab, named by its command line, with focus in the prompt input and nothing sent; sending it shows the user message with its attachment's reference, and the fake agent received an embedded resource.
- [ ] 7.2 `terminal-to-agent-choose`: two agent tabs, the list in most-recently-focused order, choosing the second; and no agent tab giving the notice line.
- [ ] 7.3 `agent-attach-file`: `agent_attach_file` with a relative path; the fake agent receives the resolved `file://` link; escape attaches nothing; backspace at the start and `agent_clear_attachments` remove.
- [ ] 7.4 `agent-attach-bounds`: a ninth attachment refused on the notice line; output over 256 KiB attached cut at its start, and sent as a link to a kept file; blank text with an attachment sends nothing.
- [ ] 7.5 `agent-attach-queue`: a prompt with an attachment sent during a turn waits with its count, and the agent receives both when the turn ends; a held one sent again keeps it; a discarded one leaves no file.
- [ ] 7.6 `agent-attach-restore`: quit with a waiting prompt that has an attachment and a sent one in the conversation; the relaunched zone shows both references, and the session's attachment directory, under the case's `BRINDLE_CACHE_DIR`, still holds their texts.
- [ ] 7.7 Run the cases on Windows (one `windows-agent-*` case with a drive-letter link and a kept text) and in the archive jobs.

## 8. Docs and checks

- [ ] 8.1 Update `docs/code-map.md` (the terminal server's directories and the store), `README.md` (the Agent tabs section, the terminal action, where attached text is kept and when it goes), and `CLAUDE.md`'s dump description, action notes and manual checks.
- [ ] 8.2 Tell `agents-acp-v2` what it inherits (its v2 module writes the same three prompt parts and reads the same capability) and `agents-prompt-images` what it builds on (the store, the upload number and `Attachment`).
- [ ] 8.3 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate agents-prompt-context --strict`.
- [ ] 8.4 Manual checks for the user, with `scripts/run-throwaway.sh` and real agent profiles: dropping a file on a local agent's zone (a link) and on an SSH or container agent's zone (text kept, a binary refused), on Linux and on Windows; a pointer press on an item's remove mark, and on a text attachment's row in the conversation to open and close it; a real agent answering about a linked file, about a small attached output, and about one over 64 KiB that it must read from the kept file.
