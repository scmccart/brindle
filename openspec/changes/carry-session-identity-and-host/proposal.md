# Proposal

## Why

Brindle's client names a shown session by its bare id after a reconnect, and it opens the config editor on whatever endpoint the default profile targets. Both break `types` rules the code otherwise keeps (audit 2026-11-16, findings F3 and F5; remediation plan, change 2):
- **F3, `types R3`:** session ids restart at 1 in a fresh terminal server core. When no holder survived (one crashed or was killed, or a remote host rebooted), a new session can take a shown session's id. Today:
  - the dead tab can stay frozen with no "session lost" line;
  - an attach cut off by the drop can be answered with another session's screen;
  - a dead tab keeps acting on its id. Its keys, its resizes and its close go to whatever session of this Brindle now has that id, so closing the dead tab ends the new tab's program. This last case needs no second client: a new tab opened after the server came back is enough.
- **F5, `types R8`:** ctrl-, runs the local editor command on the local config path, but in a tab of the default profile's endpoint. With a remote default profile, that runs on the remote host.

## What Changes

- The client keeps each shown session's key beside it, decoded once from the wire's string (`SessionKey`; `types R7`).
- After a reconnect, a shown session counts as still there only when the listing has its id **with its key**. A session whose id the endpoint now gives another session is shown lost.
- An attach cut off by a dropped link is sent again only to a session listed with its key. Otherwise it fails, as one whose session isn't listed does now.
- A session handle (`Running`) acts only while its connection shows that same session, by id and key. A lost or exited tab's input, resize, reparent, layout, detach and close never reach a session that later took its id.
- The listed size is decoded where the listing is read (`Option<(u16, u16)>`), not tested as `cols > 0 && rows > 0` where it is used (`types R7`). The attach request says whether it is a listed or a recorded attach with an enum, instead of a `recorded: bool` beside a 0x0 size.
- ctrl-, opens the config editor in a tab on this machine (`Host::Local`), whatever endpoint the default profile targets. The tab takes the default profile when it is a local one, else the first local profile.
- No wire, protocol, config-key or state-file change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `remote-connections`: "Sessions come back after a reconnect". A session counts as the endpoint's only by id and key, so one whose id was given to another session is shown lost, and a cut-off attach is never answered with another session.
- `terminal-emulation`: "A session lost for good is reported in the tab". A lost or exited tab never acts on a session that later got its id. The plan named only `remote-connections` and `configuration`, but the most reachable form of the defect is local (see design, decision 4).
- `configuration`: "Live reload". ctrl-, opens the editor on this machine.

## Impact

- **Code:**
  - `src/remote/identity.rs` (new, pure): `SessionKey` and `ListedSession`.
  - `src/remote/local/connection.rs`:
    - `State.sessions` becomes `Shown` entries, which also absorb `parents` and `foreground`;
    - the after-loss prune, `carried_attaches`, `attached_by_replay`'s insert, `Running`, `AttachRequest`, `Waiting::Attach`.
  - `src/remote/local/session.rs`: `requested`, `id_and_key`.
  - `src/terminal/mod.rs`: `server_session`.
  - `src/zone_view.rs`: `zone_server_session` writes the state file's form.
  - `src/workspace.rs`:
    - `open_config`, plus a new `launch_on` split out of `launch`;
    - `attached_terminal` and `restored_terminal`, which build attach requests.
- **Tests:**
  - unit tables in `identity.rs`, `connection.rs` and `workspace.rs`;
  - `fake::link` tests in `connection.rs`;
  - the existing connection tests' listings now carry keys;
  - two e2e cases: `local-server-reused-id` and `reload-config-editor-local`.
- **Docs:** CLAUDE.md's "Brindle's client" paragraph.
- **Audit entries closed:**
  - b25 #13, #14 and #15;
  - b07 #24;
  - b07 #23 (the same size sentinel, in `workspace.rs`), folded in because it is decoded by the same type.
- **Not in scope:**
  - b07 #20, the sessions palette's kill by bare id: `KILL` carries no key, so a key-checked kill is a protocol change. The plan files it under change 13.
  - The readers of `ServerSession.key` and `SessionInfo.key` outside the client (`reattach`, `layout`, `picker`, `zone_view`'s layout keys). They already refuse an empty key, and are brought into line when next changed.
- **Overlap with changes in flight:**
  - `add-blocks` modifies `configuration` "Behaviour settings" and `terminal-emulation` "Wheel scrolling", not the requirements modified here.
  - `port-to-macos` maps ctrl-, to cmd-, (its `macos-platform` spec) and doesn't touch what the shortcut opens.
  - `add-pocket-client` and `update-dependencies` don't overlap.
- **Overlap with later changes in the same plan:**
  - Change 9 (`bound-queues-and-contain-panics`) modifies the same two requirements, `remote-connections` "Sessions come back after a reconnect" and `terminal-emulation` "A session lost for good is reported in the tab". It must copy their text as this change leaves it.
  - Change 12 (`surface-swallowed-failures`) builds on the key and listing types added here.
  - Change 13 (`extract-decisions-into-models`) restructures `Pending`'s flags, which this change leaves alone.
