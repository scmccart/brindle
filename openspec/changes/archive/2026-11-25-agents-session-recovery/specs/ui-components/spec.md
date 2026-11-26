# Spec Delta

## ADDED Requirements

### Requirement: Prompt input reason action
Given an action label, a prompt input SHALL show a button with that label after its reason line's text. The button SHALL be a keyboard stop right after the textarea and SHALL ask the owner when pressed; it SHALL never change the textarea's text. Without a label, no button SHALL show and it SHALL be no stop.

#### Scenario: Action after the reason
- **WHEN** the owner gives a disabled prompt input the reason `the agent has exited` and the action `Continue conversation`, and the user presses Tab in the textarea, then enter
- **THEN** the button has focus after the Tab, the enter asks the owner for the action, and the textarea's text is unchanged

#### Scenario: Action taken away
- **WHEN** the owner takes the action away
- **THEN** no button shows after the reason, and Tab from the textarea doesn't reach one
