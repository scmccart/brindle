## MODIFIED Requirements

### Requirement: Working directory reports
In local tabs, Brindle SHALL record the working directory a program reports with OSC 7 (`file://<host>/<path>`, percent-decoded) or with ConEmu's OSC 9;9 (`ESC ] 9 ; 9 ; <path> ST`, the path optionally in double quotes). The latest report wins. A report whose path is not absolute for the platform SHALL be ignored. On Windows, `file://host/C:/src` SHALL mean `C:\src`, and only a drive-letter path (`X:\…` or `X:/…`) with no further `:`, no `"` and no control character SHALL count as absolute: a UNC path (`\\host\share`), a device or extended path (`\\?\…`, `\\.\…`) and an OSC 7 path that would become one SHALL be ignored. While it runs, Brindle SHALL NOT check whether a reported directory exists on the UI thread, so a slow or unreachable path never freezes the window. A directory recorded for session restore that isn't absolute by the same rule SHALL be ignored when restoring, so the tab starts in the default directory.

#### Scenario: OSC 7 on Windows
- **WHEN** a program in a Windows tab prints `ESC ] 7 ; file://pc/C:/My%20Files ESC \`
- **THEN** the tab's reported directory is `C:\My Files`

#### Scenario: OSC 9;9
- **WHEN** a program prints `ESC ] 9 ; 9 ; "C:\src" ESC \`
- **THEN** the tab's reported directory is `C:\src`, and no notification is shown

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
