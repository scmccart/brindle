## ADDED Requirements

### Requirement: The relay reports a login shell's environment
`brindle-remote relay --login-environment` SHALL also report, before relaying, the environment of the user's login shell started as an interactive login shell, or why it couldn't be read. The shell's output SHALL never reach the relay's standard output. The relay SHALL wait at most 10 s for it, then end the shell's process group. Without the option, the relay SHALL run no shell and report as before.

#### Scenario: Login environment reported
- **WHEN** the user's `~/.profile` prints `hello` and exports `FOO=1`, and `brindle-remote relay --login-environment` is started
- **THEN** its report carries a login environment with `FOO=1`, its own environment has no `FOO`, and its standard output holds only its frame marker and frames

#### Scenario: The shell takes too long
- **WHEN** the user's `~/.profile` waits forever, and `brindle-remote relay --login-environment` is started
- **THEN** within about 10 s the relay reports that the login environment took too long, no process of that shell is left running, the core's log has a line saying so, and the relay relays as usual

#### Scenario: Not asked
- **WHEN** `brindle-remote relay` is started without `--login-environment`
- **THEN** it starts no shell, and its report carries no login environment
