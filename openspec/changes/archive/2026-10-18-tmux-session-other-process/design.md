# Design

## Context

Every attach of a control-mode session goes through `TmuxSession::attach`, which spawns `tmux -u [tmux_args] -C new-session …` straight away and queues `#{version}` and `list-windows`. Before that, `Workspace` checks only this process: `attach_tmux` and `open_profile` look through `find_session`, which reads the `tmux_sessions` of this process's windows. The callers are startup (`Launch::Request`), `open_profile` (ctrl-alt-N, ctrl-shift-t, `+`, ctrl-shift-n), the new-tab palette, `restore`, and the session-ended palette, which keeps the ended tabs in `replacing` until the new session's first tab arrives. A session that fails before attaching reports `TmuxEvent::Failed`, and `Workspace` drops it and opens a message tab.

The client's environment is Brindle's own plus `child_env`. A tmux server started by that client inherits that environment, and so do the shells in its panes.

On tmux 3.6, `list-clients -t =<name> -F '#{client_pid}'` lists the session's clients and exits 1 when the session or the server doesn't exist. `/proc/<client_pid>/environ` of a `tmux -C` client shows the variables it was started with, and `/proc/<pid>/stat` has its parent pid after the `)` that ends its name. tmux names the client process `tmux: client`, which contains a space.

## Goals / Non-Goals

**Goals:**
- No second Brindle client on a session that another Brindle shows, however it's being attached.
- Nothing changes for one Brindle process, or for sessions no other Brindle shows, apart from one quick `list-clients` before attaching.

**Non-Goals:**
- Raising the other Brindle's window ("Go to"). That needs a channel between processes. The single-instance approach in Decisions would give it.
- The profile picker's "open in another window" detail for sessions another process shows. Working it out would mean running tmux every time the picker opens. Picking the profile asks, as the specs say.
- Two Brindles launched at the same moment, both checking before either has attached. See Risks.
- Classic (`plain`) profiles, whose tmux runs in a PTY and may mirror on purpose.

## Decisions

### Detect over tmux, not through Brindle-to-Brindle IPC

The check asks tmux who is attached. Alternatives:
- **Single instance**: a second launch hands its request to the running Brindle over a socket or D-Bus, so the in-process rules, "Go to" included, apply as they do now. It's more work (a socket, its lifetime, handing over `--send` and the other debug flags, a second launch with another `BRINDLE_CONFIG`). The user chose the smaller fix.
- **Session option** (`set @brindle-pid`): it goes stale when Brindle is killed, and two Brindles on the session would overwrite each other's value.
- **Parent's executable name** (`/proc/<ppid>/comm == "brindle"`): it breaks for a renamed binary, and wrappers would trip it.

### Marker: `BRINDLE_CONTROL_CLIENT=<pid>`, checked against the parent

Control clients are spawned with `BRINDLE_CONTROL_CLIENT` set to Brindle's pid. A listed client is another Brindle's when its environment has the marker, the marker equals the client's parent pid, and that pid isn't this process's.

The parent check matters because the marker leaks: a server started by a Brindle client inherits the marker, so shells in its panes have it, and so would a `tmux -C` run from one of them. Its parent is the shell, so it doesn't count. `TERM_PROGRAM=Brindle` already leaks into panes the same way. A dead Brindle's client exits when its stdin closes, so a listed client with a matching parent is a live Brindle.

Clients whose `/proc` entries can't be read (another user on a shared socket, or a client that just left) don't count.

The decision is a pure function, `other_brindle_client(pids, own_pid, read)`, where `read` gives `(ppid, environ)` for a pid. It's unit-tested next to `command_line`, with `/proc` reading kept out of it.

### The check runs inside `TmuxSession`, before the spawn

`attach` keeps its synchronous signature, so every caller and `Workspace`'s bookkeeping (`tmux_sessions`, `anchors`, `quiet_sessions`, `replacing`, `creating`) stays as it is. For `AttachSpec::Named`, the entity starts without a child. It runs the `list-clients` check on the background executor, then either calls `start` (spawn, reader and writer threads, `#{version}`, `list-windows`) or emits a new `TmuxEvent::ShownByAnotherBrindle`. `AttachSpec::New` starts at once, since tmux refuses a name that's already taken.

Commands queued before `start` wait in the writer channel, but none are expected, since `Workspace` only acts once the first window is listed. `child` is `None` until then, which `release`, `Drop` and the quit hook already allow for after a failed spawn. A quit during the check sends nothing.

Alternative: check in `Workspace` before calling `create_session`. Every caller would turn asynchronous, and restore's anchors and the session-ended `replacing` entry would need a placeholder while the check runs.

### `Workspace` treats the event like `Failed`, without the message tab

It drops the session (`tmux_sessions`, `forget_restore`, `creating`) and asks with a third `SessionLocation`, `OtherBrindle(name)`. `session_open_rows` gains a flag that leaves out "Go to", and the target is always `Tab`, because the window has already been chosen: for ctrl-shift-n it is the new window. The placeholder says "is open in another Brindle".

### Windows left without tabs, and ended tabs left behind

A session that ended with `tmux_session_end = "ask"` is already out of `tmux_sessions` while its tabs wait in `replacing` for its replacement. When the replacement turns out to be shown by another Brindle, its `replacing` entry goes, which leaves those ended tabs without an owner. The rule is therefore general, applied when the ask palette or a prompt it opened is dismissed, and after the ask palette opens a local tab: close the tabs of sessions that are no longer attached and that no `replacing` entry waits on, then close the window if it has no tabs and no session attaching. "New tmux session…" hands the ended session to the new one through `replacing`, as the session-ended palette does today, so its tabs go once the new session shows up.

A successful prompt reports `Dismissed` only after the new session's first tab has arrived (the reply is sent on `WindowAdded`), so the window is never empty at that point.

## Risks / Trade-offs

- [Two Brindles started within a few milliseconds can both see no other client and both attach] → They then mirror, as today. Launchers don't start two Brindles at once, and the window between the check and the attach is the length of one `list-clients` call.
- [A `list-clients` call before every named attach] → It's one short tmux command on the background executor, and nothing is shown before the attach anyway.
- [A Brindle client on a server started under another user, or with `/proc` hidden (`hidepid`), isn't recognised] → Brindle attaches as before. Same-user `/proc` reads are allowed under `hidepid`.
- [The marker leaks into panes of a server that a Brindle client started] → Harmless, because of the parent check. It's documented beside `child_env`.

## Migration Plan

None. No config or state changes. A Brindle without this change, running beside one with it, isn't recognised (its clients carry no marker), so the new one attaches as before.
