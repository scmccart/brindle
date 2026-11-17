# Design

## Context

See proposal.md, "Why". The audit is `docs/architecture/audits/2026-11-16.md`: findings F3 and F5, with the instances in `2026-11-16/b25.md` #13–#15 and `b07.md` #23–#24. The plan is `2026-11-16-remediation.md`, change 2, and its section "Finding 3 traced".

### Brindle's client today

All of this is in `src/remote/local/connection.rs`.
- **What it holds for a shown session:**
  - `State.sessions: HashMap<SessionId, Sink>`, plus `parents` and `foreground`, all keyed by the bare id (`:275-311`);
  - `Running { id, pid, key: String, .. }` (`:1858`), whose key is "empty from a core that predates keys" (`:1878-1882`).
- **The after-loss prune** (`:1217-1233`):
  - it builds `kept` from every listed session that has *any* client (`SessionInfo.attached` is `!detached`, `core/manage.rs:45`);
  - then `missing` (`:1853`) takes the rest out.
- **`carried_attaches`** (`:1812-1849`):
  - a recorded attach is checked by key (`check: Option<String>`, with `!key.is_empty()` as the "no key" test);
  - a non-recorded attach carried over a lost link is matched by bare id (arm `(None, Some(_))`);
  - the listed size is read through `info.cols > 0 && info.rows > 0`.
- **`attached_by_replay`** (`:1277-1290`) matches a reconnect's `REPLAY_START` (no `req`) to a waiting attach by id.
- **`Running`'s methods** act on `self.id` with no check that the connection still shows that session:
  - `input`, `resize`, `reparent`, `foreground`, `set_layout`;
  - `close` and `detach`, through `Connection::forget` (`:728`), which removes whatever entry has the id.
- **`AttachRequest`** (`:198-208`) is `{ session, key: String, cols, rows, recorded: bool }`, and a recorded attach is built with `cols: 0, rows: 0` (`src/workspace.rs:816-822`).

### Where ids are reused (plan, "Finding 3 traced", confirmed)

- A new core keeps adopted sessions' ids (`core/holders.rs:368-423`).
- So an id is reused only after a fresh core started when no holder that held it survived.
- The core gates `DATA`, `RESIZE`, `CLOSE` and the rest on `session.attached == Some(client)` (`core/sessions.rs:801-815`, `client_close` at `:1129`). Another *client's* session is therefore safe from this tab's keys.

### What the plan missed

**This client's own sessions are not safe.** Take a local tab whose holder and core died:
1. The tab is shown lost, but its `Session` phase is still `Running`.
2. The user opens a new tab. The fresh core gives its session the same id, 1.
3. Keys typed into the lost tab go to the new session (`Terminal::write` → `Session::input` → `Running::input`). `Terminal::exited` gates only `set_layout` and `server_session`.
4. Closing the lost tab runs `Running::close` → `forget(1)`. That removes the *new* tab's sink and sends `CLOSE { session: 1 }`, which the core honours because session 1 is attached to this client. The new tab's program ends.

This needs no second client and no remote host, so it is the most reachable form of F3. Decision 4 handles it.

### The config editor

`open_config` (`src/workspace.rs:1748-1752`) builds a local command and path, then launches them through `launch` on `Host::of(&profile)` of the default profile. `spawn_request` (`src/terminal/mod.rs:1159`) already resolves by `host` alone.

## Goals / Non-Goals

**Goals:**
- Name a shown session by id and key on its connection wherever the client decides about it after a link can have changed cores (`types R3`).
- Decode the wire's key and size sentinels once, where a listing or `SPAWNED` is read (`types R6`, `R7`). Wire structs stay as they are (`process R4`, `R5`, `types R9`).
- Run the config editor in a `Host::Local` tab (`types R8`).
- Put every new decision in pure code with unit tests (`fcis R1`, `R8`, `testing R1`).

**Non-Goals:**
- **No protocol change.** `REPLAY_START` and `KILL` carry no key, and none is added:
  - a reconnect's replays come only for sessions the core gives back to this client's token (decision 3);
  - the sessions palette's kill by bare id (b07 #20) stays where the plan files it, in change 13.
- **No change outside the client.**
  - `ServerSession.key` (the state file), `SessionInfo.key` and the readers that already compare it with an empty-key guard (`reattach::Detached::position`, `reattach::sweep`, `layout::layout_of`, `zone_view.rs:691`, `picker::RowKey`) are untouched. They come into line when next changed (README, "Scope of adoption").
- **No restructuring of `Pending`'s flags** (b25 #11; change 13) or of `State.protocol` (b25 #16; change 13).
- **Out of scope: a session another Brindle attached while this one was disconnected.** It is listed with its own key and a client, so this tab keeps waiting with no lost line, as today. A listing can't say which client has it, and the order of replay and listing is no proof (decision 3).

## Layers

| Part | Layer | Crate and file |
| --- | --- | --- |
| `SessionKey`, `ListedSession`, the matching rule | pure | `brindle`, `src/remote/identity.rs` (new) |
| `State::show`, `shows`, `forget`, `keep_listed`; `carried_attaches` | pure model (GPUI-free, no IO, no clock) | `brindle`, `src/remote/local/connection.rs` |
| `config_editor_launch` | pure | `brindle`, `src/workspace.rs` |
| `Connection::handle`'s `SPAWNED`, `REPLAY_START` and `SESSIONS` arms; `Running`'s methods | shell (reader thread, UI-thread callers) | `src/remote/local/connection.rs` |
| `Session::id_and_key`, `Terminal::server_session`, `ZoneTreeView::zone_server_session` | shell (carry the value) | `src/remote/local/session.rs`, `src/terminal/mod.rs`, `src/zone_view.rs` |
| `Workspace::open_config`, `launch_on`, `attached_terminal`, `restored_terminal` | shell (entity) | `src/workspace.rs` |

No shared crate changes, and no crate reference changes direction.

`src/remote/identity.rs` sits outside `remote::local`, which builds only on Linux and Windows, so its pure code compiles and is tested on every platform (`fcis R7`). On other platforms nothing uses it yet. So it is declared in `src/remote/mod.rs` with the `cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))` its siblings `client` and `hosts` carry. That file is a site of the `ports R4` exception in force (README, "Exceptions in force"; change 5 lists it by name), which ends with `port-to-macos`. This change adds no `cfg` that selects behaviour.

## Decisions

### 1. `SessionKey` and `ListedSession`, decoded once

`src/remote/identity.rs`:
- **`SessionKey`:**
  - a newtype over the key string, with a private field (`types R4`, `R11`);
  - `SessionKey::from_wire(&str) -> Option<SessionKey>` turns the wire's empty string (a core that predates keys) into `None` (`types R7`);
  - `as_str()` and `into_string()` serve the state file and logs.
  - It doesn't derive `Default`.
- **`ListedSession`:**
  - `{ session, key: Option<SessionKey>, attached: bool, size: Option<(u16, u16)> }`, made by `ListedSession::of(&SessionInfo)`;
  - `size` is `None` unless both `cols` and `rows` are non-zero (a core that predates sizes reports 0).
  - The `SESSIONS` arm decodes the listing into `Vec<ListedSession>` once. The prune and `carried_attaches` take that. Listing hooks still get the wire `Vec<SessionInfo>`, since they belong to the launch.
- **The matching rule:** one function, `ListedSession::is(&self, session, key: Option<&SessionKey>) -> bool`. The ids must be equal, and then:

| our key | listed key | same session? | why |
| --- | --- | --- | --- |
| `Some(a)` | `Some(b)` | `a == b` | the key tells sessions apart across cores |
| `Some(_)` | `None` | no | a core without keys didn't give us ours |
| `None` | any | yes, by id | we never had a key (a core that predates keys); nothing better to compare, as today |

- A recorded attach also needs the key to be present (decision 3), as today.

*Rejected:*
- **The type in `brindle-remote-protocol`:** the wire fields stay `String` (`process R5`), and the core keeps its own `meta.key`. A shared type with one user belongs with that user.
- **Keeping `String` and fixing the comparisons:** the empty-string sentinel would stay in every reader (`types R7`).

### 2. One `Shown` entry per shown session

`State.sessions` becomes `HashMap<SessionId, Shown>`, with `Shown { sink, key: Option<SessionKey>, parent: Option<u64>, foreground: Option<Foreground> }`.
- `parents` and `foreground` fold into it, so a session's window and foreground program can't outlive its entry or be pruned apart from it.
- The map stays keyed by id: `DATA`, `EXIT`, `REPLAY_*` and `PROCESS` carry only the id, and within one link the core's ids are unique.
- New model methods, all pure:
  - **`State::show(id, Shown) -> Option<Shown>`** returns an entry it displaced. A displaced entry with another key gets the lost line instead of vanishing silently. This can't happen today, because the post-`WELCOME` `LIST` is answered before any later `SPAWNED`. The guard costs one line.
  - **`State::shows(id, key) -> bool`** says whether that session is still the one shown.
  - **`State::forget(id, key) -> Option<Shown>`** removes the entry only when it is that session. It replaces `remove` for `Running`.
  - **`State::keep_listed(&[ListedSession]) -> Vec<Sink>`** is the after-loss prune. It keeps an entry only when a listed session `is` it and has a client, and it returns the sinks it took out. It replaces `missing` and the two `retain` calls.

*Rejected: keying the map by `(id, key)`.* Every frame would still be looked up by id, so the key would be checked in one place either way, with more code.

### 3. Carried attaches need the key; `attached_by_replay` stays

- **The attach request:** `AttachRequest` becomes `{ session, key: Option<SessionKey>, when: Attaching }`, with `enum Attaching`:
  - `Now { cols, rows }`: a just-listed session, attached at once;
  - `Recorded`: a restored zone's session, attached once a listing confirms it, at the listed size.
  - This removes the `recorded: bool` beside a 0x0 size that meant "unknown" (`types R1`, `R7`).
- **The waiting attach:** `Waiting::Attach` carries the request's `key` and `awaits: Awaits`, replacing `check: Option<String>`:
  - `Awaits::Listing`: a recorded attach; nothing is sent until a listing confirms it;
  - `Awaits::Replay`: its `ATTACH` is out, or will go out.
- **The decision table**, which is `carried_attaches` over `&[ListedSession]`. Messages are as today, except the new row, which reuses the "not listed" text:

| awaits | listing | outcome |
| --- | --- | --- |
| `Listing`, key present | `is` it, no client | `ATTACH` at the listed size (else the request's); the sink told `replay_size`; now `Replay` |
| `Listing`, key present | `is` it, with a client | fails: `session N on X is shown by another Brindle` |
| `Listing` | anything else, including no key | fails: `session N on X has ended` |
| `Replay` (carried over a lost link) | `is` it, with a client | waits for the reconnect's replay |
| `Replay` | `is` it, no client | `ATTACH` sent again |
| `Replay` | listed under another key (**new**), or not listed | fails: `the terminal server no longer has session N` |

- **`attached_by_replay` keeps matching by id.** A `REPLAY_START` without a `req` comes only for a session the core gives back to this client's `client_token`:
  - in a core that adopted the holders, that is the same session;
  - in a fresh core, the only sessions this token owns are ones it spawned or attached on that core after reconnecting;
  - `REPLAY_START` carries no key, and adding one is a protocol change this fix doesn't need (`process R5`).
  - The entry it inserts takes the waiting attach's key.

### 4. A `Running` acts only while its session is shown

- **The guard:** every `Running` method that reaches the core or the shown entry first checks `State::shows(self.id, self.key)`, under the state lock. That covers `input`, `resize`, `reparent`, `foreground` and `set_layout`. `close` and `detach` go through `State::forget(id, key)`.
- **What a lost or exited tab does:** it sends nothing, and no other tab's entry is removed. The tab already shows why, through its lost line or exit, so there is nothing more to show (`railway R6`).
- **Threads:**
  - `input` and `resize` run on the UI thread. They now take the state mutex once per call, as `Running::foreground` already does from the title poll.
  - Every holder of that lock does only map operations under it. The reader drops it before calling a sink, and the writer never takes it while writing. So this is not blocking work (`process R13`, `R14`).

*Rejected: invalidating the `Running` through a flag the reader sets on `lost` or `EXIT`.* That is a second copy of "is it still shown" which must agree with the map. The map already answers it.

### 5. Sizes decoded at the same boundary

`ListedSession::size` replaces both readings of the sentinel:
- `carried_attaches`' `info.cols > 0 && info.rows > 0` (b25 #15);
- `Workspace::attached_terminal`'s copy of it (`src/workspace.rs:789-791`, b07 #23). That copy uses `ListedSession::of(info)` for both the size (`unwrap_or` the default grid) and the key.

### 6. The config editor runs on `Host::Local`

- **The decision:** a pure `config_editor_launch(config: &Config, editor: (String, Vec<String>)) -> (LaunchRequest, Host)` in `src/workspace.rs`.
  - The host is always `Host::Local`.
  - The profile is the default profile when it targets this machine, else the first profile that does, else the default profile. In that last case its theme and font still apply, and the tab still runs here, as a local session reattached under a remote default does (`reattach::local_profile`).
- **The shell:**
  - `open_config` gathers the path and `editor_command`, calls `config_editor_launch` and passes the result to `launch_on`.
  - `launch_on(request, host, ..)` is `launch`'s body with the host passed in. `launch` calls it with `Host::of(&profile)`.
  - In `launch_on`, the two cwd rules that read `profile.remote` read `host` instead: the requested cwd's POSIX filter, and following the active tab only on the same endpoint (`active_cwd_for`). For an ordinary launch the host is `Host::of(&profile)`, so nothing changes there.

*Rejected:*
- **Refusing ctrl-, when no profile is local:** the file is on this machine whatever the profiles say.
- **`Profile::default_shell()`:** it isn't in the config, and a tab's profile is found again by name on reload.

### 7. Failures and where they show

| Failure | Unit | Shown as |
| --- | --- | --- |
| a shown session not listed with its key after a reconnect | its tab | the red lost line, `the terminal server no longer has this session` (existing text) |
| an entry displaced by a session with the same id | its tab | the same lost line |
| a carried attach whose id now has another key | the attaching tab | its attach failure, `the terminal server no longer has session N` |
| a recorded attach with no key, or another key | the restored zone | `session N on X has ended` (unchanged) |
| input, resize or close on a lost or exited tab | — | nothing more to show; the tab already says why |

### 8. Wire, versions, state file, server

- **Wire:** none. `SessionInfo.key`, `Spawned.key`, `SessionInfo.cols` and `rows` keep their encodings and their empty and 0 defaults (`process R4`, `R5`). Any core version meets any client version: what an older core omits decodes to `None`, and the client falls back to matching by id and to the default size, as today. No transcript or fuzz target changes (`process R6`).
- **State file:** `ServerSession.key` is still written as a string, empty for none, by `zone_server_session` at the persistence boundary (`types R9`). Files written before and after load the same (`process R15`).
- **Terminal server:** unchanged.
  - **A core restart that adopts the holders, and an upgrade:** ids and keys are kept, so shown sessions match and nothing differs from today.
  - **A fresh core with no surviving holder:** this change's rules apply.
  - **A Brindle crash:** the client's state is in memory only, and nothing persisted changes.

### 9. Tests, by layer (`testing`)

- **Unit, pure:**
  - `identity.rs`: decoding and the matching table;
  - `connection.rs`: `State::keep_listed`, `show`, `shows` and `forget`, and `carried_attaches` as tables over a `State` with `fake::sink()` sinks, with no thread, link or sleep. These close b25 #8's missing tests for `carried_attaches`;
  - `workspace.rs`: `config_editor_launch`, with `spawn_request` checking that the result is a local request.
- **Client with a fake link** (`fake::link`, threaded, as the existing reconnect tests are):
  - the reused-id prune;
  - the cut-off attach;
  - a lost handle sending nothing while the new session with its id keeps its own close.
- **Existing tests:** their listings gain the keys a real core sends. One keyless listing remains, to pin the rule for a core that predates keys.
- **e2e:**
  - `local-server-reused-id`: holder then core killed, a new tab gets the id, the lost tab is typed into and closed, and the new tab keeps running;
  - `reload-config-editor-local`: the default profile targets an unresolvable SSH host, and ctrl-, opens the editor here.
  - The `remote-*` cases run unchanged. Covering the remote reuse would need a second Brindle against the same container, so the fake-link test covers it.
- **Manual:** none. No step needs real keys or a pointer.

## Risks / Trade-offs

- [A core that predates keys, followed by a fresh core that reuses ids, still matches by id] → Old cores are stepped down by any newer Brindle (`process R12`). The rule is pinned by a unit test.
- [A session taken by another Brindle while this one was disconnected still freezes the tab] → As today; out of scope (Non-Goals).
- [The state lock is now taken on every input call from the UI thread] → Only map operations happen under it (decision 4). Typing latency can be checked with `--latency-log` if in doubt.
- [The editor tab under a remote default profile, when no profile is local, splits locally with that profile's settings] → The tab's host decides where its splits run (`remote-connections`, "A tab stays on its host"). The editor command is the `-e` kind, which isn't restored.
- [Existing tests change their fixtures] → Only to send the keys a real core always sends. The assertions are unchanged, except where the new rule is the point.

## Migration Plan

Nothing to migrate: no persisted, wire or config format changes. A rollback is a revert.
