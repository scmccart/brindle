# Design

## Context

See proposal.md for why. The user split images into their own change on 2026-11-25 (`agents-prompt-images`). Facts this design rests on, from the code on `feat/agents-profiles` (2026-11-25), the stacked designs, the pinned schema crate and the captured agent traffic:

- **A prompt is text today.** `AGENT_PROMPT { req, session, text }`; the reducer refuses text over `MAX_PROMPT` = 256 KiB, adds `Entry::User { text }`, and `rpc.rs` writes `session/prompt` with one `text` content block. The queue slot is `Slot::Waiting { text }` or `Held { text, reason }` and travels whole in one `Event::Queue`.
- **One frame holds at most 1 MiB** (`MAX_FRAME_PAYLOAD_SIZE`), and every event and client message must fit one. JSON escaping can double a text of line breaks. The log is capped at 16 MiB and the checkpoint, which holds the log and the slot, is rewritten whole.
- **ACP prompt content** (v1 and v2, the same kinds): `text`; `resource_link { uri, name, … }`, which "all agents MUST support"; `resource` with embedded text, allowed only when the agent's `promptCapabilities.embeddedContext` is true, and which ACP calls "preferred for including context as it avoids extra round-trips"; `image` and `audio`, each behind its own capability. Every captured agent's `initialize` answer has `embeddedContext: true`; Claude, Codex and Antigravity also advertise `sessionCapabilities.additionalDirectories`, and Copilot doesn't.
- **The server directory** is `$BRINDLE_SERVER_DIR`, else `$XDG_RUNTIME_DIR/brindle` (`%LOCALAPPDATA%\brindle\run` on Windows): private to the user, made by `server/platform/`, and on the host where the agent runs.
- **`agents-tool-content`** gives the `Dialect` seam, `acp/forms.rs` and `Request`; `agents-markdown` gives `CodeBlock`.
- **The terminal** (`shell-integration`): Copy Last Command Output takes the text from a command's output mark to its end, only once a prompt mark was received; the command line is known where the shell reports it.
- **Brindle** has one window. A tab is all terminal zones or all agent zones (`agents-profiles`). A profile's `Host` says whether its program runs on this machine, over SSH or in a container. GPUI 0.2.2 delivers files dropped on an element (`ExternalPaths`); nothing in Brindle handles a drop yet.
- **`brindle-ui`'s `PromptInput`** has the queue row above its textarea, whose buttons are keyboard stops, and (with `agents-commands`) a suggestion list and footer items that aren't.

## Goals / Non-Goals

**Goals:**
- A command's output reaches an agent in one action, named and bounded, without the clipboard.
- A file is pointed at as a file, by its path on the agent's host, and never copied when the agent can read it itself.
- Attached text never grows the conversation log, a queued prompt or a checkpoint: it is kept beside them and read when wanted.
- An attachment is part of its prompt everywhere the prompt goes: the queue, a restart, the conversation.
- Every agent is served: one that takes no embedded context gets a link to the kept text.
- The storage and the attachment form are ready for images.
- No new keyboard stop, and no change to a requirement another stacked change modifies.

**Non-Goals:** images and audio (`agents-prompt-images`); completing a path after `@`, and listing an agent host's files (`agents-file-mentions`); an action for a terminal selection (it is copied and pasted); attachments on `brindle agent -p`; a long paste becoming an attachment; directories; reading a linked file's contents from the agent's host; a prompt with attachments and blank text; sending on attach; attachment items as tab stops; showing an attachment's text in Brindle after its session has ended; a keep time per remote or per profile; editing these settings in the settings dialog. No state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Attachment` (a link, or a reference to kept text); `Entry::User`'s and `Slot`'s attachments | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| `AGENT_ATTACH`, `AGENT_READ_ATTACHMENT`, `ATTACHMENT`; `AGENT_PROMPT`'s `attachments` | wire | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| `PromptSupport`, `PromptPart`, `Request::Prompt`'s parts | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| The store's decisions: ids, bounds, what to remove and when (`store::plan`) | pure | `remote-agent/src/server/core/attachments.rs` (new) |
| Making the session's private directory, writing, reading and removing a kept text | platform seam | `remote-agent/src/server/platform/{unix, windows}` |
| Checks, bounds, embedded or link, resolving a link's path | pure | `acp/reduce.rs` |
| A path as a `file://` URI | pure | `acp/uri.rs` (new) |
| ACP v1: `promptCapabilities`; resource links and embedded resources in `session/prompt` | pure | `acp/v1/{decode, encode}.rs` |
| The draft's attachments, `attach` and its refusals, `drop_plan`, the conversation's attachment rows and their fetched text, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| Which agent zone takes a command's output (`target_plan`) | pure | `brindle`, `src/workspace.rs` |
| Reading a dropped file; fetching a kept text | shell (background task, reader thread) | `src/agent/mod.rs` |
| The terminal action, the drop handler, the attach prompt, drawing | shell (entities) | `src/terminal_view.rs`, `src/workspace.rs`, `src/agent/view.rs`, `src/actions.rs` |
| `PromptInput`'s attachment row | component | `brindle-ui`, `ui/src/components/prompt_input.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`). The core names no platform type: the store's file work is items of `server/platform/`, the same names on every platform (`ports R5`), and what they decide is the pure `store::plan` (`ports R7`).

## Decisions

### 1. Attached text is kept beside the log (the user's decision, 2026-11-25)

A text attachment's text is written to a file the terminal server owns, and the log, the queue slot and the checkpoint hold only a reference to it. It is read back when it is sent to the agent and when a client asks to show it.

```rust
pub enum Attachment {
    Link { path: String, name: String },                             // a file on the agent's host
    Text { id: u64, name: String, bytes: u64 },                      // text kept by the server
}
```

- `Entry::User` and both `Slot` variants that hold a prompt gain `attachments: Vec<Attachment>`, defaulted and omitted when empty, so every existing fixture is unchanged. A reference is about a hundred bytes, so a user entry and a slot stay small, and they travel in `Added` and `Queue` as today: no new event kind.
- **`id`** is the server's, per session, from 1, never reused (`types R3`).
- **Where** (the user's decisions, 2026-11-25: somewhere more durable than the server directory, under `~/.cache` by default, and settable, per remote). `<attachments directory>/<store>/<id>`, on the agent's host. The attachments directory is the one the session was started with when its profile names one, else `<cache directory>/attachments`, where the cache directory is `$BRINDLE_CACHE_DIR`, else `$XDG_CACHE_HOME/brindle`, else `~/.cache/brindle`; on Windows `%LOCALAPPDATA%\brindle\cache`. It is on disk, not in memory as `$XDG_RUNTIME_DIR` usually is, and it outlives a reboot, so a file URI the agent was given, which the agent keeps in its own saved conversation, still leads somewhere when that conversation is taken up again (decision 4).
- **Settings** (`src/config.rs`, parsed at the boundary, `types R5`):
  - `[agent] attachments_dir`: the attachments directory for agents that run on this machine. Unset means the cache directory's rule.
  - `attachments_dir` in a profile's `remote` table (`remote = { type = "ssh", host = "A8", attachments_dir = "/data/brindle" }`): the attachments directory on that host. A path means something only on one machine, so this machine's setting is never used for a remote; a remote that sets none uses that host's own cache directory. Like `connect_at_launch`, it isn't part of the endpoint: `RemoteTarget::endpoint()` drops it, so two profiles on one host still share a connection and the session state never records it. A terminal profile's remote may carry it and nothing reads it.
  - `[agent] attachments_keep_days`: how many days kept text stays after its session ends, 14 by default, 0 to remove it when the session ends, at most 3650; a value outside that is clamped and named on the config line, as an out-of-range font size is. It applies on every host.
  - A configured directory is used as written after `~` is expanded with the session's home on that host; it must be absolute. Brindle sends both values with the request that starts the agent session (decision 3), so they are the session's from then on: a config reload changes them for sessions started afterwards.
- **A directory the server can't make private refuses attaches.** The attachments directory is made if missing, and checked as the server directory is (not a link, the user's own, no one else able to write). One that fails is never used: an attach to that session is refused with the path and what is wrong with it, shown beside the prompt input.
- **A directory per session**, made private to the user as the server directory is (the same checks: not a link, the user's own). Its name, `<store>`, is made once by the core's shell from the date and random bytes and recorded in the session's checkpoint, since session ids start again with each server. Each text is a file named by its id alone. An attachment's name never reaches the file system, so no name can escape the directory. Files are written once and made read-only, so what the conversation shows is what was sent.
- **`cache_dir(env, home, windows)`** is a pure helper beside the platform seam, tested on every platform (`ports R7`, `fcis R3`). `BRINDLE_CACHE_DIR` is what tests and the throwaway scripts set, so no run writes to the real cache (`testing R7`): the e2e and perf harnesses' `isolate` gains it, with a directory per case.
- **Why beside the log.** Inlined, 64 KiB attachments would have pushed the log towards its 16 MiB bound, made every checkpoint copy them, and forced a 128 KiB total so that a queued prompt fitted one frame. Kept aside they cost the log nothing, and the same store holds images next (`agents-prompt-images`). *Rejected:* inlining (above); a store in the holder (it parses nothing and keeps one blob per session, `process R1`).
- *Rejected for the place:* the server directory (`$XDG_RUNTIME_DIR/brindle`), the first draft's choice: it is usually memory-backed, so kept text would count against RAM, and it is emptied at reboot or logout, which breaks every link an agent's saved conversation holds.

### 2. Bounds (`railway R5`, `process R9`)

| Bound | Value |
| --- | --- |
| `MAX_ATTACHMENTS` | 8 a prompt |
| `ATTACHMENT_TEXT` | 256 KiB a text attachment |
| `STORE_LIMIT` | 16 MiB of kept text a session |
| `attachments_keep_days` | 14 days by default after a session ends; 0 to 3650 (decision 1) |
| `EMBED_LIMIT` | 64 KiB: the largest text sent inside the prompt (decision 6) |
| a link's path, a name | bounded as titles are |

- `ATTACHMENT_TEXT` is what one `AGENT_ATTACH` frame holds even if JSON escaping doubles it. Larger payloads need the upload in pieces that `agents-prompt-images` defines.
- **Text over the limit is refused outright, never shortened** (the user's decision, 2026-11-25, over cutting it): Brindle refuses it when it is attached (`AttachRefusal::TooLong { bytes, limit }`), naming its size and the limit on the notice line, and the server refuses it too (`TooLong`), so a client can't exceed the bound. What an agent is given is therefore always the whole of what the user attached.
- **`ATTACHMENT_TEXT` is a transport limit, not a policy.** It is what one frame carries. When `agents-prompt-images` adds an upload in pieces, text uses it too and this limit is raised to whatever that change bounds an upload at; the session's `STORE_LIMIT` stays, since the server must bound what a client can make it write (`process R9`).
- **A full store refuses the attach** (`StoreFull`), saying how much the conversation's attachments hold. Trimmed log entries free their texts (decision 4), so a long conversation makes room as it goes.
- Every text, name and path is cleaned by the server as prompt text is.

### 3. The wire (`process R4`–`R6`)

Three messages are added to client protocol 4, which is unreleased and amended as `agents-tool-content`'s decision 8 says. Their numbers are the next free ones when this change lands: after `agents-commands`' `AGENT_SET_OPTION` (24), `AGENT_ATTACH` is 25 and `AGENT_READ_ATTACHMENT` 26 from the client, and `ATTACHMENT` is 44 from the core, after `AGENT_PAGE` (43).

- **`SPAWN_AGENT`** gains two optional fields, omitted when unset: `attachments_dir` and `attachments_keep_days`. A server that gets neither uses its cache directory and 14 days.
- **`AGENT_ATTACH { req, session, upload, name, text }`** (client): store this text for the session. `upload` is a number the client picks, unique on its connection. Answered `OK`, or `ERROR` with the refusal (`TooLong`, `StoreFull`, `NotAnAgentSession`, `DirectoryNotPrivate { path, why }`, a write failure in words).
- **`AGENT_PROMPT`** gains `attachments`, an optional list in order: `{ "kind": "link", "path", "name" }` or `{ "kind": "text", "upload" }`. A prompt naming an upload the connection doesn't hold is refused (`UnknownUpload`).
- **`AGENT_READ_ATTACHMENT { req, session, id }`** (client), answered by **`ATTACHMENT { req, id, text }`** (server), or `ERROR` (`NoSuchAttachment`, a read failure in words). One text of at most `ATTACHMENT_TEXT`, so one frame.
- **An upload belongs to its connection until a prompt takes it.** The server removes an upload when the prompt naming it is refused, and all of a connection's untaken uploads when it detaches or closes. So a Brindle that crashes between attaching and sending leaves nothing behind.
- Transcript fixtures pin each message's encoding and each refusal; all three join the protocol's fuzzing. New `tests/fixtures/v4/` files for the transcript forms; none edited.
- **The checkpoint** is format 2 (`agents-log-paging`, a base and appended segments), amended: the log's entries and the slot carry the references, in the base and in a segment's events alike, and the state every record carries gains the store's name, the next attachment id and the agent's prompt support (decision 6). It holds no attachment text.

### 4. The store's life (`server/core/attachments.rs`; `process R1`–`R3`, `R12`)

`store::plan(state, event) -> Vec<Step>` is pure: given what the session's log and slot reference and what the directory lists, it says what to write and what to remove. The core's shell runs the steps through `server/platform/`.

- **Kept while referenced.** While its session lives, a text is kept while a log entry or the queue slot references it. It is removed when its entry is trimmed from the log and when its queued or held prompt is discarded (a discarded prompt was never sent, so no agent holds its link).
- **Kept for a while after the session ends** (14 days by default, the user's choice, and settable). The agent may have saved the conversation, with its links, and `agents-session-recovery` and `agents-session-list` let a user take one up again; within that time its attached files are still there. Nothing reads them through Brindle once the session is gone: they are there for the agent.
- **A session ends when the server drops its record, not when its agent stops.** `agents-session-recovery` keeps a session whose agent exited or was lost, with its log, so that it can be continued; such a session is still live here. Its kept texts stay, can be read and shown, and go to the continued agent with a held prompt; no keep time starts. The session ends when its zone is closed or the server otherwise drops it.
- **How the time is kept.** When a session ends, the core writes into its store directory a marker holding the time it may be removed: the end time plus the session's `keep_days`. With `keep_days` 0 the directory is removed at once. A store directory with no marker and no live session (the server died first) is treated as ended when its newest file was written, with the default 14 days.
- **The sweep.** The core removes every store directory whose marker's time has passed, `now` being given to `store::plan` as a value (`fcis R2`). It sweeps the default attachments directory when it starts, and a session's attachments directory whenever a session that uses it starts or ends, so a directory named in a config is swept when it is next used. No timer is needed. It is a cache: the user, or the system, may empty it at any time, and decision 11 says what a missing file does.
- **A core restart** loses nothing: the files are on disk and the store's name and the references are in the checkpoint. The adopting core lists the directory and removes what nothing references (uploads no prompt took before the restart).
- **A missing or unreadable file** isn't fatal: sending leaves that attachment out and adds a warning notice naming it; a read answers `ERROR`, and the zone says the text is no longer kept.
- **Writes are bounded** (one text of at most 256 KiB) and done before the prompt is accepted; a write that fails (the directory is full) refuses the attach in words.
- The server writes and reads only its own attachment directories here. It still reads no file on the agent's behalf (`agents-sessions`' bound).

### 5. Links (`acp/reduce.rs`, `acp/uri.rs`)

- **Resolved when sent.** A link's path is resolved with the rule a session's directory already uses: `~` with the session's `HOME`, a relative path against the session's working directory. The log keeps the path as given.
- **`uri::file(path, windows: bool) -> String`** (pure, tested for both on every platform): `file:///home/u/a%20b.rs`, `file:///C:/Users/u/a.rs`; every byte outside RFC 3986's unreserved set and `/` is percent-encoded. The reducer is given `windows` as a value by the core's shell (`fcis R3`).
- **The server doesn't look at a linked file.** Whether it exists and can be read is the agent's to find out and say.

### 6. How each kind is sent (`acp/forms.rs`, each version's module)

```rust
pub struct PromptSupport { pub embedded: bool, pub images: bool }      // from initialize
pub enum PromptPart<'a> { Text(&'a str), Link { uri: &'a str, name: &'a str }, Embedded { uri: &'a str, name: &'a str, text: &'a str } }
Request::Prompt { session, parts: &[PromptPart] }
```

- **Order:** the user's text first, then the attachments in the order they were added.
- **A link** is `PromptPart::Link`, which the v1 module writes as `resource_link`.
- **Kept text of at most `EMBED_LIMIT`, for an agent that takes embedded context,** is `PromptPart::Embedded`, which the v1 module writes as a `resource` of `text/plain`: the agent has the text at once, with no file to read. Its URI is the kept file's `file://` URI, so the agent can also read it again later.
- **Kept text over `EMBED_LIMIT`, and any kept text for an agent without embedded context,** is `PromptPart::Link` to the kept file. Every agent must accept a link, and a long log then stays out of the agent's context until it chooses to read it.
- The choice is made in the reducer from `PromptSupport` and the text's size, which name no ACP version. The core's shell reads the kept texts a prompt needs just before the reducer builds its parts (a queued prompt may be sent long after it was attached, by another core).
- **Reading a file outside the project.** An agent may ask permission, or refuse, to read the kept file, which is outside its working directory. Task 1.1 captures each agent we can run being sent a link to a kept text. If one balks and advertises additional directories, the session's attachment directory is named to it at `session/new`; what each agent does is written into "As built". Small texts for today's agents never meet this, since they are embedded.
- **ACP v2** (for `agents-acp-v2`): the same content kinds and the same capability, so `acp/v2/encode.rs` writes the same parts.

### 7. The draft (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- The model keeps `draft: Vec<Draft>` beside the prompt input's text, each a link or a text held in memory. `attach(draft) -> Result<(), AttachRefusal>` refuses past `MAX_ATTACHMENTS` (`TooMany`), shown on the window's notice line.
- **Sending** is one sequence the model plans (`send_plan(text, draft) -> Vec<Message>`): an `AGENT_ATTACH` for each text, then the `AGENT_PROMPT` naming them. Any refusal stops it and shows beside the input as a refused prompt does, with the text and the draft kept; the server has already dropped the uploads (decision 3). An accepted prompt clears both. Blank text is still not sent, attachments or not: "The prompt input" is unchanged, and the input's hint says to add a message.
- **Removing.** `backspace_at_start()` removes the last attachment when the cursor is at the very start of the text with nothing selected; `agent_clear_attachments` ("Clear Attachments") removes all. A pointer press on an item's remove mark removes that one. The items aren't keyboard stops.
- **Option commands and completion** (`agents-commands`) look at the text only; an option command leaves the draft alone.
- **The draft is the zone's**, kept while Brindle runs and dropped with the zone; it isn't in the session state file, and nothing of it is on the server until the prompt is sent.

### 8. Where attachments come from

**A command's output** (`src/terminal_view.rs`, `src/workspace.rs`). One thing is sent from a terminal: the last command's output, which is exactly the text Copy Last Command Output copies (`shell-integration`, "Last command output"), with lines joined by `\n` on every platform. That is the grid's text from the most recent command's output mark to its end mark, or to the next prompt or the cursor while it still runs: a line the program wrapped as one line, no trailing blanks, no colours. It is what the grid shows, not the bytes the program wrote, so it has no escape sequences and a progress bar is its last state. It leaves out the prompt and the command line, which becomes the attachment's name. It exists only with shell integration loaded, and not when the output is empty or has left the scrollback.

**A selection has no action of its own** (the user, 2026-11-25): selected text is copied and pasted into the prompt input, where it is part of the prompt's text and is sent when the prompt is. Nothing else is sent from a terminal either: not the scrollback, the visible screen or an earlier command's output.

The action:
- `send_last_output_to_agent` ("Send Last Command Output to Agent"): the text Copy Last Command Output would take, named by the command line (`cargo test`) when the shell reported one, else `output from TAB TITLE`. Listed only once the terminal has a prompt mark, as the output commands are.
- **Which agent zone:** `target_plan(agent_zones) -> Target::{ None, One(zone), Ask(Vec<zone>) }`. `None` shows "no agent zone is open" on the notice line. `Ask` opens a palette of the window's agent zones, the most recently focused first, each as its tab's and zone's titles. Then the attachment joins that zone's draft (or its refusal shows), its tab becomes active and its prompt input takes focus. Nothing is sent.

**A file by path:** `agent_attach_file` ("Attach File…") opens a palette prompt for a path on the agent's host; enter attaches it as a link, as typed. Nothing is checked: the agent says if it can't read it.

**Dropped files** (`drop_plan(paths, host) -> Vec<DropStep>`, pure): for a profile whose agent runs on this machine, each path is a link. For SSH and container profiles each file is `Read(path)`: a background task reads at most `ATTACHMENT_TEXT` + 1 bytes (`process R13`), and the pure `dropped_text(name, bytes) -> Result<Draft, DropRefusal>` makes a text attachment of valid UTF-8 or refuses (`NotText`, `TooLong`, `Unreadable(reason)`). A directory is refused in both cases. Each refusal names the file on the notice line; the others are still attached.

### 9. The zone and `brindle-ui` (`ports R2`)

- **`PromptInput`** gains `set_attachments(Vec<AttachmentItem>)` (an icon for the kind, a name, a detail such as `2.1 KiB`, and a key), drawn as a wrapping row above the textarea and below the queue row; a press on an item's remove mark asks the owner with its key. Not keyboard stops.
- **The queue row** says how many attachments a waiting or held prompt has (`next · 2 attachments`); they go with it when it is sent again and are removed with it when it is discarded.
- **In the conversation** a user message lists its attachments under its text: a link as its path; a text attachment as a row with its name and size. A pointer press on the row opens it: the model asks the server for the text (`AGENT_READ_ATTACHMENT`), the row shows that it is loading (in words, with nothing that moves, so "Motion only while an agent works" is untouched), then the text in a `CodeBlock`, or a line saying the text is no longer kept. `TextState::{ Closed, Loading, Shown(text), Gone(reason) }` per row (`types R1`); closing a row drops its text, so the zone holds only what is open. No keyboard stop.
- **Dropping** on an agent zone highlights its prompt input while files hover.

### 10. The dump (`testing R2`)

```
--- entry 3 user attachments=2 "why does this fail?"
--- attached entry=3 index=0 kind=text id=5 name="cargo test" bytes=262144 state=closed
--- attached entry=3 index=1 kind=link path="src/main.rs" name="main.rs"
--- draft index=0 kind=text name="output from zsh" bytes=412 "…"
--- queued waiting attachments=1 "next"
```

- A user entry line gains `attachments=N` when it has any, followed by its `--- attached` lines; a kept text's line has its id, size and row state (`closed`, `loading`, `shown`, `gone`), and its text only while shown. `--- draft` lines, which hold their text, come before the `--- prompt` line; the `--- queued` line gains `attachments=N`. Texts are escaped with `{:?}`; `wait_text` holds them all.
- The actions run through `--action`; `agent_attach_file`'s prompt takes `--send` and `picker_confirm`; the choose-an-agent palette is driven with the list actions. A drop and a press on a row can't be driven, so their decisions are unit-tested, the read is a real-role test, and the gestures are manual checks.

### 11. Failures (`railway R6`, `R8`)

- Too many attachments: refused when attached, on the notice line, the draft unchanged.
- The server refuses an attach (too long, the store is full, the write failed) or the prompt: the text and the draft stay, the reason beside the input; the server keeps nothing of that prompt.
- A dropped file that isn't text, can't be read, or is a directory: that file refused by name; the rest attached.
- A kept text is missing when its prompt is sent: the prompt goes without it, and a warning notice names it.
- A kept text can't be read for showing: its row says it is no longer kept.
- The agent can't or won't read a linked file: the agent says so in its reply, or asks permission, which Brindle shows as it shows any.
- No agent zone for a command's output: the notice line says so.

### 12. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Kept texts are files, and their references are in the checkpoint, so a waiting or held prompt keeps its attachments across a core restart and an upgrade, and a restored zone can open them. `PromptSupport` is checkpointed with the session. A Brindle crash loses a draft that wasn't sent, as it loses unsent text, and its untaken uploads are removed when its connection closes. A reboot ends the sessions; their kept texts stay in the attachments directory until the sweep finds their time has passed. The holder is untouched: it never sees an attachment.

### 13. With the changes built under this one (refresh, 2026-11-26)

`agents-session-recovery`, `agents-login`, `agents-session-title`, `agents-zone-attention`, `agents-zone-theme-scope` and `agents-log-paging` are built. As built:
- **`agents-login`:** `AGENT_PROMPT` is refused while a login is needed or runs (`LoginRequired`, `LoggingIn`), which keeps the text and the draft as any refusal does, and the server drops that prompt's uploads (decision 3). The `-p` first prompt, which waits read-only for a login and is sent on the first `Ready`, carries no attachments, since `-p` has none (Non-Goals). A terminal login hangs the agent up and starts it again in the same session, so the store and its references are untouched.
- **`agents-session-recovery`:** a turn in flight at the agent's exit is never resubmitted, attachments or not. A held prompt is offered again once the continued agent is ready, with its attachments, and `PromptSupport` is read again from each agent's `initialize`: a text embedded for the first agent is a link for a continued one without embedded context. A session whose agent stopped is still live (decision 4), so its attachments are kept and readable until its zone is closed. A continue the agent replays (`session/load`) shows the user's earlier messages as the agent sends them, as text: a resource the agent replays isn't made an attachment again.
- **`agents-log-paging`:** a user entry is paged and fetched whole and is small, since it holds references. A text is fetched by its own request, not by a page, and a row can be opened on an entry a fetch brought in as on any other. The log now trims from 16 MiB down to 12 MiB at once, so kept texts are freed in larger batches, by the same rule. A reference changes no entry after it is added, so the fold's rule for events below a page's floor has nothing to ignore here.
- **`agents-zone-theme-scope`, `agents-zone-attention`:** the attachment row, the conversation's attachment rows and the drop highlight are parts of the zone, drawn from the zone's token set in the profile's theme. None of them moves. The palette that asks which agent zone takes a command's output is the window's, and names each zone by its title, which is the conversation's title or the profile's name (`agents-session-title`).
- **`agents-markdown`, `agents-markdown-layout-fixes`:** a user's text stays plain and a message is one bubble, with its attachments listed inside it under the text; an attachment's text is drawn as code, never rendered as Markdown.
- **`remote-login-environment`:** an agent on a remote host now starts in its login shell's environment. The cache directory is found from the terminal server's own environment and home on that host, not the agent's, so `XDG_CACHE_HOME` set only in a shell profile doesn't move it; `attachments_dir` on the profile's remote is how to place it.
- **Pointer presses** can't be driven from e2e cases (the in-process pointer hook is the proposal-only `debug-pointer-hook`), so the remove mark, a text attachment's row and drops stay manual checks.
- **`agents-client-state-fixes` (decision 5):** a prompt whose answer was lost with the link is settled by the reconnect's replay through the pure `taken` in `src/agent/model.rs`, which compares the cleaned text with the slot and the user entries after the one the zone held when it was sent. Once a prompt carries attachments, `PromptSent` must record them and `taken` must compare the attachment list as well as the text, or a prompt that differs only in its attachments is taken for the one sent.

### 14. Tests

- **Unit:** the transcript forms and their fold and replay; `store::plan` over attach, send, discard, trim, the session's end, adoption with orphans, and a missing file; `uri::file` for both path kinds and awkward bytes; the reducer's checks, path resolution, order of parts, and embedded or link by support and size; `v1::encode` for each part and `v1::decode` for `promptCapabilities`; `attach` with its refusals, `send_plan`, `backspace_at_start`, `drop_plan`, `dropped_text`, `target_plan`, the row's `TextState`; the dump lines.
- **Protocol:** fixtures and fuzzing for the three messages and `AGENT_PROMPT`'s list.
- **Real roles** (the fake ACP agent): a small text reaches the agent embedded and a large one as a link to a file that holds it; an agent without embedded context gets a link; the file is private and read-only; a read returns the text; a refused prompt, a discard, a trim and the session's end each remove what they should; a waiting prompt keeps its attachment across a core restart; untaken uploads go when the connection closes.
- **Captures:** task 1.1.
- **E2E:** `terminal-to-agent-output`, `terminal-to-agent-choose`, `agent-attach-file`, `agent-attach-bounds`, `agent-attach-queue`, `agent-attach-restore`.
- **Manual:** dropping files on a local and a remote agent's zone; pointer presses on an item's remove mark and on a text attachment's row.

## The user's decisions

Decided on 2026-11-25:
- Images are their own change, after this one.
- Attaching never sends: a command's output reaches the agent only when the user sends the prompt, as pasted text does. A selection gets no action of its own (decision 8).
- Attached text is written beside the log, with a reference in the log, and read on demand (decision 1). The user proposed this in place of the first draft's inlined text.
- The agent is given the kept file's URI where that is the right form: small text is embedded for agents that take it, and everything else is a link to the kept file (decision 6). The user offered "hand off a file URI" conditionally; embedding small text is kept because it needs no file read and ACP prefers it.

- The bounds: 8 attachments, 256 KiB a text, 16 MiB kept a session, and 64 KiB as the most that is embedded (decision 2).
- Kept text lives in the cache directory (`~/.cache/brindle`), not the server directory, so it is on disk and outlives a reboot (decision 1).
- Kept text stays 14 days after its session ends, for some room over the recommended 7 (decision 4).
- Both are settings: `[agent] attachments_keep_days`, `[agent] attachments_dir` for this machine, and `attachments_dir` in a profile's `remote` table for that host, since a path belongs to one machine (decision 1).

- Text over the limit is refused outright, which the user chose over the recommended cutting; the limit itself is raised when uploads in pieces arrive (decision 2).
- The remote's `attachments_dir` on the profile's `remote` table is fine for now; the user expects remotes to become first-class entries of the config later, when it would move there.

- A text file dropped on a remote agent's zone is attached as its contents (decision 8). The user noted this is where the upload comes in: files that aren't text, and ones over the limit, wait for `agents-prompt-images`' upload in pieces.

- With several agent zones open, a palette asks which takes a command's output (decision 8).

None is open.

## Risks / Trade-offs

- An agent may ask before reading a kept file outside its project → only text over 64 KiB, or an agent with no embedded context, is sent that way; the capture of task 1.1 says what each agent does, and additional directories are the remedy where an agent offers them.
- Kept texts may hold secrets a command printed, and now stay on disk for two weeks after their session → the directory is the user's alone, files are named by number, the README says where they are and how long they stay, `attachments_keep_days = 0` removes them with the session, and clearing the directory is always safe.
- The cache directory can grow → 16 MiB a session, trimmed entries free theirs, and the sweep removes ended sessions' directories once their time has passed.
- A directory named in a config is swept only when a session next uses it → nothing is removed late by more than the gap between uses, and nothing is removed early.
- A remote's `attachments_dir` sits on a table terminal profiles share → it is dropped from the endpoint as `connect_at_launch` is, so it changes no connection, and a terminal profile ignores it.
- The cache may be emptied under a live session → a missing text is left out of a prompt with a notice, and its row says it is no longer kept.
- A prompt needs text even with attachments → one sentence of intent goes with the context, and "The prompt input" needs no change.
- A link's file isn't checked → a typo shows up as the agent's reply, not at once.
- A text can't be shown once its session has ended → the conversation still lists its name and size.
- Drops and row presses can't be driven by the debug hooks → their decisions are pure and unit-tested, and the gestures are manual checks.

## Migration Plan

Nothing to migrate. Sessions, logs and queued prompts from before the change have no attachments and decode unchanged; the attachments directory is made when first needed.

## As built (2026-11-28)

Numbered flags for review; each is a decision taken while building, not a change of the specified behaviour.

1. **Base.** Built on 7e29de6c, the head of `perf/agents-absorb-cost` (unpushed), which sits on `perf/agents-checkpoint-deltas` (PR #158, unmerged), on main 2bdbeec6. The branch lands after both.
2. **Task 1.1 is owed.** No real agent was run. Decision 6 is applied by its conservative reading until the captures say otherwise: the session's attachment directory is named in `additionalDirectories` at `session/new`, `session/load` and `session/resume` for every agent that advertises `sessionCapabilities.additionalDirectories` (`reduce/lifecycle.rs`, `directories`), and to none that doesn't. Of the captured `initialize` answers, every agent takes embedded context, and all but Copilot (and one Claude ACP build) advertise additional directories.
3. **Names that moved.** `acp/reduce.rs` is `acp/reduce/` (the attachment decisions in `reduce/attach.rs`), `acp/checkpoint.rs` is `acp/checkpoint/` (the fields in `state.rs`), the transcript crate's lib is `event.rs`, `fold.rs`, `page.rs` and `output.rs`, the client's `model.rs` is `src/agent/model/`, and `store::plan` is `server/core/attachments.rs`'s `plan`. The new events stay within `MAX_EVENT` with rows in `acp/limits.rs` (`PROMPT_ADDED_EVENT`, `ATTACHMENTS_LEN` in `QUEUE_EVENT`), and the store and prompt support are small and bounded, so every checkpoint record holds them whole (`agents-checkpoint-deltas` decision 4).
4. **`ATTACHMENT` carries `session`**, as every other answer about a session does, so the client routes it to the zone without remembering the request.
5. **`SPAWN_AGENT`'s two fields** landed with the protocol commit (task 1.4), and are filled by task 1.6c.
6. **Cleaned before counted.** Brindle cleans an attachment's text and name (`brindle_transcript::clean`) before checking its size and sending it, and the server cleans again. Cleaned text escapes to at most twice its bytes in JSON, so `ATTACHMENT_TEXT` plus `ATTACH_ENVELOPE` fits one frame (pinned by `the_largest_attachment_fits_one_frame`).
7. **An embedded text's name isn't sent.** ACP v1's `resource` (an `EmbeddedResource` of `TextResourceContents`) has no name field, so the agent gets the kept file's URI and the text; the name is in Brindle's log only. A link keeps its name.
8. **A link's path past `MAX_PATH` is refused** (`Refused::PathTooLong`), not cut: a cut path names another file. A name is cut at a character boundary to `MAX_ATTACHMENT_NAME`.
9. **The tests' cache directory is in the server directory.** The e2e and perf harnesses' `isolate` set `BRINDLE_CACHE_DIR` to `<server dir>/cache` (`common::server::cache_dir`), and the real-role tests to `<dir>/cache`, so it is removed with the server directory and a case finds its store through one helper.
10. **The keep time is always sent.** Brindle fills `SPAWN_AGENT.attachments_keep_days` from `[agent] attachments_keep_days` (clamped) for every agent profile, and `attachments_dir` only when set, so the server's default of 14 days is for older clients.
11. **`Listed` is neutral.** The directory listing the store's plan reads is `server/platform/mod.rs`'s `Listed` (re-exported by `attachments.rs`), so the pure module names no platform module.
12. **The sweep is its own function.** `attachments::sweep(found, live, now)` decides which store directories go, apart from `plan`, since it reads a whole attachments directory rather than one store; `sweep_root` says which directory a core's start and a session's start or end sweeps. The core's start sweeps the default directory once it first accepts clients, after the holders' sessions are adopted, so no live session's store is judged by its files' age.
13. **A directory is made private again.** On Windows `make_private` protects an existing directory with the user's own ACL, as on Unix it sets 0700 after checking it is the user's and not a link; a directory another user owns is refused.
14. **The core's shell is `server/core/kept.rs`.** It holds each connection's untaken uploads and, per agent session, its store and the texts in it with their sizes. A store is known when its session first appears (each turn's `reconcile_stores`) and ends when its record is gone, which covers every way a session is removed without hooking each. After a step whose events include `Queue` or `Trimmed`, after a prompt with attachments (taken or refused) and when a connection closes, is replaced or detaches a session with untaken uploads, the plan's `Changed` removes what nothing references. A kept text read for a prompt stays in `AgentSession::texts` only while the queue slot names it.
