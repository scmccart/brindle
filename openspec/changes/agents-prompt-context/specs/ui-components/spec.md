## ADDED Requirements

### Requirement: Prompt input attachments
Given attachment items, a prompt input SHALL show them as a wrapping row above its textarea and below its queue row: each an icon the owner chooses, a name cut with an ellipsis, an optional detail, and a remove mark. A primary press on a remove mark SHALL ask the owner to remove that item, by its key. The items SHALL NOT be keyboard stops, and a prompt input given none SHALL show no row.

#### Scenario: Items
- **WHEN** the owner gives a prompt input items `cargo test` with the detail `12 KiB` and `main.rs`
- **THEN** a row above the textarea shows both, the first with its detail

#### Scenario: Remove asks the owner
- **WHEN** the user presses the remove mark of `main.rs`
- **THEN** the owner is asked to remove that item, and the row is unchanged until the owner changes it

#### Scenario: Tab skips them
- **WHEN** the textarea has focus and the user presses tab
- **THEN** focus doesn't land on an attachment item
