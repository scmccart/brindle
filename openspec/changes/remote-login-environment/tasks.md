# Tasks

No client protocol or H1 version changes (design.md decision 8). The relay link gets one additive field. Group 1 is the shared contract, groups 2–3 are pure code, groups 4–5 are the shell, and group 6 is the e2e case.

## 1. Shared contract: the relay link

- [x] 1.1 `brindle-remote-protocol` `relay.rs`: add `LoginEnv::{Captured(BTreeMap<String, String>), Failed(String)}` (snake_case, externally tagged) and `RelayHost.login: Option<LoginEnv>`, with `serde(default, skip_serializing_if = "Option::is_none")`. Correct `RelayHost.env`'s doc: it is the relay's own environment with the passwd basics, not "a fresh login's". Update the module doc's table row and add a unit round trip for both forms and for none. Verify with `cargo test -p brindle-remote-protocol relay`.
- [x] 1.2 Add the transcript fixture `remote-protocol/tests/fixtures/relay-host-login.hex` (a `RELAY_HOST` with a captured login, then one with a failed login), written only when missing, and add both forms to `fuzz.rs`'s relay seeds. Verify with `cargo test -p brindle-remote-protocol --test transcripts`, the fuzz tests, and `relay-link.hex` passing unchanged.

## 2. Pure code in the helper

- [x] 2.1 New `remote-agent/src/server/login_env.rs`, ungated, with a lint-only `cfg_attr(not(target_os = "linux"), allow(dead_code))`. It holds `invocation(shell, helper)` per design.md decision 3's table (positional `$0` for the POSIX family, fish's quoting, `Unsupported`, `NoShell`) and `LoginEnvFailure` with `text()`. Unit tests: each named shell by file name (`/usr/bin/zsh`, `/bin/busybox`), a helper path with spaces, `'` and `\`, `tcsh`, `nu`, `""`, and every failure's text. Verify with `cargo test -p brindle-remote-agent server::login_env`.
- [x] 2.2 In `login_env.rs`, add:
  - `encode_dump(vars)`: `NAME=VALUE\0` records and the empty terminator;
  - `parse_dump(bytes)`: complete, incomplete or too large, with at most 256 KiB and 4096 variables, and non-UTF-8, empty-name and no-`=` records dropped;
  - `captured(own, dump)`: `_`, `SHLVL`, `PWD` and `OLDPWD` taken from `own`;
  - `with_basics(env, passwd)`: `HOME`, `USER`, `LOGNAME` and `SHELL` filled only when unset or empty.

  Unit tests for each case, plus an encode-then-parse round trip. Verify with `cargo test -p brindle-remote-agent server::login_env`.
- [x] 2.3 Add `remote-agent/src/server/login_env.rs` to fcis's "Whole pure modules" table and to `PURE_MODULES` in `tests/architecture.rs`. Verify with `cargo test -p brindle --test architecture`.

## 3. Pure code in Brindle

- [ ] 3.1 New `src/remote/local/endpoint_env.rs` with:
  - `Start::{LoginShell, Program}`;
  - `start_of(&ToCore)`, naming every variant with no `_ =>` arm;
  - `EndpointEnv` and `Login::{NotAsked, Captured, Failed}`, built from a `RelayHost` (`EndpointEnv::from_host`);
  - `merged(endpoint, start, own) -> Merged { env, fallback }` per decision 5's table, and `Fallback`'s line text.

  Unit tests: `start_of` for every `ToCore` variant; every row of the table; a profile's `env` over the login environment, and `TERM` over it for a terminal; a fallback only for `Program` with `Failed`; `Fallback`'s text through the control filter. Verify with `cargo test -p brindle remote::local::endpoint_env`.
- [ ] 3.2 Remove `relay::relay_env`. Move its test (`the_endpoints_environment_is_under_the_profiles`) to `endpoint_env.rs` as a `merged` test, and update `src/remote/local/relay.rs`'s module doc. Add `src/remote/local/endpoint_env.rs` to fcis's table and `PURE_MODULES`. Verify with `cargo test -p brindle remote::local` and `cargo test -p brindle --test architecture`.
- [ ] 3.3 Add a unit test that, for one remote agent profile and one `EndpointEnv`, the `SPAWN_AGENT` from `agent_spawn_request` and the `AGENT_RESTART` from `.restart()` merge to the same environment. Check that no log line in `src/remote/` or `src/agent/` prints a `RelayHost` or a spawn's `env`, and that none of those types reach a `log::` call through `Debug`. Verify with `cargo test -p brindle agent::spawn remote::local::endpoint_env`, and by grepping for `log::` calls near `RelayHost` and `env`.

## 4. The helper's shell

- [x] 4.1 Extend `pty::passwd_entry` (`remote-agent/src/pty/unix.rs`) to return the user name with the shell and home. In `Reached::host`, the relay's own environment goes through `login_env::with_basics` before it is sent. Verify with `cargo test -p brindle-remote-agent`, and in `remote-agent/tests/relay.rs` with a relay started with `HOME` and `SHELL` removed from its environment: its `RELAY_HOST.env` has both from passwd.
- [x] 4.2 Add the hidden `login-env` role in `remote-agent/src/main.rs`, declared under `target_os = "linux"` beside `relay`. It refuses arguments, writes `encode_dump(vars_os())` to fd 3 and exits 0, and it is left out of the usage text. Verify with a real-role test in `remote-agent/tests/relay.rs`: run `brindle-remote login-env` with fd 3 a pipe and a set environment, then parse the pipe with `login_env::parse_dump`. With no fd 3, the role exits non-zero.
- [x] 4.3 In `server/relay.rs`, `relay [--login-environment]` (any other argument is a usage error) starts the capture thread after the marker, per decision 3's process rules:
  - the invocation;
  - stdin and stdout `/dev/null`, stderr a pipe with 4 KiB kept, fd 3 the dump pipe;
  - `setsid`, the home directory, and the filled environment.

  It joins the capture before `server.connect()`, at most 10 s after the capture started (`BRINDLE_TEST_RELAY_LOGIN_ENV_MS` overrides this). On timeout it sends `SIGKILL` to the process group and reaps the shell on a thread. It sets `RelayHost.login`, and writes one `core.log` line on failure with no values in it. Update the module doc's step list. Verify with 4.4's tests.
- [x] 4.4 Add real-role tests in `remote-agent/tests/relay.rs`, each with a private `HOME`, `SHELL=/bin/sh` and its own `.profile`:
  - captured: `.profile` prints to stdout and stderr, exports `RELAY_LOGIN_VAR=1` and leaves `sleep 30 &` holding fd 3. `login` is `Captured` with the variable, `env` lacks it, a session through the relay round-trips, and the relay's stdout held only the marker and frames.
  - timed out: `.profile` runs `sleep 60`, with `BRINDLE_TEST_RELAY_LOGIN_ENV_MS=300`. `Failed` arrives within a bound, no process of that group remains, `core.log` names it, and the relay still relays.
  - a missing shell (`SHELL=/bin/relay-test-shell`);
  - an unsupported one (`SHELL=/bin/tcsh`, named and not run);
  - not asked: existing tests unchanged, with `login` absent.

  Verify with `cargo test -p brindle-remote-agent --test relay`.
- [x] 4.5 Add a real-role test in `remote-agent/tests/agents.rs`: an `AGENT_RESTART` with `env = {HOME=/x, ONLY=1}` runs the fake agent, whose `record-env` file holds exactly those variables. That shows the core adds nothing. Verify with `cargo test -p brindle-remote-agent --test agents`.
- [x] 4.6 Update `docs/code-map.md`'s relay entry and the terminal server's management or roles entry: the capture, its limit and switch, the `login-env` role, and the passwd basics. Verify by reading the entries against 4.1–4.3.

## 5. Brindle's shell

- [ ] 5.1 Rewrite `encode` (`src/remote/local/connection.rs`) as `match (endpoint_env::start_of(message), host)`, re-encoding `SPAWN`, `SPAWN_AGENT` and `AGENT_RESTART` with `merged`'s environment. Keep the relay link's `EndpointEnv` (built once from `RELAY_HOST`) beside its `RelayHost`. Extend the existing `encode` test with an `AgentRestart` and a `SpawnAgent` that each get the host's environment, a `Spawn` with no command that gets `exec`'s, and one with a command that gets the login environment. Verify with `cargo test -p brindle remote::local::connection`.
- [ ] 5.2 In `ssh_launch_script` (`src/remote/client.rs`), exec `"$1" relay --login-environment`, and leave the container launch unchanged. Update the `SSH_LAUNCH` constant test and the function's doc. Verify with `cargo test -p brindle remote::client`.
- [ ] 5.3 For the fallback line in a terminal, record `Merged.fallback` with the spawn's request, and when its `SPAWNED` arrives, write `Fallback`'s line into that terminal before the program's output, as `lost_line` is written. Write a unit test of the line, and a connection test with a fake relay host whose `login` is `Failed`: the terminal's first output is the line, and a no-command `SPAWN` gets none. Verify with `cargo test -p brindle remote::local::connection`.
- [ ] 5.4 For the fallback note in an agent zone, pass the `Fallback` text for a `SPAWN_AGENT` or `AGENT_RESTART` to the zone through `AgentLink`'s sink. The model keeps it as an endpoint note shown above the conversation, beside a restore's note, and the dump adds a `--- note "…"` line. Write unit tests in `src/agent/model.rs` for the note's text and its dump line. Verify with `cargo test -p brindle agent::model`.
- [ ] 5.5 Update `CLAUDE.md`: the `--- note` line covers the endpoint note too, and the Manual checks list gains "an agent's terminal login over SSH with the agent on the login shell's `PATH` only, the restarted agent working". Update `docs/code-map.md`'s `remote/` entry: the program's environment, `endpoint_env`, the fallback line and note. Verify by reading both against 5.1–5.4.
- [ ] 5.6 Extend the Docker transport scenarios (`src/remote/client.rs`'s `docker` module):
  - over SSH, `~/.profile` exports `BRINDLE_LOGIN_PROFILE=1` and prepends `/root/login-bin` to `PATH`, and `~/.bashrc` exports `BRINDLE_LOGIN_RC=1` after Ubuntu root's interactive guard. A session with a command prints all three, and a session with no command shows `/root/login-bin` once in `PATH`.
  - the noisy `~/.profile` and `~/.bashrc` scenarios also check a command session's login variable.
  - in the Ubuntu container, `/root/.profile` exports a variable that a command session doesn't see, and the session's `HOME` is `/root`.

  Rebuild the static helper first (README's Docker command). Verify with `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client`.

## 6. End-to-end

- [ ] 6.1 Extend `remote-agent-login-terminal` (`tests/e2e/cases/remote.rs`):
  - before connecting, append `export BRINDLE_E2E_PROFILE=1` to the fixture's `/root/.profile`, and `export BRINDLE_E2E_BASHRC=1` and `export PATH=/root/.local/agent-bin:$PATH` to `/root/.bashrc`;
  - copy the static fake agent only to `/root/.local/agent-bin/brindle-fake-acp-agent`, and name it bare in the profile's command;
  - start both agent scripts with `record-env` (`/root/first.env`, `/root/second.env`).

  The case checks:
  - the second (restarted) agent's file has `HOME=/root`, both login variables and `PATH` starting `/root/.local/agent-bin`;
  - each of those equals the first agent's value and the login program's (`/root/login.env`);
  - the existing checks still pass.

  Verify with `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_REMOTE_E2E_REQUIRED=1 cargo test --test e2e -- remote-agent-login-terminal`, run inside `systemd-run --user --scope` as the e2e memory says, and by seeing it fail on `main`'s `encode` (reverted locally) before 5.1.
- [ ] 6.2 Integration check:
  - `cargo test --workspace --locked`;
  - `cargo clippy --locked --workspace --all-targets -- -D warnings`;
  - `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_REMOTE_E2E_REQUIRED=1 cargo test --test e2e -- remote- agent-`.

  Verify by all three passing. Then hand the user the manual check from 5.5: Antigravity's terminal login over SSH to `A8`, with `agy` only on the login shell's `PATH`.

## Workflow follow-up

- Archive the change in the implementation PR (`docs(openspec): archive remote-login-environment`), syncing the delta specs.
- After merge, offer to reinstall the user's dogfooding build (`scripts/install.sh`).
