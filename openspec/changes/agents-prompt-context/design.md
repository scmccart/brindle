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

**Non-Goals:** images and audio (`agents-prompt-images`); completing a path after `@`, and listing an agent host's files (`agents-file-mentions`); an action for a terminal selection (it is copied and pasted); attachments on `brindle agent -p`; a long paste becoming an attachment; directories; reading a linked file's contents from the agent's host; a prompt with attachments and blank text; sending on attach; attachment items as tab stops; showing an attachment's text in Brindle after its session has ended; a setting for how long kept text stays. No config key and no state file.

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
| The draft's attachments, `cut_text`, `drop_plan`, the conversation's attachment rows and their fetched text, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
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
    Text { id: u64, name: String, bytes: u64, cut: Option<Cut> },    // text kept by the server
}
pub enum Cut { Start, End }                                          // which end of a longer text was dropped
```

- `Entry::User` and both `Slot` variants that hold a prompt gain `attachments: Vec<Attachment>`, defaulted and omitted when empty, so every existing fixture is unchanged. A reference is about a hundred bytes, so a user entry and a slot stay small, and they travel in `Added` and `Queue` as today: no new event kind.
- **`id`** is the server's, per session, from 1, never reused (`types R3`). **`cut`** is data, not a mark in the text.
- **Where** (the user's decision, 2026-11-25: somewhere more durable than the server directory, under `~/.cache`). `<cache directory>/attachments/<store>/<id>`, on the agent's host. The cache directory is `$BRINDLE_CACHE_DIR`, else `$XDG_CACHE_HOME/brindle`, else `~/.cache/brindle`; on Windows `%LOCALAPPDATA%\brindle\cache`. It is on disk, not in memory as `$XDG_RUNTIME_DIR` usually is, and it outlives a reboot, so a file URI the agent was given, which the agent keeps in its own saved conversation, still leads somewhere when that conversation is taken up again (decision 4).
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
| `EMBED_LIMIT` | 64 KiB: the largest text sent inside the prompt (decision 6) |
| a link's path, a name | bounded as titles are |

- `ATTACHMENT_TEXT` is what one `AGENT_ATTACH` frame holds even if JSON escaping doubles it. Larger payloads need the upload in pieces that `agents-prompt-images` defines.
- **Brindle cuts before it sends** (`cut_text(text, keep) -> (String, Option<Cut>)`, pure, at a line break where one is near, else a character boundary): command output keeps its end, where the error is; a file keeps its start. The server refuses what is still over a bound, so a client can't exceed them.
- **A full store refuses the attach** (`StoreFull`), saying how much the conversation's attachments hold. Trimmed log entries free their texts (decision 4), so a long conversation makes room as it goes.
- Every text, name and path is cleaned by the server as prompt text is.

### 3. The wire (`process R4`–`R6`)

Three messages are added to client protocol 4, which is unreleased and amended as `agents-tool-content`'s decision 8 says. Their numbers are the next free ones when the stack is assembled.

- **`AGENT_ATTACH { req, session, upload, name, text, cut }`** (client): store this text for the session. `upload` is a number the client picks, unique on its connection. Answered `OK`, or `ERROR` with the refusal (`TooLong`, `StoreFull`, `NotAnAgentSession`, a write failure in words).
- **`AGENT_PROMPT`** gains `attachments`, an optional list in order: `{ "kind": "link", "path", "name" }` or `{ "kind": "text", "upload" }`. A prompt naming an upload the connection doesn't hold is refused (`UnknownUpload`).
- **`AGENT_READ_ATTACHMENT { req, session, id }`** (client), answered by **`ATTACHMENT { req, id, text }`** (server), or `ERROR` (`NoSuchAttachment`, a read failure in words). One text of at most `ATTACHMENT_TEXT`, so one frame.
- **An upload belongs to its connection until a prompt takes it.** The server removes an upload when the prompt naming it is refused, and all of a connection's untaken uploads when it detaches or closes. So a Brindle that crashes between attaching and sending leaves nothing behind.
- Transcript fixtures pin each message's encoding and each refusal; all three join the protocol's fuzzing. New `tests/fixtures/v4/` files for the transcript forms; none edited.
- **The checkpoint** keeps format 1, amended: the log's entries and the slot carry the references, and the session state gains the next attachment id and the agent's prompt support (decision 6). It holds no attachment text.

### 4. The store's life (`server/core/attachments.rs`; `process R1`–`R3`, `R12`)

`store::plan(state, event) -> Vec<Step>` is pure: given what the session's log and slot reference and what the directory lists, it says what to write and what to remove. The core's shell runs the steps through `server/platform/`.

- **Kept while referenced.** While its session lives, a text is kept while a log entry or the queue slot references it. It is removed when its entry is trimmed from the log and when its queued or held prompt is discarded (a discarded prompt was never sent, so no agent holds its link).
- **Kept for a while after the session ends.** A session's directory stays for `RETENTION` = 7 days after the session ends, then goes. The agent may have saved the conversation, with its links, and `agents-session-recovery` and `agents-session-list` let a user take one up again; within that week its attached files are still there. Nothing reads them through Brindle once the session is gone: they are there for the agent.
- **The sweep.** When a core starts and whenever a session ends, the core removes every store directory that no live session uses and whose newest file is older than `RETENTION`, `now` being given to `store::plan` as a value (`fcis R2`). No timer is needed. It is a cache: the user, or the system, may empty it at any time, and decision 11 says what a missing file does.
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
- **A cut text** says so to the agent in its name: `NAME (the first 12 KiB aren't included)`.
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

**Dropped files** (`drop_plan(paths, host) -> Vec<DropStep>`, pure): for a profile whose agent runs on this machine, each path is a link. For SSH and container profiles each file is `Read(path)`: a background task reads at most `ATTACHMENT_TEXT` + 1 bytes (`process R13`), and the pure `dropped_text(name, bytes) -> Result<Draft, DropRefusal>` makes a text attachment of valid UTF-8 (cut at its end if longer) or refuses (`NotText`, `Unreadable(reason)`). A directory is refused in both cases. Each refusal names the file on the notice line; the others are still attached.

### 9. The zone and `brindle-ui` (`ports R2`)

- **`PromptInput`** gains `set_attachments(Vec<AttachmentItem>)` (an icon for the kind, a name, a detail such as `2.1 KiB` or `cut`, and a key), drawn as a wrapping row above the textarea and below the queue row; a press on an item's remove mark asks the owner with its key. Not keyboard stops.
- **The queue row** says how many attachments a waiting or held prompt has (`next · 2 attachments`); they go with it when it is sent again and are removed with it when it is discarded.
- **In the conversation** a user message lists its attachments under its text: a link as its path; a text attachment as a row with its name and size. A pointer press on the row opens it: the model asks the server for the text (`AGENT_READ_ATTACHMENT`), the row shows that it is loading, then the text in a `CodeBlock` with a line saying which end was cut, or that the text is no longer kept. `TextState::{ Closed, Loading, Shown(text), Gone(reason) }` per row (`types R1`); closing a row drops its text, so the zone holds only what is open. No keyboard stop.
- **Dropping** on an agent zone highlights its prompt input while files hover.

### 10. The dump (`testing R2`)

```
--- entry 3 user attachments=2 "why does this fail?"
--- attached entry=3 index=0 kind=text id=5 name="cargo test" cut=start bytes=262144 state=closed
--- attached entry=3 index=1 kind=link path="src/main.rs" name="main.rs"
--- draft index=0 kind=text name="output from zsh" cut=none bytes=412 "…"
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

Kept texts are files, and their references are in the checkpoint, so a waiting or held prompt keeps its attachments across a core restart and an upgrade, and a restored zone can open them. `PromptSupport` is checkpointed with the session. A Brindle crash loses a draft that wasn't sent, as it loses unsent text, and its untaken uploads are removed when its connection closes. A reboot ends the sessions; their kept texts stay in the cache directory until the sweep finds them a week old. The holder is untouched: it never sees an attachment.

### 13. With the changes stacked under this one

From their designs, as the stack's session described them on 2026-11-25:
- **`agents-login`:** `AGENT_PROMPT` is refused while a login runs (`LoggingIn`), which keeps the text and the draft as any refusal does. The `-p` first prompt, which waits read-only for a login and is sent on the first `Ready`, carries no attachments, since `-p` has none (Non-Goals).
- **`agents-session-recovery`:** a turn in flight at the agent's exit is never resubmitted, attachments or not. A held prompt is offered again once the recovered agent is ready, with its attachments, and `PromptSupport` is read again from the restarted agent's `initialize`. The store lasts as long as the server keeps the session's record, so a conversation that can be continued keeps its attachments.
- **`agents-log-paging`:** a user entry is paged whole and is small, since it holds references. A text is fetched by its own request, not by a page.
- **`agents-markdown`:** a user's text stays plain; an attachment's text is drawn as code, never rendered as Markdown.
- **Pointer presses** can't be driven from e2e cases (the in-process pointer hook is the proposal-only `debug-pointer-hook`), so the remove mark, a text attachment's row and drops stay manual checks.

### 14. Tests

- **Unit:** the transcript forms and their fold and replay; `store::plan` over attach, send, discard, trim, the session's end, adoption with orphans, and a missing file; `uri::file` for both path kinds and awkward bytes; the reducer's checks, path resolution, order of parts, embedded or link by support and size, and the cut note; `v1::encode` for each part and `v1::decode` for `promptCapabilities`; `cut_text`, `attach`, `send_plan`, `backspace_at_start`, `drop_plan`, `dropped_text`, `target_plan`, the row's `TextState`; the dump lines.
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

Still open, each with a recommendation the text above follows:

1. **How long kept text stays after its session ends:** 7 days (recommended), another length, or until the cache is cleared by hand.
2. **Longer text is cut** (command output keeps its end, a file its start; recommended), or refused.
3. **A file dropped on a remote agent's zone** is attached as text when it is text (recommended), or every such drop is refused.
4. **Several agent zones open:** a palette asks which (recommended), or the most recently focused one takes it.

## Risks / Trade-offs

- An agent may ask before reading a kept file outside its project → only text over 64 KiB, or an agent with no embedded context, is sent that way; the capture of task 1.1 says what each agent does, and additional directories are the remedy where an agent offers them.
- Kept texts may hold secrets a command printed, and now stay on disk for a week after their session → the directory is the user's alone, files are named by number, the README says where they are and how long they stay, and clearing `~/.cache/brindle` is always safe.
- The cache directory can grow → 16 MiB a session, trimmed entries free theirs, and the sweep removes ended sessions' directories after a week.
- The cache may be emptied under a live session → a missing text is left out of a prompt with a notice, and its row says it is no longer kept.
- A prompt needs text even with attachments → one sentence of intent goes with the context, and "The prompt input" needs no change.
- A link's file isn't checked → a typo shows up as the agent's reply, not at once.
- A text can't be shown once its session has ended → the conversation still lists its name and size.
- Drops and row presses can't be driven by the debug hooks → their decisions are pure and unit-tested, and the gestures are manual checks.

## Migration Plan

Nothing to migrate. Sessions, logs and queued prompts from before the change have no attachments and decode unchanged; the attachments directory is made when first needed.
