# Design

## Context

`TextInput` (`src/text_input.rs`) binds its keys in the `TextInput` key context (`src/actions.rs`). ctrl-a is bound to `TextInputHome` and ctrl-shift-a to `TextInputSelectHome`, the readline pair to ctrl-e and ctrl-shift-e. `EditState::select_all` and `TextInput::select_all` already exist; the find bar calls them to select its restored query when it reopens.

## Goals / Non-Goals

**Goals:** ctrl-a selects all text in every text input.

**Non-Goals:** changing ctrl-a in terminals, where it must keep reaching the program as `^A`. Making the select-all key configurable beyond the usual user bindings.

## Decisions

### Decision 1: ctrl-a selects all; home keeps moving to the start

The user expects ctrl-a to select all. Home already moves to the start, so losing the readline ctrl-a costs only a second key for it. ctrl-e keeps moving to the end: it doesn't conflict with anything, and anyone used to ctrl-e still has it.

Alternative considered: keep readline ctrl-a and add select-all on another key. Rejected because the request is specifically ctrl-a, and desktop text boxes, which these overlays look like, use ctrl-a for select all.

### Decision 2: ctrl-shift-a selects all too

ctrl-shift-a was "select to the start", the shifted readline ctrl-a. With ctrl-a no longer moving to the start, that pairing no longer makes sense. ctrl-shift-a is the terminal's select-all key (`SelectAll` in the `Terminal` context), so making it select all in text inputs gives one key that selects all in both places. shift-home still selects to the start.

Alternative considered: leave ctrl-shift-a as select-to-start. Rejected because it would be an orphan of a pairing that no longer exists, and would do something different from the same key in the terminal.

### Decision 3: a new unlabelled action

`text_input_select_all` joins the other `text_input_*` actions: unlabelled, so it stays out of the command palette, and runnable with `--action` for e2e checks.

## Risks / Trade-offs

- Users with readline habits lose ctrl-a as "start of line" in overlays → home still does it, and the fields are single short lines, so the cost is small.
- The "Readline ends" scenario now checks ctrl-e, the readline key that remains.
