# Tasks

## 1. Parsing and marks (`brindle-term-core`)

- [ ] 1.1 Rename `parse_osc133` to `parse_mark` accepting `133` and `633` for `A`–`D`, and verify unit tests for 633 marks, ignored kinds, and a doubled 133 + 633 prompt making one command (`cargo test -p brindle-term-core shell_integration`)
- [ ] 1.2 Add the VS Code escape decoder, `parse_command_line`, and `633;P;Cwd=` in `reported_dir`; verify unit tests for decoding, the nonce, controls, overflow, and both path styles (UNC rejected on Windows style)
- [ ] 1.3 Add `Split::CommandLine` with `scanner.line`, `Command::line` and `Marks::set_line` (ignored once `C` is in); verify unit tests for `E` before and after `C`
- [ ] 1.4 Write `633;E` after `B` in `replay::render`, update its fidelity notes, and verify an emulator round trip keeps escaped lines
- [ ] 1.5 Add `Marks::last_command_line` returning `LineSource`, and verify unit tests for a reported line, a span, and marks gone from the scrollback

## 2. Server core

- [ ] 2.1 Verify with a `remote-agent/tests/core.rs` test that an attach after `633;E` replays the line (no core code change expected beyond the shared emulator)

## 3. Client and UI

- [ ] 3.1 Add `Terminal::last_command_line_text`, `TerminalView::copy_last_command`, and the `copy_last_command` action ("Copy Last Command Line") in `actions.rs`; verify `cargo test -p brindle` and that `--list-actions` lists it
- [ ] 3.2 Add e2e cases `shell-633-marks`, `shell-633-copy-line`, `shell-633-cwd` (`tests/e2e/cases/shell.rs`) and `local-server-633-attach` (`local_server.rs`); verify `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- 633`

## 4. Shipped scripts

- [ ] 4.1 `brindle.fish`: send the escaped `633;E` from `fish_preexec` before `C`; verify `shell-script-line-fish` (a line with `;`) passes
- [ ] 4.2 `brindle.zsh`: send it from `preexec` before `C`; verify `shell-script-line-zsh` (with `RPROMPT`, and a two-line command) passes
- [ ] 4.3 `brindle.ps1`: send it in the `PSConsoleHostReadLine` wrapper before `C`; verify `windows-pwsh-line` in `cases/win.rs` on the Windows CI job, and that the existing PowerShell cases still pass
- [ ] 4.4 `brindle.bash`: record the newest history number in `__brindle_prompt` and add `$(__brindle_line)` to `PS0`, sending `E` only when history advanced; check and comment `HISTTIMEFORMAT` and multi-line (`cmdhist`/`lithist`) behaviour; verify `shell-script-line-bash`, including the `HISTCONTROL=ignoreboth` fallback, and that the existing bash cases still pass

## 5. Docs

- [ ] 5.1 Update `docs/code-map.md` (parsing and shell integration, `brindle-term-core`) and README.md (supported sequences, the new command, the scripts now reporting command lines), and verify `cargo test --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings`

## Workflow follow-up

- Archive the change on the implementation PR once its checks pass.
- If `add-blocks` lands first, or after: point its header at `Command::line` in whichever change lands second.
