## MODIFIED Requirements

### Requirement: Components draw from the active tokens
Every component SHALL take its colours, corner radius and text sizes only from the active token set: background, foreground, card, popover, primary, secondary, muted, accent, destructive (each with its foreground), border, input, ring and radius. The active token set SHALL be that of the innermost theme scope the component is drawn in, else the window's. Replacing the window's token set SHALL redraw every visible component outside a theme scope with the new values, with no other step.

#### Scenario: Switching to dark tokens
- **WHEN** the gallery shows a default button with the light token set active, and the dark token set is made active
- **THEN** the button is redrawn in the dark set's primary and primary-foreground colours

#### Scenario: Inside a scope
- **WHEN** the window's token set is the light one and a default button is drawn inside a theme scope of the dark set
- **THEN** the button is drawn in the dark set's primary and primary-foreground colours, and a button beside the scope in the light set's

### Requirement: Card colours and ring
A card's surface, text, muted text, border and ring colours SHALL come from its owner, and default to the card, foreground, muted foreground, border and ring tokens. The owner SHALL be able to turn the focus ring on, and the ring SHALL be drawn outside the card's border like a control's.

#### Scenario: Default colours
- **WHEN** a card is drawn without colours from its owner
- **THEN** its surface, text and border are the card, foreground and border tokens

#### Scenario: Owner colours
- **WHEN** the owner gives a card its own surface, text and border colours
- **THEN** the card and its header are drawn in them, whatever the active tokens are

#### Scenario: Ring outside the border
- **WHEN** the owner turns the ring on
- **THEN** a 3-pixel band in the ring colour is drawn just outside the border, taking no layout room

#### Scenario: Owner's ring colour
- **WHEN** a card is drawn inside a theme scope, its owner gives it the window set's ring colour, and turns the ring on
- **THEN** the ring is drawn in the window set's ring colour, not the scope's

## ADDED Requirements

### Requirement: Theme scopes
An owner SHALL be able to draw a part of its view inside a theme scope with a token set of its own. Every component in that part SHALL take its colours from the scope's set while it is laid out, prepainted and painted, including list items laid out only when they come into view. A scope inside another SHALL win inside it. Entering or leaving a scope SHALL NOT redraw or notify anything outside it, and components outside it SHALL keep the window's set.

#### Scenario: Nested scopes
- **WHEN** a button is drawn in a scope of set B inside a scope of set A, and another button in A's scope beside it
- **THEN** the first button is drawn in set B's colours and the second in set A's

#### Scenario: A list item scrolled into view
- **WHEN** a message scroller inside a scope of the dark set is scrolled so that an item comes into view
- **THEN** that item's bubble is drawn in the dark set's colours

#### Scenario: Paint-time colours
- **WHEN** a focused textarea is drawn inside a scope of the dark set with a selection
- **THEN** its cursor and selection are drawn in the dark set's colours

### Requirement: Tooltips and overlays in a theme scope
A tooltip on a component drawn inside a theme scope SHALL be drawn in the scope's token set, at the window's text size. An overlay its owner opens with a token set (a menu, a popover) SHALL be drawn in that set; one opened without a set SHALL be drawn in the window's.

#### Scenario: Tooltip inside a scope
- **WHEN** the pointer rests on a button drawn inside a scope of the dark set while the window's set is the light one
- **THEN** its tooltip is drawn in the dark set's colours and at the window's text size

#### Scenario: Menu with a token set
- **WHEN** an owner opens a context menu with the dark set while the window's set is the light one
- **THEN** the menu's surface, items and highlight are drawn in the dark set's colours
