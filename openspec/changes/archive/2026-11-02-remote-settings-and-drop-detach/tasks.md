## 1. Remote fields in the profile editor

- [x] 1.1 Add `Field::Remote` and the ssh/container fields to `src/settings_dialog/profiles.rs` (`FIELDS`, `TEXT_FIELDS`, labels, hints, visibility by kind), with the container engine as a stepped choice.
- [x] 1.2 Load a profile's `RemoteTarget` into the draft values and build it back, keeping the inactive kind's draft values.
- [x] 1.3 Validate host, container and port; mark the field and block Save.
- [x] 1.4 Write the `remote` inline table (or remove it for `none`) in `src/config_edit.rs`, keeping comments and omitting blank optionals.
- [x] 1.5 Render the new fields in `src/settings_dialog.rs` and show `ssh:<host>` / `container:<name>` in the profile list rows.
- [x] 1.6 Unit tests: load/build round trip for both kinds, validation, write and remove, list label; a dialog test using `--dump-screen-after` style assertions where existing tests do.

## 2. Dropped remote connection

- [x] 2.1 Track last-received time in the remote control reader and send the 10 s tracked heartbeat when idle; declare the connection dead after 30 s of silence with a command pending (remote profiles only).
- [x] 2.2 Read the session's own `#{client_name}` after attach and keep it on the session.
- [x] 2.3 On a detected drop: kill the transport, emit `Detached("connection lost")` (or `Failed` if never attached), and spawn the bounded background `detach-client -t <name>` over a new connection (`remote::detach_command`).
- [x] 2.4 Unit tests for the deadline logic (fake clock) and the detach command line; confirm local sessions send no heartbeat.
- [x] 2.5 E2E case in `tests/e2e/cases/remote.rs`: attach over the Docker SSH fixture, freeze the transport (`docker pause`/blocked port), expect Detached within the limit, unfreeze, expect no leftover client in `list-clients`.

## 3. Docs and checks

- [x] 3.1 Update the README settings and remote sections.
- [x] 3.2 Run `cargo test --workspace` and the new e2e case; fix any warnings.
