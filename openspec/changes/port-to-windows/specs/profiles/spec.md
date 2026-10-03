# Spec Delta

## MODIFIED Requirements

### Requirement: Profile definition
A profile SHALL be able to set:
- a name;
- a command and its arguments. The default on Linux is `$SHELL`, then `/bin/sh`. On Windows it is `pwsh.exe` when it is on `PATH`, then `powershell.exe`, then `%COMSPEC%`, then `cmd.exe`;
- a working directory, with `~` expanded;
- environment variables;
- a theme;
- a font size;
- a tmux mode (`none`, `plain` or `control`), with a session name (default `main`) and extra tmux arguments.

There SHALL always be at least one profile; a "Shell" profile is created when the config defines none. Profiles without a name SHALL be named `Profile N`.

#### Scenario: No profiles configured
- **WHEN** the config defines no profiles
- **THEN** a single "Shell" profile running the user's shell is available

#### Scenario: Default shell on Windows
- **WHEN** a profile sets no command on a Windows machine that has PowerShell 7 installed
- **THEN** its tabs run `pwsh.exe`

#### Scenario: Windows without PowerShell 7
- **WHEN** a profile sets no command and `pwsh.exe` is not on `PATH`
- **THEN** its tabs run `powershell.exe`
