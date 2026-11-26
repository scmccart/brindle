# Design

## Context

See proposal.md for the problem. This section covers how remote programs get their environment today (`main` at 725ed28).

- **The relay.** An SSH endpoint runs `ssh … sh -c '<launch script>'`, which execs the cached helper's `relay` (`ssh_launch_script`, `src/remote/client.rs:322-339`). That command deliberately isn't a login shell, so startup files can't print into the framed stream. A container runs `docker exec … sh -c` the same way. The relay writes `FRAME_STREAM_MARKER`, reaches the core (`reach()`: probe, upgrade, then the client's connection), and sends `RELAY_HOST` (`Reached::host`, `remote-agent/src/server/relay.rs:220-240`). The client's `HELLO` must follow within the core's limit (5 s) of that connection opening.
- **`RELAY_HOST.env`** is the relay's own environment, `std::env::vars_os()`. Over SSH that is what sshd gives a non-login command: `HOME`, `USER`, `LOGNAME`, `SHELL`, `MAIL` and sshd's default `PATH`, and nothing from `/etc/profile`, `~/.profile` or `~/.bashrc`. A container gives the image's `ENV` plus what the engine adds, and no `SHELL`. The relay reads `SHELL` from passwd for its own `shell` field, but leaves `env` as it is. The protocol's doc and `relay_env`'s both call it "a fresh login's", which overstates it.
- **The merge.** `encode` (`src/remote/local/connection.rs:2516-2525`) lays a `SPAWN`'s and a `SPAWN_AGENT`'s `env` over `RELAY_HOST.env` (`relay::relay_env`). Every other message goes out as it is, `AGENT_RESTART` included. The holder runs a session with exactly the environment it is given (`exact_env`, `env_clear()`, `pty/unix.rs` and `platform/unix/piped.rs`). A session with no command runs the login shell as `-bash`. A session with a command is a bare `exec`, and Rust's `Command` looks the program up on the child's `PATH` when the environment sets one.
- **What each start sends as its own environment.** A terminal sends `child_env(profile)`: `TERM`, `COLORTERM`, `TERM_PROGRAM` and `TERM_PROGRAM_VERSION`, with the profile's `env` over them. A remote agent sends the profile's `env` alone (`spawn::environment`'s remote branch). A terminal login's zone is a terminal spawn: `child_env` of the profile with the method's applied `env` over it (`login::overlay`). A restart or continue sends `agent_spawn_request(…).restart()`, which for a remote host is the profile's `env` alone.
- **Locally,** the client builds the whole environment from Brindle's own (`server_env`, and `spawn::environment`'s local branch), restarts included. Nothing is merged at `encode`, so local restarts already match a fresh start.

## Goals / Non-Goals

**Goals:**
- A restart or continue on a remote endpoint gets exactly the environment a fresh start of the same profile gets, and no future message that starts a program can miss the merge.
- On an SSH endpoint, programs Brindle execs directly get the login shell's environment, with one capture per connection, a bounded cost, and nothing it prints reaching the frames.
- A capture that fails degrades to today's environment and says so on each program it affects.

**Non-Goals:**
- Local sessions on Linux or Windows. They keep Brindle's own environment.
- A config key to turn the capture off, or on for a container.
- Expanding `$PATH` or other references in a profile's `env`. A profile's value replaces the variable, as it does today.
- Running a remote program through the login shell (`$SHELL -l -c 'exec …'`), which would change its argv, its process name and the `PROCESS` reports.
- Any change to client protocol 4, H1 3, the holder or the core.

## Decisions

### 1. One classification of the starts, with no wildcard

A new pure module, `src/remote/local/endpoint_env.rs`, owns the client's half:

```rust
/// What a message starts on the endpoint, which decides its base environment.
pub enum Start { LoginShell, Program }

/// Every `ToCore` variant named, no `_ =>` arm (`railway R3`, `types R2`).
pub fn start_of(message: &ToCore) -> Option<Start>
//   Spawn { command: None, .. }          -> Some(LoginShell)
//   Spawn { command: Some(_), .. }       -> Some(Program)
//   SpawnAgent(_) | AgentRestart(_)      -> Some(Program)
//   Hello | Data | Resize | Close | … | AgentLogin -> None

pub struct Merged { pub env: BTreeMap<String, String>, pub fallback: Option<Fallback> }
pub fn merged(endpoint: &EndpointEnv, start: Start, own: &BTreeMap<String, String>) -> Merged
```

`encode` becomes: `match (start_of(message), host)`. With a start and a relay host, it lays the message's `env` over `merged`'s base and re-encodes the message. Otherwise it encodes the message as it is. `AGENT_RESTART` is fixed by being in the table. A new `ToCore` variant fails to compile until someone decides whether it starts a program. `relay::relay_env` is replaced by `merged`, and its test moves with it.

**Alternative:** add a third `if let` for `AgentRestart` to `encode`. That is three lines, but the next message that starts a program would have the same gap. That gap is how this bug was made.

**Alternative:** build the full environment where the request is made (`agent_spawn_request`), which needs the `RelayHost`. The request is built on the UI thread, before a link is up, and it may be re-sent on a later link after a reconnect. The merge belongs where the link is known, at the writer.

### 2. The capture happens in the relay

The relay is the one process that runs on the endpoint as the user, once per connection, before any session starts, and already reports the endpoint's environment. The capture's result goes in `RELAY_HOST`, so the core and the holder stay neutral: they run what each request says (`process R1`).

**Alternatives considered:**
- **Wrap each program in the holder** (`$SHELL -l -c 'exec "$0" "$@"'`). This costs a shell start per session, changes argv and the foreground-process reports, puts policy in the holder (`process R1`), and doesn't help a restart, which is a pipe spawn.
- **Capture in the core.** The core is shared by local clients and every relay, and it outlives connections. It would cache an environment past edits to the startup files, and it would put user-shell policy in a process that should only run requests.
- **Make the launch script a login shell** (`ssh host -l …` or `bash -lc`). Startup files would then print into the framed stream, which is what the launch avoids today (`process R7`). The relay's own environment would also become the login one, and the no-command sessions would run the profile twice (decision 5).
- **A second `ssh` command from the client** to read the environment. That is another connection and authentication per connect, and the docker transport tests require one `ssh` process per warm launch.

### 3. How the relay reads it

The pure half is a new whole pure module, `remote-agent/src/server/login_env.rs`. It is ungated, with a lint-only `cfg_attr(not(target_os = "linux"), allow(dead_code))`, since only the Linux relay calls it. It joins fcis's table and `PURE_MODULES`.

- **The invocation** (`invocation(shell: &str, helper: &str) -> Result<Vec<String>, LoginEnvFailure>`) is chosen by the shell's file name:
  - `bash`, `zsh`, `ksh`, `mksh`, `oksh`, `sh`, `dash`, `ash`, `busybox`: `[shell, "-l", "-i", "-c", "exec \"$0\" login-env", helper]`, with the helper's path a positional parameter, never quoted into the script.
  - `fish`: `[shell, "-l", "-i", "-c", "exec <helper> login-env"]`, with the path quoted by fish's single-quote rules (only `\\` and `\'` escape).
  - Anything else (`csh`, `tcsh`, `nu`, `xonsh`, `elvish`, `pwsh`, unknown) is `Unsupported { shell }`. Their `-c` doesn't combine with a login flag, or they aren't POSIX-like.
  - An empty shell is `NoShell`.

  `-i` makes `~/.bashrc` and `~/.zshrc` count. Ubuntu's `~/.bashrc` returns at once when the shell isn't interactive, and installers append below that guard (nvm, and cargo's `env` line), so with `-l` alone an `npx`-launched agent wouldn't find nvm's Node. VS Code's remote server resolves its environment the same way (`-i -l -c`, 10 s). **(Flag: the user didn't choose `-i`; recommended.)**
- **The hidden role** `brindle-remote login-env` writes its own environment (`vars_os`) to file descriptor 3 as `NAME=VALUE\0` records, then one empty record (`\0`) as the terminator (`login_env::encode_dump`), and exits 0. It is declared beside `relay`, under `target_os = "linux"`, refuses arguments, and is absent from the usage text, as `launch` is.
- **The process** (shell code in `server/relay.rs`, the relay's Linux seam):
  - stdin is `/dev/null` and stdout is `/dev/null`;
  - stderr goes to a pipe, of which the first 4 KiB are kept for the reason;
  - fd 3 is the write end of the dump pipe (`dup2` in `pre_exec`);
  - `setsid()` gives it its own session and process group and leaves it without a controlling terminal;
  - its working directory is the home directory, and its environment is the relay's own after decision 6's basics.

  None of the shell's output can reach the relay's stdout, so a noisy `~/.profile` needs no marker skipping here. Because the shell `exec`s the helper, it never exits as a shell: no `~/.bash_logout` runs and no history file is written.
- **Reading** (`login_env::parse_dump(bytes) -> DumpRead`): a reader thread reads the dump pipe until the terminator, not until EOF. A program a startup file left running (`ssh-agent`, which inherited fd 3) would hold the pipe open forever. The parse:
  - bounds the dump at 256 KiB and 4096 variables (`railway R5`);
  - drops records that aren't UTF-8, as `RELAY_HOST.env` does today;
  - drops records with an empty name or no `=`;
  - treats EOF before the terminator as `Exited`.
- **The result** (`login_env::captured(own, dump) -> BTreeMap`): the dump, except that `_`, `SHLVL`, `PWD` and `OLDPWD` are taken from the relay's own environment, or left out when it has none. The shell set those for itself, and they would be wrong in every session.

```rust
/// Why a login shell's environment wasn't read (`railway R1`, `R4`: one
/// consumer, which shows the text; the variants are for the tests and the log).
pub enum LoginEnvFailure {
    NoShell,
    Unsupported { shell: String },
    Spawn { shell: String, error: String },
    TimedOut { after: Duration },
    Exited { status: String, stderr: String },   // EOF or exit before the terminator
    TooLarge,
}
impl LoginEnvFailure { pub fn text(&self) -> String }  // what RELAY_HOST and the note say
```

The relay's outcome is the protocol's `LoginEnv` (decision 8): `Captured(env)` or `Failed(text)`. `LoginEnvFailure` stays inside the helper.

### 4. When: once per connection, alongside reaching the core, at most 10 s

- `run()` writes the marker, then starts the capture on a thread when the relay was started with `--login-environment`. It then runs `reach()`'s probe and upgrade as today. Before `server.connect()` opens the client's connection, it joins the capture, waiting at most until 10 s after the capture started. Joining before the connection opens keeps the capture out of the core's 5 s `HELLO` limit. The client's limit from marker to `WELCOME` (`Timings::bootstrap`, 120 s) is far above 10 s.
- When the 10 s run out, the relay sends `SIGKILL` to the shell's process group, a thread reaps it, and the result is `TimedOut`. A program that double-forked out of the group (a daemonised `ssh-agent`) keeps running, as it would after an SSH login.
- `core.log` gets one `relay[pid]:` line for any failure, naming it. It never logs the environment's values.
- `BRINDLE_TEST_RELAY_LOGIN_ENV_MS` replaces the 10 s for tests, as `BRINDLE_TEST_RELAY_IDLE_MS` replaces the idle limit (`process R20`). It is a timing, not a fault, so release builds honour it too, like the relay's other two.
- **Cost:** one shell start per connection, not per tab. A plain `bash -l -i` takes tens of milliseconds, and one that loads nvm or conda takes 0.5–2 s. That time overlaps the probe and upgrade, so a warm core adds roughly the shell's own time to the first tab's start, and an upgrade's step-down hides it entirely. Every reconnect starts a new relay and reads the environment again, which picks up edited startup files. Sessions already running keep the environment they started with. **(Flag: 10 s and re-reading per connection were not the user's choice; recommended.)**

### 5. Which base each start gets

`EndpointEnv` is built from `RELAY_HOST` at the boundary (`types R6`). It holds `exec` (the relay's own environment, after decision 6) and `login: Login::{NotAsked, Captured(env), Failed(text)}`. `merged` decides:

| Endpoint | `Start::LoginShell` | `Start::Program` |
| --- | --- | --- |
| SSH, `Captured(env)` | `exec` | `env` |
| SSH, `Failed(text)` | `exec`, no fallback | `exec`, with `Fallback { text }` |
| Container (`NotAsked`) | `exec` | `exec` |
| Local | not merged (no relay host) | not merged |

The request's own environment then goes over the base, with names compared as given, since endpoints are Linux. That environment is `child_env` for a terminal, the profile's `env` for an agent, `overlay(profile env, applied method env)` for a login zone, and the profile's `env` for a restart. So a profile's `env` always wins, and in a terminal so do Brindle's `TERM`, `COLORTERM`, `TERM_PROGRAM` and `TERM_PROGRAM_VERSION`. A terminal login method's denied keys (`PATH` among them) stay denied, so its program gets the login `PATH`.

A session with no command runs the login shell itself (`-bash`), whose startup files build on what sshd gave, as an SSH login does. Giving it the captured environment would run `~/.profile` over an environment it had already built, which puts every `PATH` prefix in twice and re-runs `eval "$(ssh-agent)"`. **(Flag: the user asked for the login shell's environment "for every remote program"; this keeps sshd's for the one case that runs the login shell anyway. Recommended.)**

### 6. Containers, and the passwd basics

The client passes `--login-environment` only in the SSH launch script. The container launch is unchanged, so a container's relay reports `login` as absent (`NotAsked`).

- A container's `exec` environment is the image's designed one. `/etc/profile` on Debian and Alpine sets `PATH` outright, so a login shell there would hide an image's `/usr/local/go/bin` or `/opt/conda/bin`. A container is also often the user's build environment, where the image's `ENV` is the contract.
- What containers do lack is the basics a login sets. For every endpoint, the relay's own environment gets `HOME`, `USER`, `LOGNAME` and `SHELL` filled in from the passwd entry of its uid when they are unset or empty (`login_env::with_basics(env, passwd: Option<&Passwd>)`). The passwd lookup is the existing `pty::passwd_entry`, extended with the user name. Over SSH, sshd already set them, so this changes nothing there. The capture's shell starts from this filled environment too.
- No config key turns the capture on in a container or off over SSH. Turning it on would be an additive profile key on the remote table later. **(Flag: containers were the user's "every remote program" too; recommended to keep `exec`'s environment plus the basics. Also flagged: no key.)**

### 7. A failed capture is shown where it lands

`railway R6`: the failure belongs to the endpoint, but it is felt by each program that falls back. When `merged` returns a `Fallback`, the connection records it with the spawn's request.

- **A terminal** (`SPAWN` with a command): when its `SPAWNED` arrives, the connection writes one dim line into the terminal through the notice line's control filter, as `lost_line` does: `the login shell's environment on A8 wasn't read (it took longer than 10 s), so this program has the SSH session's environment`. The line goes before the program's own output.
- **An agent zone** (`SPAWN_AGENT`, `AGENT_RESTART`): the link hands the text to the zone's sink, and the model keeps it as an endpoint note shown above the conversation. A restore's note and this one each get a line. The dump adds a `--- note "…"` line, which the e2e cases can wait for.
- **A session with no command** shows nothing, since it doesn't use the login environment.
- **Nothing is cached:** the next connection runs the capture again.

The reason texts are `LoginEnvFailure::text()`:
- "no login shell is set";
- "SHELL is /bin/tcsh, which can't be run as an interactive login shell to read it";
- "the login shell /bin/zsh couldn't start: …";
- "it took longer than 10 s";
- "the login shell exited with status 1 before printing it: <first stderr line>";
- "it was larger than 256 KiB".

**(Flag: a visible line, rather than only a log line, is recommended for `railway R6`. An `exec tmux` in an interactive `~/.bashrc` is the likely way to meet it: tmux fails with no terminal, and the line then names the exit and tmux's message.)**

### 8. The wire: one additive field on the relay link; no protocol version

```rust
pub struct RelayHost {
    …,
    /// The login shell's environment, when the relay was asked for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<LoginEnv>,
}
#[serde(rename_all = "snake_case")]
pub enum LoginEnv { Captured(BTreeMap<String, String>), Failed(String) }
// {"login":{"captured":{"HOME":"/root",…}}}  or  {"login":{"failed":"it took longer than 10 s"}}
```

- **Which versions meet.** The relay link isn't negotiated. The relay is always Brindle's own release: the launch script execs only a helper whose `--version` is Brindle's, and the client refuses a `RELAY_HOST` whose `version` differs (`client.rs:1647`). So the `--login-environment` argument never reaches an older relay. The field is still additive (`process R5`): without it, `RELAY_HOST` encodes byte for byte as before, so `relay-link.hex` is unchanged. A new fixture, `relay-host-login.hex`, pins both forms (`process R6`), and `fuzz.rs` gets the forms as seeds. `BTreeMap` keeps the keys sorted.
- **Client protocol 4 and H1 3 are not amended.** `AGENT_RESTART.env` is already "the agent's complete environment; the server adds nothing". The bug was that the client sent an incomplete one.
- **Security.** The environment already travelled to the client in `RELAY_HOST.env` and back in each `SPAWN`. The login environment adds whatever the startup files export, tokens included, over the same SSH link. Brindle never logs `RELAY_HOST` or a spawn's `env`, and the dump never prints them. Task 3.3 checks that the client's log lines don't.

### 9. Where each part goes

| Part | Kind | Crate / file |
| --- | --- | --- |
| `start_of`, `EndpointEnv`, `merged`, `Fallback` | pure (whole module) | `brindle`: `src/remote/local/endpoint_env.rs` |
| `invocation`, `encode_dump`, `parse_dump`, `captured`, `with_basics`, `LoginEnvFailure` | pure (whole module) | `brindle-remote-agent`: `server/login_env.rs` |
| `RelayHost.login`, `LoginEnv` | codec | `brindle-remote-protocol`: `relay.rs` |
| capture thread: spawn, `setsid`, `dup2`, read, kill, reap | shell, Linux seam | `remote-agent/src/server/relay.rs` |
| `login-env` role | shell | `remote-agent/src/main.rs` (already on the `ports R4` row) and a small `server/relay.rs` entry |
| passwd user name | seam item | `remote-agent/src/pty/unix.rs` (`passwd_entry`, which gains the name) |
| `encode` using `start_of`/`merged`; the fallback line; handing the note to an agent link | shell | `src/remote/local/connection.rs`, `src/remote/mod.rs` (`AgentLink`) |
| the zone's endpoint note and its dump line | model | `src/agent/model.rs` |
| `--login-environment` in the SSH launch script | pure string | `src/remote/client.rs` (`ssh_launch_script`) |

No crate reference changes direction. The helper still names no `brindle-ui`, and the app never names the helper.

### 10. Core restarts, crashes and upgrades

No server state is added. The environment is part of each spawn request, and a running program keeps its own.
- **A core restart** adopts sessions whose processes already have their environment (`process R2`).
- **A Brindle crash** leaves nothing to restore. The next connection starts a new relay, which reads the environment again.
- **An upgrade through a relay** (`reach()`'s step-down) runs while the capture runs. The capture is joined before the client's connection, whichever core answers.
- **A restart queued during a reconnect** is encoded with the host of the link it goes out on, so a reconnect that read a different environment applies the new one. That is what a new tab would get then.

### 11. Tests, by layer (`testing`)

- **Unit, `endpoint_env.rs`:**
  - `start_of` for every `ToCore` variant;
  - `merged` for every row of decision 5's table;
  - a profile's `env` over the login environment, and `TERM` over it in a terminal;
  - a restart's environment equal to a `SPAWN_AGENT`'s for the same profile and host;
  - `encode` of `AgentRestart` with a host, next to the existing `encode` test (`connection.rs:3456`).
- **Unit, `login_env.rs`:**
  - the invocation for each named shell, a path with spaces and quotes (fish quoting included), and the unsupported and empty cases;
  - `parse_dump` with the terminator, no terminator, past either bound, a non-UTF-8 record and an empty name;
  - the `_`/`SHLVL`/`PWD`/`OLDPWD` rule;
  - `with_basics` for set, empty and missing values and for no passwd entry;
  - each failure's text.
- **Transcript:** `relay-host-login.hex`, with `relay-link.hex` unchanged.
- **Real-role, `remote-agent/tests/relay.rs`**, each with a private `HOME`, `SHELL=/bin/sh` and the test's own `.profile`:
  - captured, with noise on stdout and stderr and a background `sleep` holding fd 3;
  - timed out under `BRINDLE_TEST_RELAY_LOGIN_ENV_MS=300`: the group is gone, a log line is written and the relay still relays;
  - a missing shell;
  - an unsupported one (`/bin/tcsh` named, not run);
  - not asked, where the existing tests stay as they are.
- **Real-role, `remote-agent/tests/agents.rs`:** an `AGENT_RESTART` agent's recorded environment (`record-env`) is exactly the request's. This pins that the core adds nothing, so the merge stays the client's.
- **Docker transport scenarios** (`BRINDLE_REMOTE_DOCKER_TEST=1`, `client.rs`'s `docker` module):
  - over SSH, `~/.profile` exports a variable and prepends a `PATH` directory, and `~/.bashrc` exports one below Ubuntu root's `[ -z "$PS1" ] && return` guard. A session with a command sees all three. A session with no command sees the directory once in `PATH`.
  - the existing noisy `~/.profile` and `~/.bashrc` scenarios also check that a command's session gets the login environment.
  - in the Ubuntu container, `/root/.profile` exports a variable that a command's session doesn't see.
- **e2e, `remote-agent-login-terminal`:**
  - before connecting, the case appends `export BRINDLE_E2E_PROFILE=1` to the fixture's `/root/.profile`, and `export BRINDLE_E2E_BASHRC=1` and `export PATH=/root/.local/agent-bin:$PATH` to `/root/.bashrc`;
  - it installs the static fake agent only in `/root/.local/agent-bin`, and the profile names it bare;
  - the first and second agent scripts each start with `record-env`;
  - it checks that the second (restarted) agent's file has `HOME=/root`, both login variables and that `PATH`, and that each equals the first agent's and the login program's value.
- **Manual:** the user's own run, Antigravity's terminal login over SSH to `A8`, with `agy` on the login `PATH` only.

### 12. Rules relied on

No exception is needed. The design relies on these rules:
- `fcis R1`–`R3`: every decision is in the two new pure modules, which take the shell, the helper's path, the dump bytes, the passwd entry and the environments as arguments.
- `ports R4` and `R7`: the process code stays in the relay's Linux seam, which decides nothing.
- `process R1`: the holder is untouched.
- `process R2`: the change adds no core state.
- `process R5` and `R6`: one additive field, a new fixture, fuzz seeds.
- `process R7`: the shell's output never reaches the framed stdout.
- `process R20`: one timing switch.
- `railway R1`, `R5`, `R6` and `R8`: the failure is a value, bounded, shown on each program it affects and named with its cause.
- `railway R3` and `types R2`: `start_of` has no wildcard.
- `types R1`, `R6` and `R7`: `Login::NotAsked` is a variant, parsed at the boundary.

## As built (2026-11-25)

The smallest fixes where the design met the code:
- **`busybox`** as `SHELL` is run as `busybox sh -l -i -c …`, since a multi-call binary takes the applet's name first. `ash`, the usual Alpine `SHELL`, is in the POSIX family as designed.
- **The failure's stderr line** is the last line that says something, not the first, and the kept 4 KiB are the last ones. An interactive bash with no terminal always starts its stderr with "cannot set terminal process group" and "no job control in this shell", so the first line would never name the cause. An rc's `exec tmux` then reads "…before printing it: open terminal failed: not a terminal". This answers the Open Question for bash; busybox's warnings are handled the same way. The quoted line is cut to 300 characters.
- **`TooLarge`'s text** names both bounds: "it was larger than 256 KiB or had more than 4096 variables".
- **A test's limit** is shown in milliseconds when it isn't whole seconds ("it took longer than 300 ms").
- **The capture's process** is the relay's own Linux seam code: a `poll` over the dump pipe, the stderr pipe and an exit pipe that the reaper thread closes once `wait` returns. When the shell has exited, both pipes are drained before the outcome is decided, so a dump written just before the exit is never lost to a race with the reaper.
- **The missing-shell test** names `/nonexistent/relay-test/bash`. A name that isn't in decision 3's table, such as the planned `/bin/relay-test-shell`, is `Unsupported` before it is looked for.
- **4.2's role test** parses the dump by hand, since the helper is a binary crate and its tests can't name `login_env`. The parser's own cases are `login_env`'s unit tests.
- **4.5's test** is Linux-only (`cfg(target_os = "linux")`), as remote endpoints are: a Windows program needs more than two variables to start.
- **5.1's `encode`** takes only the link's `EndpointEnv`, which replaced `Current`'s `RelayHost` (nothing else read it), and returns the frame with its `Fallback`. The match on `start_of` lives in `endpoint_env::over_endpoint`, so the shell only calls it.
- **5.3's record** is a map of its own in the connection's state (`State::fallbacks`, by request id), not a field of `Pending`, set or cleared each time the writer writes the start, so a start written again on a later link has that link's word. `SPAWNED` takes it out and tells it before the reply, `ERROR` drops it.
- **5.4's delivery** goes through a new `SessionSink::endpoint_note`, which a terminal's sink leaves as its dim note line and an agent's sink sends as `AgentEvent::EndpointNote`. A new agent's note comes with its `SPAWNED`, a restart's with its `OK`. The model keeps it beside the restore's note (`ChatModel::notes`), the view shows both, and the dump prints a `--- note` line for each.
- **3.3's check** found one `Debug` print that could carry a `RelayHost`: the reader's line for a relay frame after `HOST`. It now names the frame (a second `RELAY_HOST`, or a `RELAY_ERROR` with its message) and prints no host.

## Risks / Trade-offs

- [Startup files run once more per connection, with their side effects (an `ssh-agent` per connect, a `fortune`)] → Each SSH login does the same, and the output is discarded. A user can guard with `[ -t 0 ]`.
- [An interactive rc that `exec`s tmux or another shell] → The capture fails with the exit and its stderr's first line, and the visible line names it. Programs get today's environment.
- [Interactive-only variables (`PROMPT_COMMAND`, aliases are not exported) reach agents] → Exported variables only, and they're the user's. A profile's `env` can override any of them.
- [The first tab on a connection waits for the shell (up to 10 s)] → It overlaps reaching the core, it happens once per connection, and the limit bounds the worst case.
- [A secret exported by a startup file now travels to the client] → It travels over the same SSH link the session's own output does. It is never logged or dumped, and it is only sent back to the same endpoint.
- [A program that is itself a shell (`command = "zsh"`) re-runs `~/.zshrc` over the login environment] → That is the same as starting `zsh` from a login shell. Only `PATH` prefixes added in an rc file repeat.
- [fish's handling of extra arguments to `-c` differs between versions] → The path is quoted into fish's command instead (decision 3), and unit tests pin the quoting.

## Migration Plan

Nothing to migrate: no config key, state file or protocol version changes. The helper and Brindle ship together, as always. Rolling back is a revert. A reverted Brindle never passes `--login-environment`, and a reverted helper is never launched by a newer Brindle.

## Open Questions

- Whether `busybox sh -l -i -c` on old Alpine prints job-control warnings that hide its real exit reason in the fallback text. This only changes the wording of a failure, and the Docker scenario on Alpine will show it.

**Follow-ups, not part of this change:** these would be proposal-only changes, which the user hasn't asked for.
- A Windows Brindle could refresh a new session's environment from the registry (`CreateEnvironmentBlock`), as Windows Terminal does, so that a `PATH` changed by an installer reaches new tabs without restarting Brindle.
- A Linux Brindle started outside a login session could read a login shell's environment the same way.
- An opt-in profile key for a container's login environment.
