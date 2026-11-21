# Spec Delta

## MODIFIED Requirements

### Requirement: Semantic prompt marks recognised
A terminal SHALL recognise `ESC ] 133 ; <kind> [; params] ST`, ended by BEL or `ESC \`, where kind `A` is prompt start, `B` command start, `C` output start and `D[;<exit>]` command finished. It SHALL recognise `ESC ] 633 ; <kind> [; params] ST` with kind `A`, `B`, `C` or `D[;<exit>]` as the same mark. A mark SHALL be recorded at the cursor position reached by all output before it, even when the sequence is split across reads. Other OSC 133 and OSC 633 kinds and marks received on the alternate screen SHALL be ignored, and no mark SHALL change what is drawn in the grid.

#### Scenario: Sequence split across reads
- **WHEN** a shell writes `ESC ] 1 3` in one read and `3 ; A BEL` in the next, after printing two lines
- **THEN** a prompt mark is recorded at the start of the third line

#### Scenario: Both terminators
- **WHEN** output contains `ESC ]133;D;2 ESC \` after an output mark
- **THEN** the command is recorded as finished with exit status 2

#### Scenario: Marks are invisible
- **WHEN** a shell emits OSC 133 marks around its prompt
- **THEN** nothing extra appears in the terminal's text

#### Scenario: VS Code marks
- **WHEN** a shell emits `ESC ] 633 ; A BEL`, `ESC ] 633 ; B BEL`, then after enter `ESC ] 633 ; C BEL`, output, and `ESC ] 633 ; D ; 1 BEL`
- **THEN** a command with exit status 1 is recorded, and Jump to Previous Prompt reaches its prompt

#### Scenario: Other 633 kinds
- **WHEN** a shell emits `ESC ] 633 ; F BEL` or `ESC ] 633 ; P ; IsWindows=True BEL`
- **THEN** no mark is recorded

### Requirement: Working directory reports
In local tabs, Brindle SHALL record the working directory a program reports with OSC 7 (`file://<host>/<path>`, percent-decoded), with ConEmu's OSC 9;9 (`ESC ] 9 ; 9 ; <path> ST`, the path optionally in double quotes), or with VS Code's OSC 633 `P` property `Cwd` (`ESC ] 633 ; P ; Cwd=<path> ST`, with `\\` and `\xHH` escapes decoded). The latest report wins. A report whose path is not absolute for the platform SHALL be ignored. On Windows, `file://host/C:/src` SHALL mean `C:\src`, and only a drive-letter path (`X:\…` or `X:/…`) with no further `:`, no `"` and no control character SHALL count as absolute: a UNC path (`\\host\share`), a device or extended path (`\\?\…`, `\\.\…`) and an OSC 7 path that would become one SHALL be ignored. While it runs, Brindle SHALL NOT check whether a reported directory exists on the UI thread, so a slow or unreachable path never freezes the window. A directory recorded for session restore that isn't absolute by the same rule SHALL be ignored when restoring, so the tab starts in the default directory.

#### Scenario: OSC 7 on Windows
- **WHEN** a program in a Windows tab prints `ESC ] 7 ; file://pc/C:/My%20Files ESC \`
- **THEN** the tab's reported directory is `C:\My Files`

#### Scenario: OSC 9;9
- **WHEN** a program prints `ESC ] 9 ; 9 ; "C:\src" ESC \`
- **THEN** the tab's reported directory is `C:\src`, and no notification is shown

#### Scenario: OSC 633 Cwd
- **WHEN** a program in a Linux tab prints `ESC ] 633 ; P ; Cwd=/home/me/a\x3bb BEL`
- **THEN** the tab's reported directory is `/home/me/a;b`

#### Scenario: Linux path in a Windows tab
- **WHEN** a WSL shell in a Windows tab reports `file://pc/home/user`
- **THEN** the report is ignored, and the tab's reported directory is unchanged

#### Scenario: Network path reported on Windows
- **WHEN** a program in a Windows tab whose reported directory is `C:\src` prints `ESC ] 9 ; 9 ; \\host\share ESC \`
- **THEN** the report is ignored, the tab's reported directory stays `C:\src`, and Brindle makes no connection to `host`

#### Scenario: Network path through OSC 7 on Windows
- **WHEN** a program in a Windows tab prints `ESC ] 7 ; file://x//host/share ESC \` or `ESC ] 9 ; 9 ; \\?\UNC\host\share ESC \`
- **THEN** the report is ignored

#### Scenario: Network path recorded by an older build
- **WHEN** Brindle on Windows restores a tab whose recorded directory is `\\host\share`
- **THEN** the tab starts in the default directory, and Brindle makes no connection to `host`

#### Scenario: Unreachable reported directory
- **WHEN** a Windows tab's reported directory is on a mapped drive that has stopped answering
- **THEN** the window keeps painting and taking input, and a new tab opened meanwhile starts in Brindle's default directory instead

### Requirement: Shipped shell scripts
Brindle SHALL ship integration scripts for bash, zsh, fish and PowerShell that emit `A`, `B` (where the shell allows it), `C` and `D;<exit status>`, and, just before `C`, the command line with OSC 633 `E` (see Shipped scripts report command lines). The PowerShell script SHALL work in PowerShell 7 (`pwsh`) and Windows PowerShell 5.1, and SHALL also report the working directory with OSC 7 at each prompt. Brindle SHALL print the scripts with `brindle --shell-integration <shell>`, and they SHALL also be readable as files in the source tree. A script SHALL do nothing in a non-interactive shell, SHALL keep `$?` and any existing prompt hooks working, and SHALL be safe to load twice. Brindle SHALL NOT change shells' startup files or inject the scripts itself.

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

### Requirement: Shipped scripts report command lines
Each shipped script SHALL send the command line as typed with OSC 633 `E` before `C`, escaping `\` as `\\` and `;` and control characters as `\xHH`, and SHALL send none for an empty line. The bash script SHALL send it only when the command added a new history entry, so a command history leaves out (`HISTCONTROL`, `HISTIGNORE`, history off) sends none rather than an older one.

#### Scenario: fish
- **WHEN** a fish user with the script loaded runs `echo "a;b"`, and then runs Copy Last Command Line
- **THEN** the clipboard holds `echo "a;b"`

#### Scenario: zsh with a right prompt
- **WHEN** a zsh user whose theme sets `RPROMPT` runs `ls -la`, and then runs Copy Last Command Line
- **THEN** the clipboard holds `ls -la`, without the right prompt's text

#### Scenario: Multi-line command
- **WHEN** a zsh user runs a command typed over two lines with a trailing `\`
- **THEN** the reported line holds both lines joined by a newline

#### Scenario: bash command left out of history
- **WHEN** a bash user with `HISTCONTROL=ignoreboth` runs `make`, then ` ls` with a leading space, and then runs Copy Last Command Line
- **THEN** the clipboard holds `ls`, read from the screen between the marks, not `make`

#### Scenario: PowerShell
- **WHEN** a PowerShell user with the script loaded runs `Get-ChildItem C:/`, and then runs Copy Last Command Line
- **THEN** the clipboard holds `Get-ChildItem C:/`

### Requirement: Reported command lines
`ESC ] 633 ; E ; <line> [; <nonce>] ST` SHALL set the command line of the latest command, if it has no `C` mark yet. `\\` SHALL decode to `\`, and `\xHH` to that byte. A `C` mark (OSC 133 or 633) with a `cmdline_url=<percent-encoded line>` parameter, as fish 4 writes its own, SHALL set the line of the command whose output it starts, `%XX` decoded. A line that is not UTF-8 after decoding, contains a control character other than a newline, or arrives in a sequence over 4096 bytes SHALL be ignored. The nonce SHALL be ignored, and a reported line SHALL never be run.

#### Scenario: Escaped line
- **WHEN** a shell emits `A`, `B`, then `ESC ] 633 ; E ; echo a\x3bb ; 1f2e BEL` and `C`
- **THEN** that command's line is `echo a;b`

#### Scenario: Too late
- **WHEN** a shell emits `A`, `B`, `C`, and only then `ESC ] 633 ; E ; ls BEL`
- **THEN** that command has no reported line

#### Scenario: Line on the output mark
- **WHEN** fish 4 emits `A`, then `ESC ] 133 ; C ; cmdline_url=echo%20%22a%3Bb%22 ESC \`, and its script's `E` arrives only after that
- **THEN** that command's line is `echo "a;b"`

### Requirement: Copy Last Command Line
**Copy Last Command Line** SHALL put on the clipboard the reported line of the most recent command that has one of: a reported line, or both `B` and `C` marks. Without a reported line it SHALL copy the text from the `B` mark to the `C` mark with leading and trailing whitespace removed. It SHALL do nothing when there is no such command or its marks have left the scrollback.

#### Scenario: Reported line wins
- **WHEN** a shell reported `git commit -m "x"` with `633;E` for a command whose prompt line wrapped across two rows, and the user runs Copy Last Command Line
- **THEN** the clipboard holds `git commit -m "x"`

#### Scenario: From the marks
- **WHEN** a shell emitting only OSC 133 ran `ls -la`, and the user runs Copy Last Command Line
- **THEN** the clipboard holds `ls -la`
