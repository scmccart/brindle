# Proposal

**Depends on:** `agents-prompt-context` (a prompt's attachments, their row in the prompt input, their life in the queue slot and the log), implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. The user split it from `agents-prompt-context` on 2026-11-25, to be defined as a full change right after it.

## Why

Much of what a user wants to show a coding agent is a picture: a screenshot of a broken layout, a diagram, an error dialog. Every agent we captured says it accepts images in a prompt (`promptCapabilities.image`). With `agents-prompt-context` a prompt carries links and text, but an image on the clipboard can't be pasted into an agent's prompt, and a dropped image file is refused for an agent on another host.

Images weren't part of that change because they don't fit its transport: one message between Brindle and the terminal server holds at most 1 MiB, a screenshot is often several, and a prompt waiting in the queue must keep its bytes across a restart.

## What Changes

- A prompt's attachments gain a third kind, an image, which the server sends to the agent as an ACP `image` content block.
- Pasting with an image on the clipboard, and dropping an image file on an agent zone, attach it.
- Brindle sends an image's bytes to the terminal server in pieces before the prompt that names them.

Open questions, to answer before a design:
1. **The upload:** new client messages that carry an image in pieces of at most one frame, how a prompt names an upload, what bounds uploads in flight, and what a dropped connection or a core restart does to one that is half sent.
2. **Limits:** the largest image, how many a prompt may hold, and their total; and whether Brindle shrinks or re-encodes a large image (which needs an image library on the app's dependency list) or refuses it.
3. **Formats:** which are accepted (PNG, JPEG, GIF, WebP are what GPUI's clipboard can give), and how the type is decided for a dropped file.
4. **The queue slot.** A waiting or held prompt must keep its images until it is sent or discarded, across restarts and upgrades. Whether they live in the session's checkpoint (which the holder accepts up to 64 MiB and which is rewritten whole), or beside it.
5. **The log.** Whether the conversation keeps an image's bytes, a small preview, or only a note (its type and size), given the log's 16 MiB bound and that every attach replays it.
6. **Showing it:** a thumbnail in the prompt input and the conversation needs an image drawn in an agent zone, which nothing does today; or a named item only.
7. **An agent without image support:** refused when attaching, with the reason.
8. **Paste:** which key or action pastes an image, and what a clipboard holding both text and an image does.
9. **Audio,** which no captured agent accepts: left out.

## What the full changes already say

- `agents-prompt-context`, design decision 1: `Attachment` is an enum that an `Image` kind joins, since protocol 4 is unreleased. Decision 2: a queue slot must fit one frame, which is why its text is bounded at 384 KiB (question 4). Decision 5: `PromptSupport { embedded, images }` is already read from `initialize` and checkpointed (question 7).
- `agents-prompt-context`, `agent-zones` requirement "Dropping files on an agent zone": a file that isn't text is refused for an agent on another host. This change modifies it for images.
- `agents-sessions`: the log's 16 MiB bound and the 64 MiB checkpoint (questions 4 and 5); the holder's `CHECKPOINT_BEGIN`/`CHUNK`/`END` is the existing example of a payload sent in pieces.
- `agents-markdown`: images in an agent's message are never fetched or drawn, only named (question 6 is about the user's own images, which are different: the user chose them).

## Capabilities

### New Capabilities
- None expected.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): images in a prompt, the upload, and their bounds.
- `agent-zones` (added by `agents-profiles`): pasting and dropping images, and how they show.
- `terminal-server`: the upload messages.
- `ui-components`: only if a thumbnail is drawn.

## Impact

- `remote-protocol/`: upload messages, with fixtures and fuzzing.
- `transcript/`: the `Image` attachment and what the log keeps of it.
- `remote-agent/src/acp/` and the core: holding uploads, the checkpoint, and each version's `image` content block.
- `src/agent/`: paste, drop, the upload, and showing an image.
- `Cargo.lock`, only if an image is shrunk or decoded for a thumbnail.
