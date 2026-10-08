# Spec Delta

## Purpose

Understand the OSC 133 semantic prompt marks that shells emit, so users can jump between prompts, select or copy a command's output, and see at a glance which commands failed, in local tabs and tmux control-mode panes alike.

## ADDED Requirements

### Requirement: Semantic prompt marks recognised
A terminal SHALL recognise `ESC ] 133 ; <kind> [; params] ST`, ended by BEL or `ESC \`, where kind `A` is prompt start, `B` command start, `C` output start and `D[;<exit>]` command finished. A mark SHALL be recorded at the cursor position reached by all output before it, even when the sequence is split across reads. Other OSC 133 kinds and marks received on the alternate screen SHALL be ignored, and no mark SHALL change what is drawn in the grid.

#### Scenario: Sequence split across reads
- **WHEN** a shell writes `ESC ] 1 3` in one read and `3 ; A BEL` in the next, after printing two lines
- **THEN** a prompt mark is recorded at the start of the third line

#### Scenario: Both terminators
- **WHEN** output contains `ESC ]133;D;2 ESC \` after an output mark
- **THEN** the command is recorded as finished with exit status 2

#### Scenario: Marks are invisible
- **WHEN** a shell emits OSC 133 marks around its prompt
- **THEN** nothing extra appears in the terminal's text

### Requirement: Commands built from marks
Marks SHALL be grouped into commands. `A` starts a new command, except that an `A` on the same line as the latest command that has no output mark yet is treated as a prompt redraw and replaces it. `B` and `C` attach to the latest command. `D` finishes the latest command only if it has an output mark and is not finished yet. Otherwise it SHALL be ignored, so pressing enter on an empty line never inherits an old exit status.

#### Scenario: Empty command line
- **WHEN** the shell emits `D;1` after a prompt where the user pressed enter without typing a command (no `C`)
- **THEN** no command is marked as failed

#### Scenario: Failed command
- **WHEN** the shell emits `A`, `B`, `C`, then output, then `D;1`
- **THEN** that command is recorded with exit status 1

### Requirement: Marks follow the text
A mark SHALL stay on its text line as lines scroll into the scrollback and as the oldest lines are dropped. A mark whose line leaves the scrollback, is cleared from it (`CSI 3 J`, `clear`, Clear Scrollback) or is wiped by a full reset SHALL be forgotten. Resizing the window SHALL move marks with the reflowed text, including while a full-screen program has the alternate screen.

#### Scenario: Output scrolls the prompt into history
- **WHEN** a prompt is marked and the command then prints 500 lines
- **THEN** jumping to that prompt shows the prompt's line at the top of the view

#### Scenario: Scrollback cleared
- **WHEN** the user runs `clear` (which clears the scrollback) at a prompt
- **THEN** only the new prompt's mark remains

#### Scenario: Narrower window
- **WHEN** a long command line wraps after the window is made narrower
- **THEN** its prompt mark is still on the prompt's first row

### Requirement: Prompt navigation
**Jump to Previous Prompt** SHALL scroll the view so that the nearest prompt above the top visible line becomes the top line. **Jump to Next Prompt** SHALL do the same with the nearest prompt below the top line, scrolling no further than the live screen, and SHALL go to the bottom when there is none. They SHALL be bound to ctrl-shift-up and ctrl-shift-down. In a terminal with no prompt marks, the keys SHALL scroll one line instead.

#### Scenario: Previous prompt
- **WHEN** the view is at the bottom after a command printed several screens of output, and the user presses ctrl-shift-up
- **THEN** the view scrolls so that command's prompt line is at the top

#### Scenario: No shell integration
- **WHEN** the shell emits no OSC 133 marks and the user presses ctrl-shift-up
- **THEN** the view scrolls up one line, as before

#### Scenario: Next prompt reaches the end
- **WHEN** the view is scrolled up to the latest prompt and the user presses ctrl-shift-down
- **THEN** the view returns to the bottom

### Requirement: Last command output
**Select Last Command Output** SHALL select the text of the most recent command that has an output mark. The output runs from the `C` mark up to the `D` mark, or, if the command is not finished, up to the next prompt or the cursor. The selection SHALL also become the primary selection. **Copy Last Command Output** SHALL put the same text on the clipboard and leave any existing selection unchanged. Neither SHALL do anything when that output is empty or has left the scrollback.

#### Scenario: Copy output
- **WHEN** the user ran `printf 'one\ntwo\n'`, and runs Copy Last Command Output at the next prompt
- **THEN** the clipboard holds `one` and `two` on two lines, and the prompt line is not included

#### Scenario: Select output
- **WHEN** the user runs Select Last Command Output after `ls`
- **THEN** exactly the lines `ls` printed are highlighted

### Requirement: Failed command mark
When `mark_failed_commands` is on (the default), a terminal SHALL draw a thin vertical bar in the theme's red (ANSI color 1) at the left edge of the grid, beside the rows from a command's prompt start to its command start, for every visible command that finished with a non-zero exit status. The bar SHALL NOT cover text in a padded terminal, and it SHALL scroll with its prompt.

#### Scenario: Failure marked
- **WHEN** the user runs `false` and then `true`
- **THEN** a red bar appears beside the `false` prompt and none beside the `true` prompt

#### Scenario: Turned off
- **WHEN** `mark_failed_commands = false`
- **THEN** no bars are drawn

### Requirement: Shipped shell scripts
Brindle SHALL ship integration scripts for bash, zsh and fish that emit `A`, `B` (where the shell allows it), `C` and `D;<exit status>`. It SHALL print them with `brindle --shell-integration <shell>`, and they SHALL also be readable as files in the source tree. A script SHALL do nothing in a non-interactive shell, SHALL keep `$?` and any existing prompt hooks working, and SHALL be safe to load twice. Brindle SHALL NOT change shells' startup files or inject the scripts itself.

#### Scenario: bash opt-in
- **WHEN** a bash user adds `eval "$(brindle --shell-integration bash)"` to `~/.bashrc`
- **THEN** new shells emit OSC 133 marks, and `$?` and an existing `PROMPT_COMMAND` still work

#### Scenario: Unknown shell
- **WHEN** the user runs `brindle --shell-integration tcsh`
- **THEN** an error listing the supported shells is printed and the exit status is 2

### Requirement: tmux panes
In tmux control-mode panes, marks the pane's shell emits SHALL work like they do in local tabs, because tmux passes pane output through to control clients unchanged. Marks are only recorded from the time Brindle receives output, so restoring a pane on attach SHALL NOT recreate marks for earlier output. In classic tmux, which does not pass OSC 133 to the outer terminal, Brindle SHALL see no marks.

#### Scenario: Control-mode pane
- **WHEN** a shell with the bash script loaded runs in a control-mode pane and prints several screens of output
- **THEN** ctrl-shift-up scrolls that pane to the command's prompt

#### Scenario: Reattach
- **WHEN** Brindle reattaches to a session whose panes had marked prompts
- **THEN** the restored text has no marks until the shells print new prompts
