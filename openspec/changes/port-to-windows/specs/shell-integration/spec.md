# Spec Delta

## MODIFIED Requirements

### Requirement: Shipped shell scripts
Brindle SHALL ship integration scripts for bash, zsh, fish and PowerShell that emit `A`, `B` (where the shell allows it), `C` and `D;<exit status>`. The PowerShell script SHALL work in PowerShell 7 (`pwsh`) and Windows PowerShell 5.1, and SHALL also report the working directory with OSC 7 at each prompt. Brindle SHALL print the scripts with `brindle --shell-integration <shell>`, and they SHALL also be readable as files in the source tree. A script SHALL do nothing in a non-interactive shell, SHALL keep `$?` and any existing prompt hooks working, and SHALL be safe to load twice. Brindle SHALL NOT change shells' startup files or inject the scripts itself.

#### Scenario: bash opt-in
- **WHEN** a bash user adds `eval "$(brindle --shell-integration bash)"` to `~/.bashrc`
- **THEN** new shells emit OSC 133 marks, and `$?` and an existing `PROMPT_COMMAND` still work

#### Scenario: Unknown shell
- **WHEN** the user runs `brindle --shell-integration tcsh`
- **THEN** an error listing the supported shells is printed and the exit status is 2

#### Scenario: PowerShell opt-in
- **WHEN** a PowerShell user adds `brindle --shell-integration pwsh | Out-String | Invoke-Expression` to their `$PROFILE`
- **THEN** new shells emit OSC 133 marks and an OSC 7 report of the current directory, and a custom `prompt` function they defined earlier still draws the prompt

#### Scenario: PowerShell exit status
- **WHEN** a PowerShell user with the script loaded runs `cmd /c exit 3`
- **THEN** the command's mark records exit status 3, and `$LASTEXITCODE` is still 3 at the next prompt

## ADDED Requirements

### Requirement: Working directory reports
In local tabs, Brindle SHALL record the working directory a program reports with OSC 7 (`file://<host>/<path>`, percent-decoded) or with ConEmu's OSC 9;9 (`ESC ] 9 ; 9 ; <path> ST`, the path optionally in double quotes). The latest report wins. A report whose path is not absolute for the platform SHALL be ignored. On Windows, `file://host/C:/src` SHALL mean `C:\src`.

#### Scenario: OSC 7 on Windows
- **WHEN** a program in a Windows tab prints `ESC ] 7 ; file://pc/C:/My%20Files ESC \`
- **THEN** the tab's reported directory is `C:\My Files`

#### Scenario: OSC 9;9
- **WHEN** a program prints `ESC ] 9 ; 9 ; "C:\src" ESC \`
- **THEN** the tab's reported directory is `C:\src`, and no notification is shown

#### Scenario: Linux path in a Windows tab
- **WHEN** a WSL shell in a Windows tab reports `file://pc/home/user`
- **THEN** the report is ignored, and the tab's reported directory is unchanged
