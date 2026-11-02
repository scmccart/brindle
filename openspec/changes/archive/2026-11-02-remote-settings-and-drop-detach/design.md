## Context

- The profile editor (`src/settings_dialog/profiles.rs`) has a fixed `FIELDS` list, text fields in `Values::texts`, enum fields stepped with `step(...)` (tmux mode, theme), and tmux-only fields filtered by `visible fields`. `config_edit.rs` writes a profile's table keeping comments.
- `RemoteTarget` is a tagged enum with `deny_unknown_fields`; `Profile.remote` is `Option<RemoteTarget>`.
- Remote tmux runs `tmux -u -C` through `ssh` or `docker|podman exec -i` pipes (`remote::tmux_command`). ssh already has `ServerAliveInterval=5`, `ServerAliveCountMax=3`, so a dead ssh exits locally after about 15 s, closing stdout and giving the reader EOF, which becomes `on_detached`. Container exec has no keepalive, and a stalled pipe that never closes is never noticed. The remote tmux server keeps the control client attached until sshd/dockerd notices the closed pipe, which can take minutes after a silent cut.
- An empty line on a control-mode stdin makes tmux exit the client, so a heartbeat must be a real command.

## Goals / Non-Goals

**Goals:**
- Edit every `remote` key from the dialog with the same save/preview/validation behaviour as other profile fields.
- Detect a dead remote control connection in bounded time, for ssh and containers alike, and release the server-side client when the host is reachable.

**Non-Goals:**
- Reconnecting automatically (a detached session is reattached by the user, as today).
- Detaching clients left by a crashed Brindle, or by a different machine.
- Changing plain-tmux or non-tmux remote tabs; the helper's own idle limit covers those.

## Decisions

1. **Remote fields in the profile editor.** Add `Field::Remote` (enum `none|ssh|container`, stepped like `Tmux`) plus text fields `SshHost`, `SshUser`, `SshPort`, `SshIdentity`, `SshArgs`, `ContainerEngine`, `Container`, `ContainerUser`. The container engine is a stepped choice (`docker`, `podman`) rather than free text, so it can't be invalid. Fields not belonging to the chosen kind are filtered out like tmux-only fields. Values for the inactive kind are kept in the draft so switching back doesn't lose typing, but only the active kind is written. Alternative, one generic "remote" text blob in TOML syntax, was rejected: no validation, and unlike the other fields.
2. **Writing.** `config_edit` writes `remote` as an inline table matching the documented form, omitting blank optionals, and removes the key for `none`. It reuses the shell-word split/join already used for `args` and `tmux_args`.
3. **Validation.** Same mechanism as the font size field: a per-field error that blocks Save.
4. **Heartbeat as drop detector.** A remote control session sends a tracked no-op command every 10 s when no other command is outstanding (`display-message -p ''`, matched FIFO like any command), and treats the connection as dead when nothing at all arrives from tmux for 30 s while a command is pending. Any received line resets the clock, so busy sessions never need the heartbeat to answer. This works for ssh and containers and catches stalls ssh's keepalive can't see (a wedged `docker exec`). Only remote profiles run it. Alternative, rely on `ServerAlive*` alone, was rejected for containers and because it leaves the stall case open.
5. **Releasing the remote client.** At attach, after the first reply, the session asks for `#{client_name}` of its own client. On a detected drop it kills the local transport, then on a background thread runs `tmux [tmux_args] detach-client -t <name>` over a fresh `remote_command` with `ConnectTimeout` and an overall 10 s deadline, then forgets it. No window state depends on it. The client name is stored on the session only, nothing is persisted.
6. **Reporting.** Drops use `TmuxEvent::Detached("connection lost")` when the session had attached, else the existing `Failed`. A clean `%exit` and a user detach keep their own reasons and do not run the cleanup (nothing is held).

## Risks / Trade-offs

- A 30 s limit may fire on a very slow link; the constant is named and the scenario "slow but live" pins the intent. Mitigation: any received byte resets it, not only the heartbeat reply.
- The cleanup opens one more connection to a host that was just flaky; it is bounded, silent, and skipped when the session was never attached.
- Hidden inactive-kind drafts could surprise someone saving after switching kinds; only the active kind is written, which is the visible state.

## Open Questions

- None blocking. If the user wants automatic reconnect after a drop, it is a separate change.
