## MODIFIED Requirements

### Requirement: Confirmations
A confirmation SHALL show a title and, while requested, one small button per action the owner gives, each a keyboard stop activated by enter or space or a primary click. Once the owner marks it accepted, rejected or cancelled, it SHALL show that outcome with the chosen action's label and no buttons. An owner SHALL be able to give it the destructive tone, which while requested draws a warning icon in the destructive colour before the title, a background tinted with the destructive colour and a border in it; answered, it SHALL be drawn in the default tone.

#### Scenario: Requested
- **WHEN** a confirmation is requested with actions `Allow once` and `Reject`
- **THEN** both buttons show, and pressing space on the focused `Reject` button asks the owner for `Reject`

#### Scenario: Answered
- **WHEN** the owner marks a confirmation accepted with `Allow once`
- **THEN** it shows `Allow once` as the outcome and no buttons

#### Scenario: Destructive tone
- **WHEN** a requested confirmation has the destructive tone
- **THEN** it shows the warning icon, the tinted background and the destructive border, and its actions keep the variants the owner gave them

#### Scenario: Destructive tone once answered
- **WHEN** the owner marks a destructive confirmation rejected with `Reject`
- **THEN** it shows the outcome in the default tone, with no icon, tint or destructive border

## ADDED Requirements

### Requirement: Loaders
A loader SHALL be a `loader-circle` icon sized like the status dots it stands in for, turned to the step its owner gives, one of 12 steps a turn, so the owner's clock and not the component decides when it moves. It SHALL take no focus and SHALL NOT ask for animation frames itself.

#### Scenario: Owner's step
- **WHEN** an owner draws a loader at step 3 and then at step 4
- **THEN** the icon is turned a quarter of a turn, then a twelfth of a turn further

#### Scenario: No frames of its own
- **WHEN** a loader is drawn and its owner gives no new step
- **THEN** nothing redraws the window for it

### Requirement: Shimmering text
A shimmer SHALL draw one line of text in the muted foreground with a lighter band, up to the foreground colour, centred at the position its owner gives, from before the text's start to past its end. Its text SHALL never be dimmer than the muted foreground. It SHALL take no focus and SHALL NOT ask for animation frames itself.

#### Scenario: The band moves with the owner's position
- **WHEN** an owner draws `Working…` with the band at its start, then at its middle
- **THEN** the first characters are the lightest, then the middle ones, and the rest are in the muted foreground

#### Scenario: Outside the text
- **WHEN** the band's position is before the text's start
- **THEN** every character is in the muted foreground
