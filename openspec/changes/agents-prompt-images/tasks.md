# Tasks

Before starting, check that `agents-prompt-context` is implemented and archived (the user's decisions are settled; design.md lists them):
- From it: `Attachment::{Link, Text}`, the store and `store::plan`, `AGENT_ATTACH`, `AGENT_READ_ATTACHMENT`, `ATTACHMENT`, `PromptSupport`, `PromptPart`, the draft, `send_plan`, `drop_plan`, the attachment rows and `TextState`, `PromptInput::set_attachments`, `BRINDLE_CACHE_DIR` in the harnesses.
- Its spec deltas modify requirements that `agents-prompt-context` adds, so `openspec validate` notes that an archive would refuse them until that change's specs are synced. That is the order of the stack, not a fault.
- Client protocol 4 and holder protocol H1 3 are still unreleased. If either has shipped, stop and raise design decisions 3 and 4.
- Refreshed on 2026-11-26 for the built stack: design decision 11 holds what `agents-session-recovery`, `agents-login`, `agents-zone-attention`, `agents-zone-theme-scope` and `agents-log-paging` change. The function and file names in the tasks are from before those changes; check each against the code.

## 1. Shared contracts

- [ ] 1.1 Add `Attachment::Image` and `ImageType` to `brindle-transcript`, and the pure `image_type(head)`; verify with unit tests that the forms round-trip, each type's signature is recognised, a truncated head and lookalikes (`RIFF` that isn't WebP, `GIF8` alone, an SVG, a BMP, a TIFF) aren't, new `tests/fixtures/v4/` files are written once, and no existing fixture changed.
- [ ] 1.2 In `brindle-remote-protocol`, replace `AGENT_ATTACH` with `AGENT_UPLOAD_BEGIN` (taking its number, 25), `AGENT_UPLOAD_CHUNK` (27) and `AGENT_UPLOAD_END` (28), and give `AGENT_READ_ATTACHMENT` its `offset` and `ATTACHMENT` its `offset`, `total` and raw bytes; verify with fixtures for each message and refusal, a piece of the largest size fitting one frame, decoding errors, and fuzzing that includes pieces out of order, past the stated length and for unknown uploads.
- [ ] 1.3 Add `PromptPart::Image` to `acp/forms.rs` and to `Dialect::encode`; verify `cargo test -p brindle --test architecture` passes.
- [ ] 1.4 Add the store's platform items for appending to an upload's file, sealing it read-only, and reading a range of a kept file, under the same names on every platform; verify with tests on each for a sealed file refusing writes and a range read at the start, the middle and past the end.

## 2. Pure code in the server

- [ ] 2.1 Extend `store::plan` with begin, piece, end and abandon, the bounds (`UPLOAD_LIMIT`, `PROMPT_IMAGES`, `STORE_LIMIT` at 64 MiB) and each refusal (`TooLong`, `StoreFull`, `UploadInProgress`, `Incomplete`, `WrongType`, `NotText`); verify with unit tests for each, a piece passing the stated length, an end with too few bytes, a connection closing mid-upload, adoption with a partial file, and UTF-8 checked across a piece boundary.
- [ ] 2.2 Add `acp/base64.rs` (`encode`); verify against known vectors and every length modulo 3; add it to `PURE_MODULES`.
- [ ] 2.3 Write `PromptPart::Image` as `image` content in `acp/v1/encode.rs`; verify by pinning the line for each type.
- [ ] 2.4 Extend the reducer: the image bound of a prompt before anything is logged or queued; an image as `PromptPart::Image` when `PromptSupport.images`, else a link to its kept file; a kept text cleaned when it is embedded, since it is no longer cleaned when kept; verify with unit tests for each, the order of parts, and a prompt at every bound producing a line under 13 MiB.
- [ ] 2.5 Give pipe sessions their own input bound in `server/holder.rs` (`PIPE_INPUT_QUEUE_LIMIT` = 16 MiB), a terminal's staying at 4 MiB; verify with a holder test that a 13 MiB write to a pipe session whose program reads slowly arrives whole, and that a PTY session still refuses past 4 MiB.

## 3. Server roles

- [ ] 3.1 Handle the upload and the ranged read in the core, writing each piece as it arrives through `server/platform/` and reading and encoding a prompt's images just before the reducer builds its parts; teach the fake ACP agent to record each prompt's content kinds, sizes and a hash to a file a test reads; verify with real-role tests: a 5 MiB image arrives with the same hash; an agent without image support gets a link to a file with that hash; 8 MiB of images in one prompt reaches a slow-reading agent whole; a 3 MiB text is kept exactly as sent and linked; reading back in pieces returns the same bytes.
- [ ] 3.2 Add the failure and life-cycle cases: a wrong type, an incomplete upload, a second upload on one connection and a closed connection each leave no file; a waiting prompt keeps its image across a core restart and an upgrade; an upload in flight at a core restart leaves nothing after adoption.
- [ ] 3.3 Check the static musl helper still builds and `Cargo.lock` is unchanged.

## 4. Brindle's model

- [ ] 4.1 Extend the client and `src/agent/shared.rs` for the new messages and the image reference; verify with unit tests for each message's encoding and a replay holding an image.
- [ ] 4.2 Add `paste_plan`, `drop_plan`'s image arm (by `image_type`, for every host) and `attach`'s `TooManyImageBytes`; verify with unit tests for a clipboard with text, an image, both and neither, each clipboard format, a file whose name and bytes disagree, and the image bound at its edge.
- [ ] 4.3 Rework `send_plan` into begin, pieces and end per kept attachment, with `Sending { sent, total }`, the next piece fed as the last is taken, and stop; verify with unit tests for the sequence, progress, a refusal at each step, a stop mid-piece, and text and draft kept after each.
- [ ] 4.4 Add the image row with `ImageState::{Closed, Loading, Shown, Gone, Undrawable}` and the ranged fetch, and the first-256-KiB view of a long text with its line; verify with unit tests for each transition, a row closed while loading, an answer for a closed row, and the text view of a 3 MiB attachment.
- [ ] 4.5 Extend the dump with image lines and `status=sending`; verify with unit tests for each form.

## 5. brindle-ui

- [ ] 5.1 Give `AttachmentItem` an optional picture drawn with GPUI's `img` in the icon's place, falling back to the icon; verify with component tests for both, and that the item is still no keyboard stop; add it to the gallery.

## 6. The zone

- [ ] 6.1 Wire paste in `src/agent/view.rs` through `paste_plan`, and add `agent_paste_image` ("Paste Image") to `ACTIONS`, handled only by `AgentView`; verify with a unit test for the palette's listing.
- [ ] 6.2 Read dropped and pasted images on a background task (at most `UPLOAD_LIMIT` + 1 bytes for a file), attach or refuse by the plans, and show the preview. Nothing is read or decoded on the UI thread beyond GPUI's own image element (`process R13`, `R14`).
- [ ] 6.3 Run the send on the reader thread with progress in the prompt input; escape and the stop button stop it. The progress is drawn when it changes and runs no motion clock, and a send shows no working row or tab spinner (design decision 11); verify in the e2e case for a send that the dump has `--- motion stopped` and no `--- working` line while `status=sending`.
- [ ] 6.4 Draw the conversation's image rows and the long-text line.
- [ ] 6.5 Add the hidden `--attach-file PATH` debug step (a usage error for an unreadable path; a note in the dump in a terminal zone); verify with unit tests for the parser and the usage error.

## 7. End-to-end cases (fake ACP agent)

- [ ] 7.1 `agent-image-attach`: `--attach-file` with a small PNG gives a draft image; sending it shows the user message's image reference, and the fake agent's record holds image content with the file's hash.
- [ ] 7.2 `agent-image-bounds`: a file over 5 MiB, a third image passing 8 MiB, and a TIFF are each refused on the notice line; a text file named `.png` is attached as text for a remote agent (in `remote-agent-ssh`).
- [ ] 7.3 `agent-image-queue`: an image sent during a turn waits with its prompt and arrives when the turn ends; a discarded one leaves no file.
- [ ] 7.4 `agent-image-restore`: quit with a sent image and a waiting one; the relaunched zone shows both references and the files are still under the case's `BRINDLE_CACHE_DIR`.
- [ ] 7.5 `agent-text-large`: a 1 MiB command output is attached whole and reaches the fake agent as a link to a file of that size.
- [ ] 7.6 Run the cases on Windows (one `windows-agent-*` case attaching a PNG) and in the archive jobs.

## 8. Docs and checks

- [ ] 8.1 Update `docs/code-map.md`, `README.md` (pasting and dropping images, the limits, where they are kept), and `CLAUDE.md`'s debug steps, dump description and manual checks.
- [ ] 8.2 Tell `agents-acp-v2` what it inherits: its v2 module writes the same `image` part.
- [ ] 8.3 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate agents-prompt-images --strict`.
- [ ] 8.4 Manual checks for the user, with `scripts/run-throwaway.sh` and real agent profiles: pasting a screenshot from the clipboard, and a clipboard holding text and an image; dropping PNG, JPEG, GIF and WebP files on a local and a remote agent's zone, on Linux and on Windows; the preview; a pointer press opening and closing an image row; stopping a large send over SSH; a real agent describing a pasted screenshot.
