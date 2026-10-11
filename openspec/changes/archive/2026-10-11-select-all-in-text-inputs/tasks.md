# Tasks

## 1. Select all

- [x] 1.1 Add `TextInputSelectAll` to `src/actions.rs` (`text_input_select_all`, unlabelled) and bind ctrl-a and ctrl-shift-a to it in the `TextInput` context, replacing `TextInputHome` and `TextInputSelectHome` on those keys. Handle it in `TextInput` with the existing `select_all`. Verify with `cargo build` and `brindle --list-actions | grep text_input_select_all`.
- [x] 1.2 Add an `EditState` unit test for the spec scenario: select all on `main`, then insert `work`, gives `work`. Verify with `cargo test text_input`.
- [x] 1.3 Add an e2e case `input-select-all`: open the command palette, type `splt`, move left, run `text_input_select_all`, type `tab`, and check the dumped input holds `tab` with the cursor at 3. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- input-select-all`.
