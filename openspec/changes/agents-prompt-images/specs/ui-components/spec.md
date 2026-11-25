## ADDED Requirements

### Requirement: Prompt input attachment previews
An attachment item of a prompt input SHALL show, in place of its icon, a small picture the owner gives it, scaled whole into the icon's place. An item given a picture that can't be drawn SHALL show its icon. The picture SHALL change nothing else about the item: its name, detail and remove mark stay, and it is still not a keyboard stop.

#### Scenario: With a picture
- **WHEN** the owner gives an attachment item a picture
- **THEN** the item shows the picture where its icon would be, with its name and detail beside it

#### Scenario: A picture that can't be drawn
- **WHEN** the owner gives an attachment item bytes that aren't a picture
- **THEN** the item shows its icon
