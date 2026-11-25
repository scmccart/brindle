# Design

## Context

See proposal.md for why. The user split images into their own change on 2026-11-25 (`agents-prompt-images`). Facts this design rests on, from the code on `feat/agents-profiles` (2026-11-25), the stacked designs, the pinned schema crate and the captured agent traffic:

- **A prompt is text today.** `AGENT_PROMPT { req, session, text }`; the reducer refuses text over `MAX_PROMPT` = 256 KiB, adds `Entry::User { text }`, and `rpc.rs` writes `session/prompt` with one `text` content block. The queue slot is `Slot::Waiting { text }` or `Held { text, reason }` and travels whole in one `Event::Queue`. A user entry's text travels as `Added` plus `Appended` pieces of `TEXT_CHUNK` = 64 KiB.
- **One frame holds at most 1 MiB** (`MAX_FRAME_PAYLOAD_SIZE`), and every event and client message must fit one. JSON escaping can double a text of line breaks.
- **ACP prompt content** (v1 and v2, the same kinds): `text`; `resource_link { uri, name, … }`, which "all agents MUST support"; `resource` with embedded text or a blob, allowed only when the agent's `promptCapabilities.embeddedContext` is true; `image` and `audio`, each behind its own capability. Every captured agent's `initialize` answer has `embeddedContext: true` and `image: true`; none has `audio: true`.
- **`agents-tool-content`** gives the `Dialect` seam, `acp/forms.rs` and `Request`; `agents-markdown` gives `CodeBlock`.
- **The terminal** (`shell-integration`): Copy Last Command Output takes the text from a command's output mark to its end, only once a prompt mark was received; the command line is known where the shell reports it. `Terminal::selection_text()` gives the selection.
- **Brindle** has one window. A tab is all terminal zones or all agent zones (`agents-profiles`). A profile's `Host` says whether its program runs on this machine, over SSH or in a container. GPUI 0.2.2 delivers files dropped on an element (`ExternalPaths`); nothing in Brindle handles a drop yet.
- **`brindle-ui`'s `PromptInput`** has the queue row above its textarea, whose buttons are keyboard stops, and (with `agents-commands`) a suggestion list and footer items that aren't.

## Goals / Non-Goals

**Goals:**
- What is on a terminal's screen reaches an agent in two actions, named and bounded, without the clipboard.
- A file is pointed at as a file, by its path on the agent's host, and never copied when the agent can read it itself.
- An attachment is part of its prompt everywhere the prompt goes: the queue, a restart, the conversation.
- Every agent is served: one that takes no embedded context still gets the text.
- The attachment form is ready for images to join as a third kind.
- No new keyboard stop, and no change to a requirement another stacked change modifies.

**Non-Goals:** images and audio (`agents-prompt-images`); completing a path after `@`, and listing an agent host's files (`agents-file-mentions`); attachments on `brindle agent -p`; a long paste becoming an attachment; directories; reading a file's contents from the agent's host; a prompt with attachments and blank text; sending on attach; attachment items as tab stops; a context menu in the terminal. No config key and no state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Attachment`; `Entry::User`'s and `Slot`'s attachments; `Event::Attached`; the fold, replay and bounds | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| `AGENT_PROMPT`'s `attachments` | wire, additive | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| `PromptSupport`, `PromptPart`, `Request::Prompt`'s parts | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| Checks, bounds, the text fallback, resolving a link's path | pure | `acp/reduce.rs` |
| A path as a `file://` URI | pure | `acp/uri.rs` (new) |
| ACP v1: `promptCapabilities`; resource links and embedded resources in `session/prompt` | pure | `acp/v1/{decode, encode}.rs` |
| The draft's attachments, `cut_text`, `drop_plan`, the conversation's attachment rows, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| Which agent zone takes terminal text (`target_plan`) | pure | `brindle`, `src/workspace.rs` |
| Reading a dropped file | shell (background task) | `src/agent/mod.rs` |
| The terminal actions, the drop handler, the attach prompt, drawing | shell (entities) | `src/terminal_view.rs`, `src/workspace.rs`, `src/agent/view.rs`, `src/actions.rs` |
| `PromptInput`'s attachment row | component | `brindle-ui`, `ui/src/components/prompt_input.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`).

## Decisions

### 1. The form (`types R1`, `R2`, `R7`)

```rust
pub enum Attachment {
    Link { path: String, name: String },                 // a file on the agent's host
    Text { name: String, text: String, cut: Option<Cut> },
}
pub enum Cut { Start, End }                              // which end of a longer text was dropped
```

- `Entry::User` and both `Slot` variants that hold a prompt gain `attachments: Vec<Attachment>`, defaulted and omitted when empty, so every existing fixture is unchanged.
- **A link's `path` is as the user gave it**, relative paths included; the server resolves it when the prompt is sent (decision 4). `name` is the file's last component.
- **`cut`** is `None` for a whole text; `Some(Start)` when the beginning was dropped (command output) and `Some(End)` when the end was. It is data, not a flag in the text, so the zone and the agent's copy can each say it their own way.
- **A third kind is expected.** `agents-prompt-images` adds `Image`; since protocol 4 is unreleased it joins the same enum.

### 2. Bounds (`railway R5`, `process R9`)

| Bound | Value |
| --- | --- |
| `MAX_ATTACHMENTS` | 8 a prompt |
| `ATTACHMENT_TEXT` | 64 KiB a text attachment (= `TEXT_CHUNK`) |
| `ATTACHMENTS_TEXT` | 128 KiB of attachment text a prompt |
| a link's path, a name | bounded as titles are |

- With the 256 KiB prompt, a queue slot is at most 384 KiB of text, which fits one frame even if JSON escaping doubles it. That is why the total is 128 KiB and not more.
- **Brindle cuts before it sends** (`cut_text(text, keep) -> (String, Option<Cut>)`, pure, at a line break where one is near, else a character boundary): command output keeps its end, where the error is; a selection and a file keep their start. The server refuses what is still over a bound (`TooManyAttachments`, `AttachmentTooLong`, `AttachmentsTooLong`), so a client can't exceed them.
- Every text and path is cleaned by the reducer as prompt text is.

### 3. The wire, events and the fold (`process R4`–`R6`)

- **`AGENT_PROMPT`** gains `attachments`, an optional list, omitted when empty. A core that predates this change ignores the field, which would send the prompt without its context, so the client refuses to send attachments to an endpoint whose server doesn't know them. Since no released server has protocol 4 at all, that gate is the protocol 4 gate `agent_start` already applies; if protocol 4 is released before this lands, the field needs protocol 5 (this decision is then reopened, as `agents-tool-content`'s decision 8 says).
- **`Event::Attached { id, attachment }`** adds one attachment to a user entry, so no event carries more than one text attachment. `Added` for a user entry carries none; the reducer and `replay()` follow it with one `Attached` each. Fold errors: `NotAUser { id }`, `TooManyAttachments { id }`. The log is unchanged on an error.
- **`Event::Queue { slot }`** carries the slot whole, attachments included (decision 2 bounds it).
- `fold(replay(t)) == t` gains attachments; new `tests/fixtures/v4/` files, none edited; the new forms join `fuzz.rs`.
- **The checkpoint** keeps format 1, amended: the log's entries and the slot carry the new field, and the session state gains the agent's prompt support (decision 5).

### 4. Links (`acp/reduce.rs`, `acp/uri.rs`)

- **Resolved when sent.** A link's path is resolved with the rule a session's directory already uses: `~` with the session's `HOME`, a relative path against the session's working directory. The log keeps the path as given, which is what the user typed or dropped.
- **`uri::file(path, windows: bool) -> String`** (pure, tested for both on every platform): `file:///home/u/a%20b.rs`, `file:///C:/Users/u/a.rs`; every byte outside RFC 3986's unreserved set and `/` is percent-encoded. The reducer is given `windows` as a value by the core's shell (`fcis R3`), since the agent's host is the server's.
- **The server doesn't look at the file.** Whether it exists and can be read is the agent's to find out and say; the server reads no file on an agent's behalf here either (`agents-sessions`' bound).

### 5. How each kind is sent (`acp/forms.rs`, each version's module)

```rust
pub struct PromptSupport { pub embedded: bool, pub images: bool }      // from initialize
pub enum PromptPart<'a> { Text(&'a str), Link { uri: &'a str, name: &'a str }, Embedded { uri: &'a str, name: &'a str, text: &'a str } }
Request::Prompt { session, parts: &[PromptPart] }
```

- The dialect's `initialize_result` gives `PromptSupport`; the session keeps it and the checkpoint records it, so a queued prompt sent after a restart is still sent the right way.
- **Order:** the user's text first, then the attachments in the order they were added.
- **A link** is `PromptPart::Link`, which the v1 module writes as `resource_link`.
- **Text, when the agent takes embedded context**, is `PromptPart::Embedded` with the URI `brindle:///attachment/N/NAME` (N its place in the prompt, the name percent-encoded), which the v1 module writes as a `resource` of `text/plain`. The scheme isn't `file`, so an agent doesn't take it for a path it could read or edit.
- **Text, when it doesn't** (`embedded: false`): the reducer appends it to the prompt's text block instead, as a blank line, the name on its own line, and the text in a fence longer than any run of backticks inside it. This is decided in the reducer from `PromptSupport`, which names no ACP version.
- **A cut text** says so to the agent in its name: `NAME (the first 12 KiB aren't included)`.
- **ACP v2** (for `agents-acp-v2`): the same three content kinds and the same capability, so `acp/v2/encode.rs` writes the same parts.

### 6. The draft (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- The model keeps `draft: Vec<Attachment>` beside the prompt input's text. `attach(attachment) -> Result<(), AttachRefusal>` refuses past `MAX_ATTACHMENTS` and past `ATTACHMENTS_TEXT` (`TooMany`, `TooMuchText { over }`), shown on the window's notice line.
- **Sending** takes the text and the draft together. A prompt the server accepts clears both; a refused one keeps both, with the reason beside the input as today. Blank text is still not sent, attachments or not: "The prompt input" is unchanged, and the input's hint says to add a message.
- **Removing.** `backspace_at_start()` removes the last attachment when the cursor is at the very start of the text with nothing selected; `agent_clear_attachments` ("Clear Attachments") removes all. A pointer press on an item's remove mark removes that one. The items aren't keyboard stops.
- **Option commands and completion** (`agents-commands`) look at the text only; an option command leaves the draft alone.
- **The draft is the zone's**, kept while Brindle runs and dropped with the zone; it isn't in the session state file.

### 7. Where attachments come from

**Terminal text** (`src/terminal_view.rs`, `src/workspace.rs`):
- `send_selection_to_agent` ("Send Selection to Agent"): the selection's text, named `selection from TAB TITLE`. Listed and enabled only with a selection.
- `send_last_output_to_agent` ("Send Last Command Output to Agent"): the text Copy Last Command Output would take, named by the command line (`cargo test`) when the shell reported one, else `output from TAB TITLE`. Listed only once the terminal has a prompt mark, as the output commands are.
- **Which agent zone:** `target_plan(agent_zones) -> Target::{ None, One(zone), Ask(Vec<zone>) }`. `None` shows "no agent zone is open" on the notice line. `Ask` opens a palette of the window's agent zones, the most recently focused first, each as its tab's and zone's titles. Then the attachment joins that zone's draft (or its refusal shows), its tab becomes active and its prompt input takes focus. Nothing is sent.

**A file by path:** `agent_attach_file` ("Attach File…") opens a palette prompt for a path on the agent's host; enter attaches it as a link, as typed. Nothing is checked: the agent says if it can't read it.

**Dropped files** (`drop_plan(paths, host) -> Vec<DropStep>`, pure): for a profile whose agent runs on this machine, each path is a link. For SSH and container profiles each file is `Read(path)`: a background task reads at most `ATTACHMENT_TEXT` + 1 bytes (`process R13`), and the pure `dropped_text(name, bytes) -> Result<Attachment, DropRefusal>` makes a text attachment of valid UTF-8 (cut at its end if longer) or refuses (`NotText`, `Unreadable(reason)`). A directory is refused in both cases. Each refusal names the file on the notice line; the others are still attached.

### 8. The zone and `brindle-ui` (`ports R2`)

- **`PromptInput`** gains `set_attachments(Vec<AttachmentItem>)` (an icon for the kind, a name, a detail such as `2.1 KiB` or `cut`, and a key), drawn as a wrapping row above the textarea and below the queue row; a press on an item's remove mark asks the owner with its key. Not keyboard stops.
- **The queue row** says how many attachments a waiting or held prompt has (`next · 2 attachments`); they go with it when it is sent again and are dropped with it when it is discarded or removed.
- **In the conversation** a user message lists its attachments under its text: a link as its path; a text attachment as a row with its name and size that opens, on a pointer press, to its text in a `CodeBlock`, with a line saying which end was cut. Rows start closed; `by_hand` state as tool rows have (`agents-tool-content`). No keyboard stop.
- **Dropping** on an agent zone highlights its prompt input while files hover.

### 9. The dump (`testing R2`)

```
--- entry 3 user attachments=2 "why does this fail?"
--- attached entry=3 index=0 kind=text name="cargo test" cut=start bytes=65536 "…"
--- attached entry=3 index=1 kind=link path="src/main.rs" name="main.rs"
--- draft index=0 kind=text name="selection from zsh" cut=none bytes=412 "…"
--- queued waiting attachments=1 "next"
```

- A user entry line gains `attachments=N` when it has any, followed by its `--- attached` lines; `--- draft` lines come before the `--- prompt` line; the `--- queued` line gains `attachments=N`. Texts are escaped with `{:?}`; `wait_text` holds them all.
- The actions run through `--action`; `agent_attach_file`'s prompt takes `--send` and `picker_confirm`; the choose-an-agent palette is driven with the list actions. A drop can't be driven, so `drop_plan` and `dropped_text` are unit-tested and the drop is a manual check.

### 10. Failures (`railway R6`, `R8`)

- Too many attachments, or too much text: refused when attached, on the notice line, the draft unchanged.
- A dropped file that isn't text, can't be read, or is a directory: that file refused by name; the rest attached.
- The server refuses the prompt (a bound, the agent not ready): text and draft stay, the reason beside the input, as for any refused prompt.
- The agent can't read a linked file: the agent says so in its reply; Brindle shows that as it shows any reply.
- No agent zone for terminal text: the notice line says so.

### 11. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Attachments in the log and the slot are in the checkpoint, so a waiting or held prompt keeps them across a core restart and an upgrade, and a restored zone shows them. `PromptSupport` is checkpointed with the session. A Brindle crash loses a draft that wasn't sent, as it loses unsent text.

### 12. With the changes stacked under this one

From their designs, as the stack's session described them on 2026-11-25:
- **`agents-login`:** `AGENT_PROMPT` is refused while a login runs (`LoggingIn`), which keeps the text and the draft as any refusal does. The `-p` first prompt, which waits read-only for a login and is sent on the first `Ready`, carries no attachments, since `-p` has none (Non-Goals); attaching while it waits adds to the draft, and the draft goes with whatever is sent next.
- **`agents-session-recovery`:** a turn in flight at the agent's exit is never resubmitted, attachments or not. A held prompt is offered again once the recovered agent is ready, with its attachments, and `PromptSupport` is read again from the restarted agent's `initialize`.
- **`agents-log-paging`:** a user entry is paged whole, so its attachments come with it and count against the page's budget. The largest user entry here is 384 KiB of text, above a 256 KiB page, which paging's rule of always sending the last entry whole already covers.
- **`agents-markdown`:** a user's text stays plain; an attachment's text is drawn as code, never rendered as Markdown.
- **Pointer presses** can't be driven from e2e cases (the in-process pointer hook is the proposal-only `debug-pointer-hook`), so the remove mark, a text attachment's row and drops stay manual checks.

### 13. Tests

- **Unit:** the fold, replay and bounds; `uri::file` for both path kinds and awkward bytes; the reducer's checks, path resolution, order of parts, the fenced fallback (including text holding backtick runs) and the cut note; `v1::encode` for each part and `v1::decode` for `promptCapabilities`; `cut_text`, `attach`, `backspace_at_start`, `drop_plan`, `dropped_text`, `target_plan`; the dump lines.
- **Protocol:** fixtures for `AGENT_PROMPT` with and without attachments; fuzzing.
- **Real roles** (the fake ACP agent): a prompt with a link and a text reaches the agent as a resource link and an embedded resource; an agent without embedded context gets the fenced text; a waiting prompt keeps its attachments across a core restart.
- **E2E:** `terminal-to-agent-output`, `terminal-to-agent-selection`, `terminal-to-agent-choose`, `agent-attach-file`, `agent-attach-bounds`, `agent-attach-queue`, `agent-attach-restore`.
- **Manual:** dropping files on a local and a remote agent's zone; pointer presses on an item's remove mark and on a text attachment's row.

## Open decisions

Each has a recommendation, which the text above follows. They are the user's to settle before implementation.

1. **Attached text is kept whole in the conversation** (recommended), so the record shows what the agent was given, or only its name and size.
2. **The bounds:** 8 attachments, 64 KiB a text and 128 KiB together (recommended; the total is what keeps a queued prompt inside one 1 MiB frame).
3. **Longer text is cut** (command output keeps its end, other text its start; recommended), or refused.
4. **A file dropped on a remote agent's zone** is embedded as text when it is text (recommended), or every such drop is refused.
5. **Several agent zones open:** a palette asks which (recommended), or the most recently focused one takes it.
6. **An agent that takes no embedded context** gets the text written into the prompt in a fence (recommended), or the attachment is refused for that agent.
7. **Attaching never sends** (recommended), or the terminal actions send at once when the prompt has text.

## Risks / Trade-offs

- A prompt needs text even with attachments → one sentence of intent goes with the context, and "The prompt input" needs no change; a later change can relax it.
- A link's file isn't checked → a typo shows up as the agent's reply, not at once; checking would mean the server reading the agent host's files.
- 128 KiB of attachment text is small beside a long build log → the end of the output is where errors are, and the item says how much was cut.
- An embedded text's `brindle:` URI means nothing to an agent → it is a name, and the text is in the block; a `file:` URI would invite the agent to edit a file that may not exist on its host.
- Drops can't be driven by the debug hooks → their decisions are pure and unit-tested, and the gesture is a manual check.

## Migration Plan

Nothing to migrate. Sessions, logs and queued prompts from before the change have no attachments and decode unchanged.
