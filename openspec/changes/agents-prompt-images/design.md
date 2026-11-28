# Design

## Context

See proposal.md for why. Facts this design rests on, from `agents-prompt-context`'s design (2026-11-25), the code on `feat/agents-profiles`, the pinned schema crate and GPUI 0.2.2:

- **`agents-prompt-context`** gives: `Attachment::{Link, Text}` as references in the log and the slot; a store of kept texts under `~/.cache/brindle/attachments/<store>/<id>` on the agent's host, written once and read-only, removed by a pure `store::plan`; `AGENT_ATTACH` (one text in one frame, under an `upload` number the client picks), `AGENT_READ_ATTACHMENT` and `ATTACHMENT` (one whole text); `ATTACHMENT_TEXT` = 256 KiB, refused outright past it and described as "what one frame carries"; `STORE_LIMIT` = 16 MiB; `EMBED_LIMIT` = 64 KiB; `PromptSupport { embedded, images }` read from `initialize` and checkpointed; `drop_plan`, which refuses a non-text file for a remote agent.
- **As `agents-prompt-context` was built (2026-11-28; its design.md "As built", flags 3, 9–20):** the store's decisions are `remote-agent/src/server/core/attachments.rs` (`plan`, `sweep`, `cache_dir`, `attachments_root`, `store_name`; `store::plan` above), its shell `server/core/kept.rs`, and what a session holds of it `acp/reduce/attach.rs`'s `Store` (with `EMBED_LIMIT` and the prompt's parts, `acp/forms.rs`'s `PromptPart::{Text, Embedded, Link}` and `PromptSupport`). A store is `<attachments directory>/<YYYYMMDD-16 hex>` with texts `1`, `2`, … and an `expires` marker; the directory is `SPAWN_AGENT`'s `attachments_dir` else the cache's `attachments`, kept `attachments_keep_days` after the session ends. Brindle numbers uploads by the attach's request id (`upload: req`, unique on the connection), sends them from `ChatModel::send_plan` (`src/agent/model/attach.rs`, `PlanPart::{Link, Upload}`), and its draft items are `Draft::{Link, Text}` keyed by `DraftKey`; a conversation row is `BlockBody::Attached` with `TextState`. `drop_plan(paths, local)` takes whether the agent runs on this machine, and `dropped_text(name, size, read)` refuses a file that isn't UTF-8 (`DropRefusal::NotText`), which is where an image file would branch off. The upload in pieces replaces `Requests::attach` and `AgentLink::attach_text`.
- **One frame holds at most 1 MiB** (`MAX_FRAME_PAYLOAD_SIZE`). The holder's protocol already sends a large blob as begin, chunks and end (`CHECKPOINT_BEGIN`/`CHUNK`/`END`).
- **The holder bounds input queued for a session at 4 MiB** (`INPUT_QUEUE_LIMIT`): more is refused with "the program isn't reading its input; dropped some". An agent's `session/prompt` is one line written to its stdin through that queue, so a line over 4 MiB can be cut. Holder protocol H1 3, which carries pipe sessions, is unreleased.
- **ACP** (v1 and v2 alike): `image { data, mimeType, uri? }` with base64 data, allowed only when the agent's `promptCapabilities.image` is true. Every captured agent's is. `audio` is behind its own capability, which none has.
- **GPUI 0.2.2:** `ClipboardEntry::Image(Image)` with `Image { format: ImageFormat, bytes }` and `ImageFormat::{Png, Jpeg, Webp, Gif, Svg, Bmp, Tiff}`; `img(Arc<Image>)` draws one, decoding it itself. So Brindle needs no image library.
- **Pointer presses** can't be driven from e2e cases (`debug-pointer-hook` is proposal-only).

## Goals / Non-Goals

**Goals:**
- An image on the clipboard or in a file reaches an agent as an image, from any host, and the user sees what they attached.
- One way to send an attachment of any size up to its bound, and one way to read it back.
- Nothing is believed about a file but its bytes: its type comes from them, on both sides.
- An image never enters the log, a queued prompt or a checkpoint; a prompt with images always fits the path to the agent.
- No new dependency, and no image an agent sent is ever drawn.

**Non-Goals:** audio; shrinking, converting or re-encoding; SVG, BMP and TIFF; a preview in the conversation without opening the row; images on `brindle agent -p`; taking screenshots; resuming an interrupted upload; several uploads at once on one connection; drawing an agent's images. No config key and no state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Attachment::Image`, `ImageType` | pure, shared contract | `brindle-transcript` |
| `AGENT_UPLOAD_BEGIN`/`CHUNK`/`END`; `AGENT_READ_ATTACHMENT`'s and `ATTACHMENT`'s offset | wire | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| `image_type(first bytes)`, the base64 encoder | pure | `brindle-transcript` (the type check, used by both sides); `remote-agent/src/acp/base64.rs` (new) |
| The upload's steps and checks (`store::plan` gains begin, piece, end, abandon) | pure | `remote-agent/src/server/core/attachments.rs` |
| Appending to, sealing and reading a range of a kept file | platform seam | `remote-agent/src/server/platform/{unix, windows}` |
| `PromptPart::Image`; `image` content in `session/prompt` | pure | `acp/forms.rs`, `acp/v1/encode.rs` |
| The holder's input bound for pipe sessions | constant | `remote-agent/src/server/holder.rs` |
| `send_plan` in pieces, progress, `paste_plan`, `drop_plan`'s image arm, the image row's state, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| Reading a dropped or pasted image, sending pieces, fetching pieces | shell (background task, reader thread) | `src/agent/mod.rs` |
| The preview and the opened image | shell and component | `src/agent/view.rs`; `brindle-ui`, `ui/src/components/prompt_input.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`): the type check and base64 are a few dozen lines each, and decoding for display is GPUI's.

## Decisions

### 1. The form (`types R1`, `R2`)

```rust
Attachment::Image { id: u64, name: String, kind: ImageType, bytes: u64 }
pub enum ImageType { Png, Jpeg, Gif, Webp }
```

A reference, like `Attachment::Text`: the log, the slot and the checkpoint hold no image bytes. `ImageType` is closed: a type outside it can't be attached, so nothing downstream handles "some other image". Dimensions aren't kept: knowing them would mean decoding on the server.

### 2. The type comes from the bytes (`railway R5`)

`image_type(head: &[u8]) -> Option<ImageType>` (pure, in `brindle-transcript` so both sides use one function): PNG's eight-byte signature, JPEG's `FF D8 FF`, `GIF87a` or `GIF89a`, and `RIFF`…`WEBP`. Brindle uses it on a dropped file and on a pasted image's bytes (the clipboard's stated format is only a hint); the server uses it on an upload's first piece and refuses one whose bytes don't match what `AGENT_UPLOAD_BEGIN` declared (`WrongType`). A name or extension decides nothing. SVG, BMP and TIFF, which the clipboard can hold, match nothing and are refused with the list of types that can be attached: converting them needs an image library, and SVG is a document, not a picture.

### 3. The upload in pieces (`process R4`–`R6`, `R9`)

Client protocol 4 is unreleased and amended (`agents-tool-content`, decision 8). These replace `agents-prompt-context`'s `AGENT_ATTACH`, so there is one way to send an attachment; a text of 1 KiB is a begin, one piece and an end. `AGENT_UPLOAD_BEGIN`, `AGENT_UPLOAD_CHUNK` and `AGENT_UPLOAD_END` are 27, 28 and 29, after `AGENT_READ_ATTACHMENT` (26). `AGENT_ATTACH`'s number, 25, is retired with it and not given to another message, so a number never has two meanings; `ATTACHMENT` stays 44.

- **`AGENT_UPLOAD_BEGIN { req, session, upload, kind, name, bytes }`**, `kind` being `text` or one of the four image types and `bytes` the whole length. Checked before any data: `TooLong` (over `UPLOAD_LIMIT`), `StoreFull`, `DirectoryNotPrivate`, `UploadInProgress` (one at a time on a connection), `NotAnAgentSession`. Answered `OK` or `ERROR`.
- **`AGENT_UPLOAD_CHUNK { session, upload, data }`**: the next bytes, at most what a frame holds, as raw bytes after the two numbers, with no answer. A piece that would pass the declared length, or that belongs to no upload, ends the upload with an `ERROR` and removes its file.
- **`AGENT_UPLOAD_END { req, session, upload }`**: `OK` when exactly the declared bytes arrived, the type check passed, and (for text) the bytes are UTF-8; the file is then made read-only and the upload can be named by a prompt. Otherwise `ERROR` (`Incomplete`, `WrongType`, `NotText`) and the file is removed.
- **Text is kept as sent.** `agents-prompt-context` cleaned an attachment's text of control characters before keeping it. A file an agent reads by its link should be the file the user gave, so a kept text is now exactly the bytes sent, and it is cleaned where it is shown or embedded: by the reducer when it embeds a text in a prompt, and by Brindle when it draws one.
- **An upload belongs to its connection until a prompt takes it**, as before: abandoned when the connection closes or detaches, or when the prompt naming it is refused; its file is removed, complete or not. A core restart in the middle leaves a file nothing references, which the adopting core removes.
- **Reading back:** `AGENT_READ_ATTACHMENT { req, session, id, offset }` answered by `ATTACHMENT { req, id, offset, total, data }`, one frame of raw bytes from `offset`. The client asks for the next piece when one arrives, so nothing queues on the server. Errors as before (`NoSuchAttachment`, a read failure in words).
- Fixtures pin each message and refusal; all join the protocol's fuzzing, with pieces out of order, too long and for unknown uploads.

### 4. Bounds (`railway R5`, `process R9`)

| Bound | Was | Now |
| --- | --- | --- |
| `UPLOAD_LIMIT`: one text attachment | 256 KiB (`ATTACHMENT_TEXT`) | 5 MiB |
| `IMAGE_LIMIT`: one image, a hard maximum | — | 5 MiB |
| `PROMPT_IMAGES`: a prompt's images together | — | 8 MiB |
| `STORE_LIMIT`: kept a session | 16 MiB | 64 MiB |
| `MAX_ATTACHMENTS`, `EMBED_LIMIT` | 8, 64 KiB | unchanged |
| The holder's input queued for a pipe session | 4 MiB | 16 MiB |

- **Why a prompt's images are bounded together.** The agent gets a prompt as one line. With 8 MiB of images (10.7 MiB as base64), eight embedded texts and the prompt's own text at their worst-case escaping, a line is under 13 MiB.
- **Why the holder's bound moves.** That line goes through the holder's input queue, which refuses what passes 4 MiB and would cut a prompt in two. Pipe sessions get their own bound, `PIPE_INPUT_QUEUE_LIMIT` = 16 MiB; a terminal's stays at 4 MiB. It is a number, not a feature (`process R1`): the holder still parses nothing. Only one prompt is in flight for a session at a time (a second waits in the slot), so a line never shares the queue with another. H1 3 is unreleased, so no holder with the smaller bound for pipe sessions is in use; if one is released first, the core must keep a prompt's line under 4 MiB for such a holder, and this decision is reopened.
- **One image has a hard maximum of its own** (the user, 2026-11-25), `IMAGE_LIMIT`, separate from text's `UPLOAD_LIMIT` so the two can move apart. It is checked three times: by Brindle when the image is attached, by the server at `AGENT_UPLOAD_BEGIN` before any byte is sent, and by the server again as pieces arrive. Nothing shrinks an image to fit. 5 MiB is what the model APIs behind these agents commonly accept for one image; a larger one would be kept and then refused by the agent.
- Over any bound an attachment is refused outright, with its size and the limit, as text already is.

### 5. How an image is sent (`acp/forms.rs`, each version's module)

`PromptPart::Image { kind, data: &str }` joins the parts, written by the v1 module as `image` content with its MIME type. The reducer chooses, from `PromptSupport`, which names no ACP version:

- **the agent accepts images:** the image itself. The core's shell reads the kept file and the pure `base64::encode` makes its data, just before the reducer builds the parts;
- **it doesn't:** a link to the kept file (`PromptPart::Link`), which every agent must accept. What such an agent does with a picture is its own business, but it is told it exists and where.

Order is unchanged: the user's text, then the attachments as added. ACP v2 has the same content kind and capability, so `acp/v2/encode.rs` writes the same part.

### 6. Paste, drop and the draft (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **`paste_plan(clipboard) -> Paste::{ Text(String), Image(Draft), Nothing, Refused(reason) }`** (pure): a clipboard with text pastes the text, as today; one with an image and no text attaches the image; one with both pastes the text. `agent_paste_image` ("Paste Image") attaches the clipboard's image whatever else it holds, and says so on the notice line when there is none or its type can't be attached.
- **`drop_plan` gains an image arm:** a dropped file whose first bytes are an image type is `Image(path)` for every host, read on a background task up to `UPLOAD_LIMIT` + 1 bytes (`process R13`). For an agent on this machine other files stay links; for a remote agent, text files stay text and anything else is still refused.
- **The draft** holds an image's bytes in memory until the prompt is sent, bounded by `PROMPT_IMAGES` (`attach` refuses past it: `TooManyImageBytes`).
- **Sending** (`send_plan`) is now, for each kept attachment, a begin, its pieces and an end, then the `AGENT_PROMPT`. The model tracks `Sending { sent, total }` so the zone can show progress; the reader thread feeds the next piece as the connection takes the last, so nothing piles up in Brindle's write queue. Stopping (escape, or the prompt input's stop button) abandons the sequence: the upload is dropped on both sides and the text and draft stay. Any refusal ends it the same way, with the reason beside the input.

### 7. Showing images (`ports R2`)

- **In the prompt input** an image's item shows a small preview in place of its icon, drawn by GPUI's `img` from the draft's bytes; `AttachmentItem` gains an optional image for it. Bytes GPUI can't decode show the plain icon.
- **In the conversation** an image is a row with its name, type and size, closed. A pointer press opens it: the model fetches it piece by piece (`ImageState::{ Closed, Loading { got, total }, Shown(image), Gone(reason), Undrawable }`, `types R1`), then the view draws it scaled to fit the message's width and at most 24 rem tall. Closing a row drops its bytes, so a zone holds only open images.
- **A long text** is opened the same way now that a text can be 5 MiB: the first 256 KiB are fetched and shown, with a line saying how much more there is.
- **Only the user's own images are drawn.** `agents-markdown`'s rule stands: an image in an agent's message is named and never fetched or drawn.

### 8. The dump (`testing R2`)

```
--- attached entry=3 index=0 kind=image id=5 type=png name="screenshot.png" bytes=1048576 state=closed
--- draft index=0 kind=image type=png name="pasted image" bytes=204800
--- prompt status=sending sent=1048576 total=5242880
```

An image's lines have its type and size and never its bytes; its row state is `closed`, `loading`, `shown`, `gone` or `undrawable`. The `--- prompt` line says `sending` with the bytes sent and the total while a send is in progress. `--wait-for` matches them. The debug hooks can't paste or drop, so a hidden debug step attaches a file as if dropped (`--attach-file PATH`, a usage error when the file can't be read), as `--modality` stands in for a pointer press.

### 9. Failures (`railway R6`, `R8`)

- A type that can't be attached, or bytes over a bound: refused when attached, on the notice line, naming the file, its size and the limit or the types accepted.
- The server refuses an upload or the prompt: the text and the draft stay, the reason beside the input; the server keeps nothing of it.
- The connection drops mid-send: the send ends as a refused prompt does once the zone knows; after the reconnect the user sends again.
- A kept image is missing when its prompt is sent: left out, with a notice, as a text is.
- A kept image can't be read or drawn when its row is opened: the row says so.
- The agent rejects an image (too large for its model, a type it won't take): its error ends the turn as any prompt error does.

### 10. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

As for text: kept images are files and their references are in the checkpoint, so a waiting or held prompt keeps them across a core restart and an upgrade. An upload in flight at a core restart is lost: its partial file is removed on adoption, and Brindle's send fails with the connection's error. A Brindle crash loses the draft and leaves no upload behind. The holder gains nothing but a larger bound.

### 11. The built agent stack (refresh, 2026-11-26)

`agents-session-recovery`, `agents-login`, `agents-zone-attention`, `agents-zone-theme-scope` and `agents-log-paging` are built since this was written. `agents-prompt-context`'s decisions 4 and 13 say what they change for attachments of every kind; for images:

- **A continued agent** gives its own `PromptSupport`: a held prompt's image goes as an image to an agent that takes images and as a link to one that doesn't, whichever agent first had it. A session whose agent stopped keeps its images, which can still be opened.
- **Progress doesn't move on a clock.** The send's progress and an opening image's (`Loading { got, total }`) are the share done, drawn when a piece is taken or arrives. Nothing spins or shimmers, so `agents-zone-attention`'s motion clock isn't run and "Motion only while an agent works" is untouched.
- **A send is not a turn.** While pieces are sent the prompt's status is `sending` and the turn is still idle: the working row, the tab's spinner and the attention state don't show, and escape and the stop button stop the send (decision 6), not the agent.
- **The checkpoint** is format 2; nothing of an image or an upload is in it but the references, as for text.
- **The holder's bound** is still one constant for every session (`INPUT_QUEUE_LIMIT`, 4 MiB, checked at `remote-agent/src/server/holder.rs`); H1 3 gained `CHECKPOINT_APPEND` and is still unreleased, so decision 4's larger bound for pipe sessions stands.
- **Drawing.** The preview, the image row and the opened image are parts of the zone, in the zone's token set.
- **`agents-client-state-fixes` (decision 5):** a prompt whose answer was lost with the link is settled by the reconnect's replay through the pure `taken` (`src/agent/model.rs`), which compares the cleaned text with the slot and the later user entries. Once a prompt carries images, `PromptSent` must record them and `taken` must compare the attachment list as well as the text.

### 12. Tests

- **Unit:** `image_type` over each type's signature, truncated heads and lookalikes; `base64::encode` against known vectors and every length modulo 3; `store::plan` for begin, pieces, end, each refusal, abandon and adoption with a partial file; the reducer's image or link by support, the order of parts and the line's size at the bounds; `paste_plan`, `drop_plan`, `attach` at `PROMPT_IMAGES`, `send_plan` with progress and a stop, `ImageState`'s transitions; the dump lines.
- **Protocol:** fixtures and fuzzing (decision 3).
- **Real roles** (the fake ACP agent, which records each prompt's content kinds, sizes and a hash): a 5 MiB image arrives with the same hash; an agent without image support gets a link to a file with that hash; 8 MiB of images in one prompt reaches a slow-reading agent whole; a wrong type, an incomplete upload and a closed connection each leave no file; reading back returns the same bytes; a 3 MiB text is kept exactly as sent.
- **E2E:** `agent-image-attach` (through `--attach-file`), `agent-image-bounds`, `agent-image-queue`, `agent-image-restore`, `agent-text-large`.
- **Manual:** pasting an image from the clipboard and dropping image files, on Linux and on Windows, for a local and a remote agent; the preview; opening an image row; a real agent describing a pasted screenshot.

## The user's decisions

None is open. The user settled each on 2026-11-25, one at a time: the holder's input bound for agent sessions rises to 16 MiB, and one image has a hard maximum of its own (decision 4). The numbers are accepted: 5 MiB as one image's hard maximum, 5 MiB a text attachment, 8 MiB of images a prompt, 64 MiB kept a session.

An agent that doesn't accept images gets a link to the kept file (decision 5).

A dropped image file is attached as an image for a local agent too (decision 6).

A clipboard with both text and an image pastes the text, with Paste Image for the picture (decision 6).

Kept text is stored exactly as sent and cleaned where it is shown or embedded, which changes `agents-prompt-context`'s cleaning before keeping (decision 3).

A hidden `--attach-file` debug step lets e2e cases attach an image (decision 8).

## Risks / Trade-offs

- A prompt line of up to 13 MiB goes to the agent's stdin → the holder's bound is raised for it, only one prompt is in flight, and a real-role test sends the largest to a slow reader.
- An agent's model may refuse an image the bounds allow → its error ends the turn with its own words; Brindle doesn't shrink images.
- An image is read whole into memory to send and to show → at most 8 MiB a draft, and only open rows hold theirs.
- GPUI decodes the images shown → only ones the user attached, never an agent's.
- One upload at a time on a connection → attachments of a prompt go in turn; with two zones on one host, the second waits for the first's upload.

## Migration Plan

Nothing to migrate: the messages this replaces are unreleased. `agents-prompt-context`'s e2e cases that attach text keep passing through the new sequence.
