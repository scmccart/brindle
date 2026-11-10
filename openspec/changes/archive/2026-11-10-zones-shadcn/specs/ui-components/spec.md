## ADDED Requirements

### Requirement: Cards
A card SHALL be a bordered, rounded surface with an optional header row above its content, after shadcn's Card. The header SHALL hold a title, an optional leading adornment (a status dot) and an optional action slot at its end, and be separated from the content by a border line. The content SHALL fill the rest of the card with no padding of its own. A card SHALL NOT know of terminals.

#### Scenario: Header optional
- **WHEN** a card is drawn without a header
- **THEN** its content fills the whole card inside the border

### Requirement: Card colours and ring
A card's surface, text, muted text and border colours SHALL come from its owner, and default to the card, foreground, muted foreground and border tokens. The owner SHALL be able to turn the focus ring on, and the ring SHALL be drawn outside the card's border like a control's.

#### Scenario: Default colours
- **WHEN** a card is drawn without colours from its owner
- **THEN** its surface, text and border are the card, foreground and border tokens

#### Scenario: Owner colours
- **WHEN** the owner gives a card its own surface, text and border colours
- **THEN** the card and its header are drawn in them, whatever the active tokens are

#### Scenario: Ring outside the border
- **WHEN** the owner turns the ring on
- **THEN** a 3-pixel band in the ring colour is drawn just outside the border, taking no layout room

### Requirement: Rounded corners over content
A card SHALL keep its rounded corners when its content paints to the corners, by painting the area outside the rounded edge in a surround colour given by its owner, above the content and below the border.

#### Scenario: Content with a square background
- **WHEN** a card's content fills its box with a square background
- **THEN** the card's corners show the surround colour outside the rounded border and none of the content's background
