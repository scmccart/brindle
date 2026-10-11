# Proposal

## Why

Text inputs follow readline: ctrl-a moves the cursor to the start of the text. In practice the user reaches for ctrl-a to select a whole query or field and retype it, as in a browser or a desktop text box. The find bar's restored query, a palette filter and a settings field all want "select everything, type over it", and today that takes shift-home from the end or a drag. There is no select-all key at all.

## What Changes

- **BREAKING:** ctrl-a in a text input selects all its text instead of moving to the start. Home still moves to the start, and ctrl-e still moves to the end.
- ctrl-shift-a selects all too, matching the terminal's select-all key, instead of selecting to the start. shift-home still selects to the start.
- A new `text_input_select_all` action, unlabelled like the other `text_input_*` actions.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `text-input`: ctrl-a leaves the cursor-movement keys, and a select-all requirement is added.

## Impact

- `src/actions.rs`: the new action and the ctrl-a and ctrl-shift-a bindings in the `TextInput` context.
- `src/text_input.rs`: the action handler, using the existing `select_all`.
- Every text input changes at once: the command and new-tab palettes, palette prompts, the settings dialog and the find bar. Terminals are not affected; ctrl-a still reaches the shell as `^A`.
