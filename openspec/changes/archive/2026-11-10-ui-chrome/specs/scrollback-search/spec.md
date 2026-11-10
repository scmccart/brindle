# Spec Delta

## ADDED Requirements

### Requirement: Find bar controls
The find bar's match-case and regular-expression options SHALL be toggles that show pressed while the option is on. The previous-match, next-match and close buttons and both toggles SHALL show a tooltip with their name and shortcut: "Match case (alt-c)", "Regular expression (alt-r)", "Previous match (shift-enter)", "Next match (enter)" and "Close (escape)", using the keys the keymap currently binds.

#### Scenario: Toggle shows its state
- **WHEN** the user clicks the match-case toggle
- **THEN** it is drawn pressed and the search reruns with match case on

#### Scenario: Close tooltip
- **WHEN** the pointer rests on the bar's close button
- **THEN** a tooltip reads "Close" followed by the key bound to `find_close`

#### Scenario: Invalid expression
- **WHEN** regex is on and the query is not a valid expression
- **THEN** the query field is framed in the destructive colour and the bar says the expression is invalid
