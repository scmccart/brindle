# Design

**Proposal-stage notes, not a settled design.** They carry forward what phase 1 decided and found. Phase 1's full design is in `openspec/changes/archive/2026-11-07-terminal-server/design.md` once PR #22 merges.

## Context

- **Protocol 1 already has** `SPAWN { policy }` with `kill` and `detach`, `ATTACH` with `REPLAY_START`/`REPLAY_END`, `DETACH`, `LIST` (`SESSIONS`), and reconnect by `client_token`, which returns sessions without a client to their owner.
- **Brindle's client** (`remote::local`) already applies replays quietly: bells, OSC 9/777 notices and status notices are dropped between the markers, while the state still updates.
- **The core keeps each session's owner and policy** in the `meta` blob the holder stores. H1 can't update `meta` after `SPAWN`.

## Close behaviour (from phase 1's exploration)

| Event | kill | detach |
|---|---|---|
| close a zone, tab or window | ends the session | session keeps running, listed under its host |
| quit Brindle, or a crash | ends the session | tab reattaches on next launch |
| connection drop | ends after the reconnect grace | stays; Brindle reconnects |
| the shell exits | tab closes | tab closes; the server forgets the session |

`ask` prompts on close and quit. A crash can't ask, so it behaves as `detach`. That is one of the open questions below.

## Gaps phase 1 left for this change

- **A `kill` session explicitly `DETACH`ed** has no client, and no policy ever ends it. Either `DETACH` is refused for `kill` sessions, or it switches the policy to `detach`.
- **Ownership after a core restart:** an attaching client becomes the owner only inside the running core, because H1 can't update the `meta` the holder keeps. After a core restart, the original owner gets the session back. A new H1 message, for example `SET_META { session, meta }`, fixes this, and means bumping H1 to version 2. The core has to keep speaking H1 version 1 to older holders.

## Risks known from phase 1

- **The desktop's app scope may end the holders when Brindle exits:** GNOME and KDE start apps in a systemd scope. Check what happens to a detached holder when Brindle's scope ends. If needed, start holders through `systemd-run --user` or a user unit.
- **Orphans accumulate** once sessions outlive Brindle. `brindle-remote list` and `kill` exist. Decide whether the palette should also surface old detached sessions.

## Open questions (to settle before specs)

- The option's name and default: does `kill` stay the default?
- Whether `ask` covers a quit with many tabs as one prompt or one per tab.
- What a crash does under `ask`.
- Whether restore also reattaches sessions that another Brindle process started.
- Whether to add `brindle --attach <id>`.
