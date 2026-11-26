# Proposal

**Depends on:** `agents-login` and `agents-session-recovery` (archived), whose `AGENT_RESTART` is the message this change fixes.

**Status: designed.** design.md, the delta specs and tasks.md are written (2026-11-25). The choices the user didn't make are listed under "Choices made here", each with the recommendation the design follows.

## Why

On 2026-11-25 the user ran Antigravity's terminal login in an agent zone over SSH. The login worked, but the agent that started again after it failed because `$HOME` wasn't set. A fresh tab then worked. The cause is that Brindle merges the endpoint's environment into `SPAWN` and `SPAWN_AGENT` but not into `AGENT_RESTART` (`encode`, `src/remote/local/connection.rs:2516`). The restarted agent therefore runs with only the profile's `env`: no `HOME`, `USER` or `PATH`. Continue conversation on a live session takes the same path.

The user also reported "related issues like this" and asked that the user's login shell be run. Today the endpoint's environment is what sshd gives a non-login command: `HOME`, `USER`, `SHELL` and sshd's default `PATH`, and nothing from `~/.profile` or `~/.bashrc`. Only a terminal profile with no command gets a login shell. An agent or command that lives in `~/.local/bin`, or comes from nvm, cargo or pyenv, is not found, or a different Node is found.

## What Changes

- **The restart bug.** Every client message that starts a program on a remote endpoint (`SPAWN`, `SPAWN_AGENT` and `AGENT_RESTART`) gets the endpoint's environment under its own, through one closed `match` over `ToCore`. A restarted or continued agent then runs with the same environment a fresh start of the same profile gets.
- **A login shell's environment, read once per SSH connection.** When it starts, the relay on an SSH endpoint runs the user's login shell (`$SHELL -l -i -c`). That shell execs a hidden `brindle-remote login-env` role, which writes its environment to a pipe of its own. Nothing the startup files print can reach the relay's framed stdout. The capture runs while the relay reaches the core, is limited to 10 s, and is killed with its process group when it runs past that. `RELAY_HOST` gains an optional `login` field, which carries either the environment or why it wasn't read.
- **Which environment each program gets.** A remote program that Brindle execs directly gets the login shell's environment with the request's environment over it. These are a terminal profile's command, an agent, a login zone's program, and a restart or continue. A terminal session with no command runs the endpoint's login shell itself, so it keeps the environment it has today: sshd's, which that shell's own startup files build on. When the capture failed, a program that would have used it gets sshd's environment and says so visibly: a dim line in a terminal, a note in an agent zone.
- **Containers** keep the environment of the engine's `exec`, which is what the image's `ENV` designed. A login shell there would run `/etc/profile`, which on Debian and Alpine resets `PATH` and would hide, for example, an image's `/usr/local/go/bin`. Every endpoint's environment gets `HOME`, `USER`, `LOGNAME` and `SHELL` filled in from the passwd entry when the endpoint left them unset or empty.
- **A profile's `env`** stays on top of everything, the login environment included. Brindle's terminal variables (`TERM`, `COLORTERM`, `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`) stay over the endpoint's in terminals, and a terminal login's method `env` stays under its deny list.
- **No protocol change.** Client protocol 4 and holder H1 3 are untouched: the fix and the merge are on the client, and `AGENT_RESTART` already carries a complete environment. The relay link's `RELAY_HOST` gets an additive optional field. A relay always runs Brindle's own release, and without the field it encodes byte for byte as before.
- **Out of scope:** local sessions on Linux and Windows. A local program gets Brindle's own environment, as today. On Linux that is a login session's when the desktop started Brindle (display managers run the session through a login shell). The design names two possible proposal-only follow-ups: refreshing a Windows Brindle's environment from the registry for new sessions, as Windows Terminal does, and a Linux Brindle started outside a login session.

## Choices made here

The user asked for a login shell's environment for every remote program and left the details open. These are the recommendations the design follows, each in design.md:
1. `-l -i` (login and interactive) rather than `-l` alone, so `~/.bashrc` lines below Ubuntu's interactive guard (nvm, cargo) count. VS Code's remote environment resolution does the same (decision 3).
2. Containers keep the `exec` environment, with only the passwd basics filled in (decision 6).
3. A terminal session with no command keeps sshd's environment, so its own login shell doesn't add its `PATH` entries a second time (decision 5).
4. A 10 s limit, run alongside reaching the core (decision 4).
5. A failed capture is shown on each program that falls back, not only logged (decision 7, `railway R6`).
6. Shells Brindle can't run as `-l -i -c` (csh, tcsh, nu and others) count as a failed capture, with the reason given (decision 3).
7. No config key to turn the capture on or off, or to ask for it in a container (decision 6).
8. The environment is read anew for each connection, so a reconnect picks up edited startup files. It is not cached on the host (decision 4).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `remote-connections`: "Remote programs get the endpoint's environment" says which environment each kind of remote program gets: the login shell's for programs Brindle execs, sshd's for a login-shell session, the `exec` environment in a container, and the profile's over each. A new requirement says when and how an SSH endpoint's login environment is read, and what a failed read shows.
- `terminal-server`: a new requirement, "The relay reports a login shell's environment", for when the relay is asked for one.
- `agent-zones`: a new requirement, "An agent started again gets a new start's environment", which covers a restart after a terminal login and Continue conversation.

## Impact

- `brindle-remote-protocol`: `relay.rs` gets `RelayHost.login` (`LoginEnv::{Captured, Failed}`), a transcript fixture and fuzz seeds.
- `brindle-remote-agent`: a new pure module `server/login_env.rs` (the invocation per shell, the dump's encoding and parse, the merge, the passwd basics); `server/relay.rs` gets the capture thread and the `--login-environment` argument; `main.rs` gets the hidden `login-env` role. Real-role tests go in `remote-agent/tests/relay.rs` and `agents.rs`.
- `brindle`: a new pure module `src/remote/local/endpoint_env.rs` (which base each `ToCore` start gets, and the merge); `encode` in `connection.rs`; the fallback line or note; the SSH launch script in `src/remote/client.rs`. The Docker transport scenarios gain checks.
- e2e: `remote-agent-login-terminal` checks the restarted agent's `HOME`, the login variables and `PATH`.
- Docs: `docs/code-map.md` (`remote/`, the relay), the fcis table of whole pure modules and `PURE_MODULES`, and `CLAUDE.md`'s manual checks.
