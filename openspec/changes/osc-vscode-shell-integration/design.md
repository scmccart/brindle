# Design

## Context

`brindle-term-core`'s `shell_integration` module has its own scanner (not `osc::Scanner`). It keeps up to `OSC_KEEP` (4096) bytes of each OSC body and splits the vte processor's input at points where the term must be inspected (`Split::Mark`, `Split::Cwd`, …). `osc_split` tries `parse_osc133`, then `reported_dir` (OSC 7 and 9;9, by the terminal's `PathStyle`), and holds a found directory in `scanner.cwd` until `Parser::advance` applies `Split::Cwd`. `Marks` groups marks into `Command { prompt, input, output, end, exit }`, and `last_output` turns marks into grid points. `Terminal::last_output_text` then reads the text. `replay::render` writes marks back as `133;A/B/C/D` at their positions. The same code runs in the client and in the server core's `Emulator`.

## Goals / Non-Goals

**Goals:**
- 633 handled in the same module and the same pass as 133, so client and core can't diverge (`fcis R6`).
- The reported line is data only. Nothing executes or trusts it, so the nonce (VS Code's guard for its "run recent command") isn't needed.

**Non-Goals:**
- `633;P` properties other than `Cwd`, and `F`/`G`/`H` (VS Code's prompt-input regions and continuation markers).

## Decisions

### Parsing

`parse_osc133` becomes `parse_mark`, accepting a first field of `133` or `633` for `A`–`D`. Two new pure functions sit beside it:
- `parse_command_line(body) -> Option<String>`: `633;E;…`, decoding `\\` and `\xHH`, stopping at the first unescaped `;` (the nonce follows), and rejecting non-UTF-8 text or controls other than `\n`.
- `reported_dir` learns `633;P;Cwd=…` with the same decoder, then runs the existing `is_absolute_path` checks, so the Windows UNC and device-path rules apply unchanged.

An overflowed buffer (`self.overflow`) yields no command line, as it yields no directory today.

### A `Split::CommandLine`, held like `Cwd`

`Split` stays `Copy`. The decoded line waits in `scanner.line` and `Parser::advance` applies `Split::CommandLine` by calling `marks.set_line(line)`. `Marks::set_line` sets `Command::line` on the latest command when its `output` is `None`, and otherwise ignores it. This needs no grid position, so it doesn't matter where the split falls. `Command` gains `line: Option<String>` (≤ 4096 bytes by construction). The 10 000-command cap (`MAX_COMMANDS`) bounds the total.

Alternative considered: putting the line in `Mark` as `Mark::CommandLine(String)`. Rejected: `Mark` is `Copy` and positional, and every match on it would gain a variant that has no position.

### Replay

`replay::render` writes `ESC ] 633 ; E ; <escaped> ESC \` right after a command's `B` mark (or after its `A` when it has no `B`), escaping `\`, `;` and bytes below 0x20 as `\\` and `\xHH`. It keeps writing 133 for the marks themselves, so old clients still get their marks. Versions:
- old client, new core: its scanner ignores 633, so the marks arrive without lines;
- new client, old core: no lines, as the spec allows.

A core restart renders a checkpoint, so lines survive it and step-down upgrades (`process R1`–`R3`, `R12`). No protocol version changes (`process R4`).

### Copy Last Command Line

`Marks::last_command_line<T>(&self, term) -> Option<LineSource>`, with `enum LineSource { Reported(String), Span(Point, Point) }` (pure, in `shell_integration`). It picks the most recent command with a line or with both `input` and `output`. `Terminal::last_command_line_text` reads a `Span` from the grid as `last_output_text` does, trims trailing whitespace, and joins wrapped rows. `TerminalView` gets `copy_last_command` beside `copy_last_output`, and `actions.rs` gains `("copy_last_command", Some("Copy Last Command Line"), …)`, so it shows in the palette and can be bound. No default key.

### Shipped scripts send `633;E`

Each script gets one small escaping helper (`\` → `\\`, `;` → `\x3b`, newline and other bytes below 0x20 and 0x7F → `\xHH`) and sends `ESC ] 633 ; E ; <escaped> BEL` immediately before its `C`, from the hook that already sends `C`. It sends no nonce, since Brindle ignores it.

| Script | Source of the line | Notes |
|---|---|---|
| `brindle.fish` | `$argv[1]` of the `fish_preexec` event | fish sends no `B`, so this is the only way its line is known |
| `brindle.zsh` | `$1` of `preexec` (the line as typed, also when history drops it; empty only when history is off, which sends nothing) | right prompts and autosuggestions never enter the line |
| `brindle.ps1` | `$line` in the `PSConsoleHostReadLine` wrapper | same in pwsh and 5.1; `[Console]::Write` as for `C` |
| `brindle.bash` | the newest history entry, read in `PS0` | see below |

bash has no preexec. `PS0` (bash 4.4+, already needed for `C`) gains `$(__brindle_line)` before its `C`. That function runs in a command-substitution subshell, so it only reads state:
- `__brindle_prompt` records the newest history number at each prompt (`__brindle_hist`, from `fc -l -0`'s first field, or empty when history is off);
- `__brindle_line` reads `fc -l -0` again and prints the escaped `E` only when the number is greater than `__brindle_hist`.

A command that `HISTCONTROL` (`ignorespace`, `ignoredups`, `erasedups`), `HISTIGNORE` or `set +o history` keeps out of history adds no entry, so nothing is sent and Copy Last Command Line reads the screen. bash sends `B` and `C`, so that still works. The implementer checks that `fc -l` output is unaffected by `HISTTIMEFORMAT`, and how `cmdhist`/`lithist` store a multi-line command, and pins both in the script's comments. Alternatives considered: a `DEBUG` trap with `$BASH_COMMAND`. Rejected: it gives one simple command, not the line, and a `DEBUG` trap clashes with users' own and with bash-preexec.

The scripts keep their existing guarantees (`$?`, existing hooks, loading twice), which the `Shipped shell scripts` scenarios already check.

### Testing

- `shell_integration.rs` unit tests: 633 `A`–`D` and the ignored kinds; `E` decoding (`\\`, `\x3b`, a nonce, invalid escapes, controls, overflow); `E` after `C` ignored; `P;Cwd` on both path styles, including a UNC path rejected on Windows; `last_command_line` for both sources and for commands gone from the scrollback.
- `replay.rs` / `emulator.rs` round trip: lines survive a render, escaped.
- e2e (`cases/shell.rs`): `shell-633-marks` (prompt jump through the dump), `shell-633-copy-line` (the clipboard after `--action copy_last_command`, read as the clipboard cases do), `shell-633-cwd` (a new tab's directory), and `local-server-633-attach` in `local_server.rs`.
- e2e, one case per shipped script, each skipping when its shell isn't installed: `shell-script-line-bash` (a plain command, then `HISTCONTROL=ignoreboth` with a leading-space command falling back to the screen), `shell-script-line-zsh` (with an `RPROMPT`, and a two-line command), `shell-script-line-fish` (a line containing `;`), and on Windows `windows-pwsh-line` in `cases/win.rs`.

## Risks / Trade-offs

- [A shell emits both 133 and 633 for the same prompt (VS Code's scripts can, when nested)] → the second `A` on the same line is already treated as a prompt redraw, and duplicate `B`/`C` attach to the same command. Unit tests pin that duplicates make one command.
- [A long or hostile line] → 4096-byte cap, controls rejected, and the line is only ever copied, never run or shown unescaped in chrome.
