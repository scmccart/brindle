# Proposal

**Depends on:** `agents-prompt-context` (a prompt's attachments, the store that keeps them beside the log, their row in the prompt input and the conversation), implemented and archived.
**Next:** `agents-plans`, still to be proposed.

## Why

Much of what a user wants to show a coding agent is a picture: a screenshot of a broken layout, a diagram, an error dialog. Every agent we captured says it accepts images in a prompt (`promptCapabilities.image`). With `agents-prompt-context` a prompt carries links and text, but an image on the clipboard can't be pasted into an agent's prompt, and a dropped image file is refused for an agent on another host.

Images weren't part of that change because they don't fit its transport: one message between Brindle and the terminal server holds at most 1 MiB, and a screenshot is often several. The same limit is why that change refuses text over 256 KiB outright. Both need an upload that spans several messages.

## What Changes

- **An upload in pieces.** Brindle sends an attachment to the terminal server as a start, pieces of at most one message each, and an end, and reads one back the same way, a piece at a time. It replaces `agents-prompt-context`'s one-message attach, so there is one way to send an attachment.
- **Text attachments grow.** The 256 KiB limit, which was only what one message carries, becomes 5 MiB. Text over 64 KiB was already sent to the agent as a link to the kept file, so nothing else changes for it.
- **Images are a third kind of attachment:** PNG, JPEG, GIF and WebP, up to 5 MiB each and 8 MiB a prompt, kept in the same store as text. The type is decided from the file's first bytes, by Brindle and again by the server, never from a name.
- **Pasting and dropping.** Pasting into the prompt input with an image on the clipboard attaches it; `agent_paste_image` does so by name. An image file dropped on an agent zone is attached as an image, whichever host the agent runs on.
- **How the agent gets it.** An agent that accepts images receives the image itself, as ACP's image content. One that doesn't receives a link to the kept file, so no agent is left out.
- **Showing it.** An image waiting in the prompt input shows a small preview. In the conversation it is a row with its name, type and size that opens, on a pointer press, to the image, fetched from the server then. Brindle draws only images the user attached; an image in an agent's message is still never drawn.
- **Sending shows progress** and can be stopped: a large image over a slow connection takes time.
- **The store's bound** rises from 16 MiB to 64 MiB a session, now that it is on disk.
- **The holder's bound on input queued for an agent session** rises from 4 MiB to 16 MiB, so that one prompt with its images always fits.
- **Client protocol 4 is amended**, as for the rest of the series.

Not in this change: audio, which no captured agent accepts; shrinking, converting or re-encoding an image; SVG, BMP and TIFF; previews in the conversation without opening a row; images in a first prompt from `brindle agent -p`; screenshots taken by Brindle; drawing images an agent sends.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: "Prompt attachments are bounded" and "A client can read an attachment's text" change; new requirements for uploads in pieces, images in a prompt, their types and how they are sent.
- `agent-zones`: "Dropping files on an agent zone" and "Attachment limits are said when attaching" change; new requirements for pasting an image, previews, opening an image, and sending with progress.
- `ui-components`: an attachment item's preview.
- `command-line`: the dump's lines for images and for a send in progress, and a debug step that attaches a file.

## Impact

- **`brindle-remote-protocol`** (`remote-protocol/`): `AGENT_UPLOAD_BEGIN`, `AGENT_UPLOAD_CHUNK`, `AGENT_UPLOAD_END` in place of `AGENT_ATTACH`; `AGENT_READ_ATTACHMENT` and `ATTACHMENT` gain an offset; fixtures and fuzzing.
- **`brindle-transcript`** (`transcript/`): `Attachment::Image` and `ImageType`; new fixtures.
- **`brindle-remote-agent`**: the store writing an upload as it arrives; a pure type check and base64 encoder; `acp/v1/encode.rs` writing `image` content; the holder's input bound for pipe sessions.
- **`brindle`** (`src/agent/`): the upload and download sequences, paste, drop, previews, the conversation's image rows; an action in `src/actions.rs`.
- **`brindle-ui`** (`ui/`): an attachment item's preview, drawn with GPUI's own image element. No new dependency anywhere.
- **Tests:** real-role tests through the fake ACP agent, including an image's bytes arriving unchanged; new `agent-image-*` e2e cases; manual checks for paste and drop.
