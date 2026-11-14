# Proposal

**Status: proposal stage.** This change needs phase 4 of the terminal-server series (`terminal-server-remote`), whose `brindle-remote --attach` relay is the pocket client's transport. It has no specs or tasks yet.

## Why

Brindle's sessions now outlive the app, but they can only be reached from a desktop. A pocket device that attaches to them would cover the common case away from the desk: glancing at a running session (a build, a long job, an agent waiting for input) and, now and then, working in one for longer. The hardware is bought: a LilyGO T-Deck MAX for the device itself and a LilyGO T-Display-S3 as the dev board.

The pocket client would also be the second client of the `terminal-server-remote` relay, which tests that design beyond the desktop app.

## What Changes

- **A pocket client firmware** for the T-Deck MAX (ESP32-S3, 320×240 e-paper, keyboard), in Rust on ESP-IDF with std. It lists a host's sessions and attaches to one.
- **Transport:** SSH to a host, running `brindle-remote --attach`, speaking client protocol 1 (`LIST`, `ATTACH`, the `REPLAY_START`/`DATA`/`REPLAY_END` replay). Sessions survive the device dropping off the network, and reconnecting restores the screen.
- **Networks:** Wi-Fi first, with a phone hotspot when away. Tailscale reaches hosts that aren't on the same network. LTE (the board's A7682E) is optional and later.
- **E-ink rendering:** partial refresh for changed rows, with periodic full refreshes against ghosting. E-ink is acceptable for this use.
- **A shared core** (protocol client, emulator, screen model) that runs on desktop Linux in a simulator window, on the T-Display-S3, and on the T-Deck MAX, behind `Input` and `Display` traits.
- **Shared crates where they fit:** `brindle-remote-protocol` for the wire format, and `brindle-term-core` for the emulator if it builds for `xtensa-esp32s3-espidf`.

## Out of scope

LoRa and Meshtastic (the SX1262 radio), audio (ES8311), GPS.

## Capabilities

### New Capabilities
- `pocket-client`: listing and attaching to sessions from the device, reconnect and replay, the e-paper renderer, keyboard input, and host keys and credentials on the device.

### Modified Capabilities
- `terminal-server`: possibly none, if the relay serves the device unchanged. Anything the device needs from the relay (liveness, a smaller replay, a resize policy for a 53×30 client) lands in `terminal-server-remote` or here.

## Impact

- A new firmware crate (or repository; see the design's points to settle).
- `remote-protocol/`: platform-specific modules gated so the crate builds for `espidf`.
- `term-core/`: possibly a feature flag, depending on whether `alacritty_terminal` builds for `espidf`.
- Depends on `terminal-server-remote` and shares its open point on liveness over a relay.
