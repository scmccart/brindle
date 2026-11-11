# Proposal

**Status: outline only.** Follows `terminal-server-on-close` (phase 2), which needs to land first. No specs, design or tasks yet; settle the open questions below before writing them.

## Why

After phase 2, local terminals live in the terminal server, but where they sat (window, tab, split, focus, titles) lives in Brindle's own `session.toml`. The two can disagree:
- Sessions the state file doesn't know about (a closed tab, a `--no-restore` or `-p` launch, another Brindle's tabs) come back as flat tabs in the first window, losing their splits and windows.
- The file is per machine and per process lock, while the server is per user and per server directory, and in phase 4 per remote host. A layout can't follow its sessions to another Brindle, or to a remote host's server.
- Restore has to match file records to live sessions (id plus key), with a respawn fallback for every mismatch.

If the server keeps the layout of its sessions, reattaching rebuilds the windows and splits from the same place the programs live.

## What Changes

To be decided. The likely shape:

- **The core stores a layout** for its sessions: windows (size, maximized), tabs (order, active, manual title, profile name) and each tab's split tree (axis, ratio, focused zone), with sessions as the leaves.
- **Brindle updates it as it changes,** the way it writes `session.toml` today, through new protocol-1 messages or an additive version bump.
- **The layout survives core restarts and upgrades,** carried in the holders' checkpoints or meta, or a core-owned file in the server directory.
- **Launch-time reattach rebuilds windows and splits** from the layout for every detached session, not just those in `session.toml`.
- **`session.toml` keeps what the server can't hold:** remote tabs until phase 4, Windows tabs until phase 3, and respawn records for after a reboot.

## Open questions

- **Where the layout lives:** in the core's memory and holder meta (it dies when the last session ends), in a file in the server directory (it survives a reboot, but then needs a respawn path), or both?
- **One layout or one per client:** what happens when two Brindles show sessions from the same server at once, and one quits?
- **Granularity:** whole windows, or only the tab and split tree around each session, with Brindle choosing windows?
- **Session-less tabs:** whether remote tabs and Windows tabs move into the server layout too, as placeholders the client re-opens, so `session.toml` can go away entirely once phases 3 and 4 land.
- **Partial layouts:** what a split tab shows when one of its sessions has ended.
- **Interaction with phase 4:** one layout per host server, or a client-side merge of several hosts' layouts.

## Capabilities

To be decided. Likely `terminal-server` (layout storage and messages) and `session-restore` (where restore reads from), and possibly `tabs`, `zones` and `command-palette`.

## Impact

- `remote-protocol`: layout messages.
- `remote-agent`: layout storage in the core, and its persistence.
- `src/session.rs`, `workspace.rs`, `zone_view.rs`, `main.rs`: recording to the server, and restoring from it.
