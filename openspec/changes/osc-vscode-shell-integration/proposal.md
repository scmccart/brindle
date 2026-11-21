# Proposal

## Why

VS Code's shell integration (OSC 633) is a superset of OSC 133, and some shell setups emit only 633: VS Code's own scripts, shells started with VS Code's environment, and tools that copy its protocol. In Brindle those shells get no prompt marks, so prompt jumps, Copy Last Command Output and the failed-command mark all stop working. 633 also carries what 133 doesn't: the exact command line (`633;E`), which Brindle can't reliably read back from the grid once a prompt wraps or a line editor redraws it.

## What Changes

- Treat `OSC 633 ; A|B|C|D[;exit]` exactly as the matching OSC 133 marks.
- Record `OSC 633 ; E ; <command line> [; <nonce>]` (with VS Code's `\\` and `\xHH` escapes) as the latest command's command line.
- Accept `OSC 633 ; P ; Cwd=<path>` as a working-directory report, under the same rules as OSC 7 and OSC 9;9.
- Add a **Copy Last Command Line** command (`copy_last_command`), which copies the most recent command's reported line, or its text between the `B` and `C` marks when none was reported.
- Keep the command lines in the terminal server, so they survive detach, attach and core restarts.
- Ignore the other 633 kinds (`F`, `G`, `H`, other `P` properties) and never act on the nonce.
- Make all four shipped scripts (bash, zsh, fish, PowerShell) send `633;E` with the line as typed, just before `C`. That matters most for fish, whose script sends no `B`, so the line can't be read from the screen; and for zsh and fish, where the screen holds right prompts and autosuggestions. bash reads the line from history and sends it only when the command added a history entry.

Out of scope: the blocks header using the reported line (once `add-blocks` lands, its header can take it from the same field).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `shell-integration`: 633 marks, reported command lines, `P;Cwd`, the Copy Last Command Line command, and the shipped scripts reporting command lines.
- `terminal-server`: reported command lines survive attach.

## Impact

- `term-core/src/shell_integration.rs` (the marks scanner and `Command`, which gains `line: Option<String>`), `term-core/src/replay.rs`.
- `shell-integration/brindle.{bash,zsh,fish,ps1}`.
- `src/actions.rs`, `src/terminal_view.rs` (the new command), `docs/code-map.md`, README.md, `tests/e2e/cases/shell.rs`.
- No wire change, no new dependency. Old clients ignore 633 in a replay.
