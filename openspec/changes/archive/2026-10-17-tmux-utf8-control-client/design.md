# Design

## Context

tmux decides per client whether it handles UTF-8. A client counts as UTF-8 when it was started with `-u`, when `$TMUX` is set, or when the first non-empty one of `LC_ALL`, `LC_CTYPE` and `LANG` names UTF-8. For a control client that isn't UTF-8, tmux replaces control characters and non-ASCII bytes in command output with `_`. On tmux 3.6, with a server started under `C.UTF-8` and a window named `café`, a `-C attach` client with no locale variables gets `@0_caf_` for `list-windows -F "#{window_id}\t#{window_name}"`. With `-u` it gets `@0<TAB>café`.

The process's own locale is a separate matter: tmux tries to set a UTF-8 `LC_CTYPE` for itself (`en_US.UTF-8`, then `C.UTF-8`) and only falls back to the environment's when neither is installed. In the CI failure and the probe above the server ran under `C.UTF-8`, so the failure was the client's flag alone, which `-u` sets.

Brindle builds the control client's command line in `tmux::command_line` and spawns it with Brindle's environment plus `child_env` (which sets no locale). The e2e suite has its own raw control client, `ControlClient::attach`, which runs `tmux -L <socket> -f /dev/null -C attach -t e2e`, and the `tmux-*` cases parse its replies the same way.

## Goals / Non-Goals

**Goals:**
- Control clients behave the same in every locale as they do under a UTF-8 one today.
- The e2e suite's raw control client is just as independent of the locale, so the suite no longer needs `LC_ALL` for tmux.

**Non-Goals:**
- Classic (`tmux = "plain"`) profiles. Their client runs in a Brindle PTY and draws non-ASCII text as `_` without a UTF-8 locale, but that is a separate change (it touches the `profiles` spec and is a manual check), not the attach failure fixed here.
- Setting a locale for the shells in panes. Panes get the server's environment, as now.
- `has_session_command`: its output is never parsed, only its exit status.

## Decisions

### `-u` first, before `tmux_args`

The command line becomes `tmux -u [tmux_args] -C new-session …`. tmux takes its flags in any order, so placing `-u` first keeps `tmux_args` next to `-C` as now and means the profile's flags are read after it. A `-u` already in `tmux_args` is harmless: the flag just sets a bit twice.

Alternative: setting `LC_CTYPE=C.UTF-8` in the client's environment. It would work through the same check, but it also changes the locale of anything that environment reaches, like a server started by this client, whose environment the panes inherit. It also depends on how tmux reads the variables. `-u` says exactly what Brindle means: this client reads UTF-8, which it always does.

Where a UTF-8 locale is already set, `-u` changes nothing, since tmux already sets the same flag. So this fixes the locale-less case without changing any other.

### The raw e2e client gets `-u` too

`ControlClient::attach` runs `-u -C attach -t e2e`. The `tmux-*` cases check tmux behaviour that Brindle relies on, so they should see what Brindle's client sees: a UTF-8 client. Without this, those cases would still fail without `LC_ALL`, which would contradict the docs once they stop saying tmux needs it.

### `scripts/with-xvfb` keeps `LC_ALL=C.UTF-8`

It still gives consistent runs (bash, `ls` and other tools in panes follow the locale). Its comment and `CLAUDE.md` stop giving tmux's mangled tabs as the reason.

### The e2e case removes the locale from Brindle only

A new display case, `attach-without-locale`, starts the throwaway server as usual (under the harness's locale), renames its window to `café`, adds a window `logs`, and runs Brindle with `LANG`, `LC_ALL`, `LC_CTYPE` and `TMUX` removed from Brindle's environment. It checks that the dump's tabs are `café` and `logs`. `TMUX` is removed because tmux treats any client with `$TMUX` set as UTF-8, so running the suite from inside tmux would otherwise hide the bug.

This matches what failed in CI (the client had no locale) and what the probe above showed. `Run` gains an `env_remove` builder for it, beside `env`.

The case should fail on the current `main` (no tabs appear, since the window list can't be parsed). Running it once before the fix confirms that it catches the bug.

## Risks / Trade-offs

- [Brindle itself runs without a locale in the e2e case: GPUI or xkb may log warnings or skip compose tables] → the case checks only tab titles, which don't depend on it. A crash would show up as a failed run, which is worth knowing about anyway.
- [A host with no UTF-8 locale installed at all] → tmux refuses to start (`need UTF-8 locale`), with or without `-u`, and Brindle's attach-failure tab shows that error. Out of scope here.
- [A tmux build without `-u`] → `-u` has been in tmux since 1.x, older than every version Brindle supports (control mode, `refresh-client -C`).
