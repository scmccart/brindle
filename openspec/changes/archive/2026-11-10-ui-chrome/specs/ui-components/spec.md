# Spec Delta

## ADDED Requirements

### Requirement: Tooltips
A tooltip SHALL show a short label, and optionally a shortcut, beside a control after the pointer has rested on it for 500 ms. It SHALL hide when the pointer leaves the control, when the control is pressed, and on Escape. A tooltip SHALL NOT take focus, SHALL NOT receive pointer events, and SHALL NOT appear on a disabled control.

#### Scenario: Hover shows it
- **WHEN** the pointer rests on a control that has a tooltip for 500 ms
- **THEN** the tooltip appears beside the control with its label

#### Scenario: Press hides it
- **WHEN** a tooltip is showing and the user presses the control
- **THEN** the tooltip hides and the control still activates

#### Scenario: Shortcut shown
- **WHEN** a tooltip is given a shortcut
- **THEN** the shortcut is drawn after the label as keycaps

### Requirement: Keyboard shortcut display
A shortcut SHALL be drawn as one keycap per key, using the key names the keymap prints (`ctrl`, `shift`, `alt`, `cmd`, a letter or a named key), in the muted foreground on the muted background. A shortcut of several strokes SHALL show a gap between the strokes.

#### Scenario: Chord
- **WHEN** a shortcut `ctrl-shift-t` is shown
- **THEN** three keycaps read `ctrl`, `shift` and `t`

### Requirement: Separators
A separator SHALL be a one-pixel rule in the border colour, horizontal or vertical, spanning its container. It SHALL NOT take focus or pointer events and SHALL NOT be a stop in list or menu navigation.

#### Scenario: Skipped by navigation
- **WHEN** a menu highlights the item above a separator and the user presses down
- **THEN** the item below the separator is highlighted

### Requirement: Toggles
A toggle SHALL show a pressed or unpressed state set by its owner, and SHALL ask the owner to flip it on a primary click, and on enter or space while it has focus. A pressed toggle SHALL be drawn with the accent background and foreground, an unpressed one transparent, with the muted background on hover, as shadcn's toggle. A disabled toggle SHALL be drawn muted and SHALL NOT ask. Without a focus handle a toggle SHALL NOT take focus on a press.

#### Scenario: Click flips it
- **WHEN** the user clicks an unpressed toggle
- **THEN** the owner is asked to press it, and the toggle shows pressed once the owner does

#### Scenario: Disabled toggle
- **WHEN** the user clicks a disabled toggle
- **THEN** the owner is not asked and focus does not move

### Requirement: Context menus
A context menu SHALL open on a secondary pointer press on its target, placed with its top-left corner at the pointer, flipped and clamped to stay inside the window. It SHALL list items, each with a label, an optional shortcut and an optional disabled state, and separators between groups. Opening it SHALL move the highlight to nothing; the first down key highlights the first enabled item.

#### Scenario: Opens at the pointer
- **WHEN** the user presses the secondary button at (200, 40) on a target with a menu
- **THEN** the menu appears with its corner at (200, 40)

#### Scenario: Near the window edge
- **WHEN** the pointer is within a menu's width of the window's right edge
- **THEN** the menu opens to the left of the pointer and stays inside the window

### Requirement: Context menu keys and selection
In an open context menu, up and down SHALL move the highlight through the enabled items without wrapping, home and end to the first and last, and typing printable characters to the next enabled item whose label starts with the typed text. Enter, space and a primary click on an enabled item SHALL select it, tell the owner and close the menu. Moving the pointer over an enabled item SHALL highlight it. A disabled item SHALL be skipped and SHALL NOT be selected.

#### Scenario: Enter selects
- **WHEN** a menu is open, the user presses down twice and enter
- **THEN** the second enabled item is selected and the menu closes

#### Scenario: Disabled item
- **WHEN** the user clicks a disabled item
- **THEN** nothing is selected and the menu stays open

#### Scenario: Release of the opening press
- **WHEN** the user releases the secondary button within 10px of where it was pressed to open the menu
- **THEN** no item is selected and the menu stays open

#### Scenario: One gesture
- **WHEN** the user presses the secondary button to open the menu, moves more than 10px onto an enabled item and releases
- **THEN** that item is selected and the menu closes

### Requirement: Context menu dismissal
Escape SHALL close an open context menu. A pointer press outside it SHALL close it and SHALL NOT reach what is beneath. Closing SHALL return focus to what had it when the menu opened, if that still exists.

#### Scenario: Press outside
- **WHEN** a menu is open and the user presses a button outside it
- **THEN** the menu closes and the button is not activated

#### Scenario: Escape
- **WHEN** a menu is open and the user presses Escape
- **THEN** the menu closes and focus is where it was before

### Requirement: Small icon buttons
Buttons SHALL also come in the sizes icon-small (2rem square) and icon-extra-small (1.5rem square), with the icon scaled to fit, as in shadcn/ui.

#### Scenario: Extra small
- **WHEN** an icon-extra-small button is drawn at the default root text size
- **THEN** it is 24 px square

### Requirement: Dropdown menus
A dropdown menu SHALL open on a primary pointer press on its trigger, or on enter, space or down while the trigger has focus, and SHALL close when the trigger is pressed again. It SHALL be anchored below the trigger, aligned to its start, and flipped and clamped to stay inside the window. Its items, keys, selection, pointer gesture and dismissal SHALL be those of a context menu. A menu taller than the window SHALL scroll.

#### Scenario: Toggle from the trigger
- **WHEN** the user presses a dropdown's trigger twice
- **THEN** the menu opens on the first press and closes on the second

#### Scenario: Press, drag, release
- **WHEN** the user presses the trigger, moves onto the third item and releases
- **THEN** the third item is selected and the menu closes

#### Scenario: Long menu
- **WHEN** a menu has more items than fit in the window
- **THEN** it is as tall as the window allows and scrolls, and the highlight is kept in view

### Requirement: Tab lists
A tab list SHALL hold tab triggers, exactly one of them active, and SHALL ask its owner to activate a trigger on a primary press on it. The list SHALL be one keyboard stop: focus SHALL rest on the active trigger. While it has focus, left and right SHALL move focus to the previous and next enabled trigger and activate it, wrapping at the ends; home and end SHALL move to the first and last. A disabled trigger SHALL be skipped and SHALL NOT activate.

#### Scenario: Arrow activates
- **WHEN** a tab list has focus on its second of four triggers and the user presses right
- **THEN** the third trigger is active and has focus

#### Scenario: Wrap
- **WHEN** focus is on the last trigger and the user presses right
- **THEN** the first trigger is active and has focus

#### Scenario: Disabled trigger
- **WHEN** the next trigger is disabled and the user presses right
- **THEN** the trigger after it is active

### Requirement: Tab trigger additions
A tab trigger SHALL be able to carry a close button, adornments (status dots) and a tooltip, to close on a middle-click, to shrink between a minimum and maximum width and truncate its label, and to be dragged onto another trigger to ask its owner to move it there. A press on a trigger's close button SHALL close and SHALL NOT activate the trigger. These additions to shadcn's behaviour SHALL be listed with their reasons in the crate's documentation.

#### Scenario: Close button
- **WHEN** the user presses a trigger's close button
- **THEN** the owner is asked to close that trigger and the trigger is not activated

#### Scenario: Drag reorder
- **WHEN** the user drags the first trigger onto the third
- **THEN** the owner is asked to move it to the third place and a bar marks the drop target while dragging

#### Scenario: Middle click
- **WHEN** the user middle-clicks a trigger
- **THEN** the owner is asked to close it

### Requirement: Tab list look
A tab list SHALL be drawn as shadcn's default (pill) tab list. It SHALL NOT offer other variants: neither shadcn's `line` nor one of its own.

#### Scenario: Pill list
- **WHEN** a tab list is drawn
- **THEN** its triggers sit inside one rounded rectangle on the muted background, and the active trigger is raised with a shadow

### Requirement: Tab trigger colours
An owner MAY give any trigger its own surface and text colours (its icons take the text colour). A trigger with its own colours SHALL be drawn on its surface in its text colour whether it is active or not, except that while inactive and not hovered the text SHALL be drawn at 60% opacity. A colour the owner did not give SHALL come from the tokens as usual.

#### Scenario: Owner colours
- **WHEN** the owner gives a trigger a surface colour of `#1a1b26` and a text colour of `#c0caf5`
- **THEN** that trigger, while active, is drawn on `#1a1b26` with `#c0caf5` text, and other triggers are unaffected

#### Scenario: Inactive trigger with owner colours
- **WHEN** an inactive, unhovered trigger has the owner colours above
- **THEN** it is drawn on `#1a1b26` with `#c0caf5` text at 60% opacity, and full `#c0caf5` while hovered

### Requirement: Menu item descriptions
A menu item MAY carry a description, drawn in the muted foreground on a second line under its label. Typeahead and the item's accessible name SHALL use the label only.

#### Scenario: Description shown
- **WHEN** a menu item has the label `tmux` and the description `tmux control mode · main`
- **THEN** the item shows both lines, and typing `t` highlights it
