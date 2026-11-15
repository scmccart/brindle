## MODIFIED Requirements

### Requirement: Command filtering
A command list SHALL show a filter input above its items, which may be grouped under headings. Typed text SHALL filter items by a case-insensitive subsequence match on their labels, keeping their order. Groups left with no items SHALL be hidden, and a message SHALL show when nothing matches. Each change to the filter SHALL highlight the first match. When its owner replaces the items, the filter text SHALL be kept, and the item the owner names SHALL be highlighted if it matches the filter and is enabled. Otherwise the first match SHALL be highlighted.

#### Scenario: Filtering
- **WHEN** a command list holds `Split Right` and `Close Tab`, and the user types `spr`
- **THEN** only `Split Right` is listed and it is highlighted

#### Scenario: Nothing matches
- **WHEN** the user types text that matches no item
- **THEN** no group heading is shown and the empty message is

#### Scenario: Items replaced
- **WHEN** a command list holds `alpha`, `beta` and `gamma` with `beta` highlighted, and its owner replaces them with `alpha`, `beta`, `delta` and `gamma`, naming `beta`
- **THEN** `beta` is still highlighted, and the filter text is unchanged

#### Scenario: Named item filtered out
- **WHEN** the filter is `ga`, and the owner replaces the items naming one that doesn't match `ga`
- **THEN** the first item matching `ga` is highlighted
