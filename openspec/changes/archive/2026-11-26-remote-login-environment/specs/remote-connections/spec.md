## MODIFIED Requirements

### Requirement: Remote programs get the endpoint's environment
A remote program SHALL get its endpoint's environment with the profile's variables over it. Over SSH, a program Brindle runs directly (a profile's command, an agent, a login zone's program, an agent started again) SHALL get the user's login shell's environment, and a session with no command SHALL run the login shell with the SSH environment. In a container, programs SHALL get the `exec` environment. A missing working directory SHALL become the user's home there.

#### Scenario: Missing directory
- **WHEN** a restored remote zone recorded `/srv/old`, which no longer exists on its host
- **THEN** its new shell starts in the user's home directory on that host

#### Scenario: Host environment
- **WHEN** `ubuntu`'s environment over SSH on `A8` has `LANG=en_GB.UTF-8`, the profile sets `env = { FOO = "1" }`, and the user runs `env` in a new tab
- **THEN** the output has `LANG=en_GB.UTF-8` and `FOO=1`, and no variable that exists only in Brindle's own environment

#### Scenario: A command gets the login shell's environment
- **WHEN** `~/.profile` on `A8` adds `~/.local/bin` to `PATH` and exports `EDITOR=vim`, `agy` is installed only in `~/.local/bin`, and the user opens an agent profile on `A8` whose command is `agy acp`
- **THEN** the agent starts, and its environment has `EDITOR=vim` and a `PATH` that includes `~/.local/bin`

#### Scenario: Interactive startup files count
- **WHEN** `~/.bashrc` on `A8` loads nvm below its "not interactive, return" guard, and a terminal profile on `A8` runs `node --version`
- **THEN** the version printed is nvm's Node

#### Scenario: An agent started again gets a fresh start's environment
- **WHEN** an agent on `A8` is started again in its session, after a terminal login or by Continue conversation
- **THEN** it runs with the same `HOME`, `USER`, `PATH` and login variables as the same profile's agent in a new tab, and it doesn't fail for want of `HOME`

#### Scenario: A login shell session builds on the SSH environment
- **WHEN** `~/.profile` on `A8` prepends `~/.local/bin` to `PATH`, and the user opens a terminal profile on `A8` with no command and runs `echo $PATH`
- **THEN** `~/.local/bin` appears in it once

#### Scenario: The profile's variables win
- **WHEN** `~/.profile` on `A8` exports `MODEL=large`, and an agent profile on `A8` sets `env = { MODEL = "small" }`
- **THEN** the agent runs with `MODEL=small`

#### Scenario: A container keeps its image's environment
- **WHEN** a container's image sets `PATH` to include `/usr/local/go/bin`, its `/etc/profile` resets `PATH`, and a container profile runs `go version`
- **THEN** `go` is found, and nothing from `/etc/profile` or `~/.profile` is in the program's environment

#### Scenario: A container with no HOME
- **WHEN** a container's `exec` sets no `HOME` for the profile's user, whose passwd entry has home `/home/dev` and shell `/bin/bash`
- **THEN** a program run there has `HOME=/home/dev`, `USER` and `LOGNAME` set to the user's name, and `SHELL=/bin/bash`

## ADDED Requirements

### Requirement: An SSH endpoint's login environment is read once per connection
Each time Brindle connects to an SSH endpoint, the user's login shell SHALL be run once there, as an interactive login shell with no terminal, and its environment read, without anything its startup files print reaching the connection. Reading SHALL take at most 10 s, after which the shell is ended. When it can't be read, each program that would have used it SHALL get the SSH environment and SHALL say why, in its terminal or its agent zone.

#### Scenario: Startup files that print
- **WHEN** `~/.profile` and `~/.bashrc` on `A8` each print a line to standard output and to standard error, and the user opens an agent profile on `A8`
- **THEN** the connection works, the agent gets the login shell's environment, and none of those lines appear in any tab or zone

#### Scenario: A slow startup file
- **WHEN** `~/.profile` on `A8` waits 60 s before finishing, and the user opens a terminal profile on `A8` whose command is `htop`
- **THEN** within about 10 s `htop` runs with the SSH environment, and its terminal shows one dim line saying the login shell's environment on `A8` wasn't read because it took longer than 10 s

#### Scenario: A shell Brindle can't run this way
- **WHEN** the user's shell on `A8` is `/bin/tcsh`, and the user opens an agent profile on `A8`
- **THEN** the agent runs with the SSH environment, and the zone shows a note naming `tcsh` as the reason the login shell's environment wasn't read

#### Scenario: A startup file starts a background program
- **WHEN** `~/.profile` on `A8` starts `ssh-agent`, which keeps running
- **THEN** the login shell's environment is read without waiting for `ssh-agent` to exit, and programs on `A8` get it

#### Scenario: A reconnect reads it again
- **WHEN** the user adds `export FOO=2` to `~/.profile` on `A8` while connected, and the connection is lost and comes back
- **THEN** programs started on `A8` after the reconnect have `FOO=2`, and programs already running keep the environment they started with

#### Scenario: A shell session doesn't show the line
- **WHEN** the login shell's environment on `A8` couldn't be read, and the user opens a terminal profile on `A8` with no command
- **THEN** its login shell starts with the SSH environment, and its terminal shows no line about the login environment
