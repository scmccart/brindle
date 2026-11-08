# Spec Delta

## Purpose

Give Brindle's interface one shared set of themed components and overlay behaviours, modelled on shadcn/ui, so every dialog, popover, list and control looks and responds to keys and the pointer the same way.

## ADDED Requirements

### Requirement: Components draw from the active tokens
Every component SHALL take its colours, corner radius and text sizes only from the active token set: background, foreground, card, popover, primary, secondary, muted, accent, destructive (each with its foreground), border, input, ring and radius. Replacing the active token set SHALL redraw every visible component with the new values, with no other step.

#### Scenario: Switching to dark tokens
- **WHEN** the gallery shows a default button with the light token set active, and the dark token set is made active
- **THEN** the button is redrawn in the dark set's primary and primary-foreground colours

### Requirement: Built-in neutral token sets
The component set SHALL provide a light and a dark token set matching shadcn/ui's neutral theme. A dark set's border and input colours SHALL be the foreground at low opacity, as in shadcn/ui.

#### Scenario: Neutral light background
- **WHEN** the built-in light token set is active
- **THEN** the background is white and the primary colour is shadcn/ui's neutral primary

### Requirement: Sizes follow the window's text scale
Component padding, heights, radii and text sizes SHALL scale with the window's root text size, so a larger interface text size enlarges components proportionally.

#### Scenario: Larger text size
- **WHEN** the window's root text size doubles
- **THEN** a default button's height and text size double

### Requirement: Escape dismisses the topmost overlay
When overlays are stacked (a popover opened from a dialog, say), Escape SHALL request dismissal of the topmost overlay only. The overlays below SHALL stay open.

#### Scenario: Popover over a dialog
- **WHEN** a dialog is open with a select's menu open inside it, and the user presses Escape
- **THEN** the menu closes and the dialog stays open

### Requirement: Outside presses dismiss the topmost overlay
A pointer press outside the topmost overlay SHALL request its dismissal. A press inside a higher overlay SHALL NOT count as outside any overlay below it. Overlays other than the topmost SHALL ignore outside presses.

#### Scenario: Pressing inside a nested menu
- **WHEN** a dialog has a select's menu open, and the user presses an item in the menu, which lies outside the dialog's bounds
- **THEN** the item is chosen and the dialog stays open

#### Scenario: Pressing outside both
- **WHEN** a dialog has a popover open, and the user presses the dialog's backdrop
- **THEN** only the popover closes

### Requirement: Modal overlays block what is beneath
While a modal overlay (a dialog) is open, pointer hover and presses SHALL NOT reach content beneath it, and keyboard focus SHALL stay inside it. A non-modal overlay (a popover or a select's menu) SHALL block pointer events only within its own bounds.

#### Scenario: Clicking behind a dialog
- **WHEN** a dialog is open over a button, and the user presses where the button shows through the backdrop
- **THEN** the button is not activated, and the dialog is asked to dismiss

### Requirement: Focus stays inside a modal overlay
In a modal overlay, Tab SHALL move focus to the next focusable control inside it and shift-Tab to the previous, in the overlay's order. Focus SHALL wrap from the last control to the first and back, and SHALL NOT reach controls outside the overlay.

#### Scenario: Wrapping forward
- **WHEN** a dialog's last control has focus and the user presses Tab
- **THEN** the dialog's first control has focus

### Requirement: Focus moves into an overlay when it opens
Opening an overlay SHALL focus the control it designates as initial, else its first focusable control, else the overlay itself, so its keys work at once.

#### Scenario: Dialog with an input
- **WHEN** a dialog whose first control is a text input opens
- **THEN** typing goes into that input

### Requirement: Focus returns when an overlay closes
Closing an overlay SHALL return focus to whatever had focus when it opened, if that still exists. If it no longer exists, closing SHALL leave focus for the overlay's owner to set.

#### Scenario: Back to the trigger
- **WHEN** a select's trigger has focus, the user opens its menu, and presses Escape
- **THEN** the trigger has focus again

### Requirement: Focus rings for keyboard focus
A focused control SHALL show a ring in the ring colour when it got focus from the keyboard (Tab, shift-Tab, an arrow key, or an overlay opened by a key). A control focused by a pointer press SHALL NOT show the ring, except text inputs, which show it whenever focused.

#### Scenario: Clicked button
- **WHEN** the user clicks a button
- **THEN** the button has focus and shows no ring

#### Scenario: Tabbed button
- **WHEN** the user presses Tab and focus lands on a button
- **THEN** the button shows the ring

### Requirement: Anchored placement
An anchored overlay SHALL be placed against its trigger on the requested side (top, bottom, left or right), aligned to the trigger's start, centre or end, at a fixed offset. When the overlay does not fit on that side within the window, it SHALL flip to the opposite side. It SHALL stay within the window, at least a small margin from its edges.

#### Scenario: Flip near the bottom edge
- **WHEN** a select near the bottom of the window opens a menu taller than the space below it
- **THEN** the menu opens above the trigger

### Requirement: List navigation
In a list of options, up and down SHALL move the highlight to the previous and next enabled option, skipping disabled ones, and home and end to the first and last. Moving the pointer over an enabled option SHALL highlight it. The highlighted option SHALL be scrolled into view. A list SHALL either wrap at its ends or stop, as the component defines.

#### Scenario: Skipping a disabled option
- **WHEN** a list highlights option 1, option 2 is disabled, and the user presses down
- **THEN** option 3 is highlighted

### Requirement: Typeahead
In a select, typing printable characters SHALL move the highlight (or, when the menu is closed, the selection) to the first enabled option whose label starts with the typed text, case-insensitively. Characters typed within one second of each other SHALL accumulate. Repeating a single character SHALL cycle through the options starting with it.

#### Scenario: Cycling by letter
- **WHEN** a select's open menu lists `bash`, `fish`, `foot`, the highlight is on `bash`, and the user types `f` and, two seconds later, `f` again
- **THEN** the highlight is on `foot`

### Requirement: Buttons
A button SHALL activate on a primary pointer click, and on enter or space while it has focus. A disabled button SHALL be drawn muted, SHALL NOT activate, and SHALL NOT take focus. Buttons SHALL come in the variants default, destructive, outline, secondary, ghost and link, and the sizes default, small, large and icon, styled as in shadcn/ui.

#### Scenario: Space activates
- **WHEN** a button has focus and the user presses space
- **THEN** the button's action runs once

#### Scenario: Disabled button
- **WHEN** the user clicks a disabled button
- **THEN** nothing runs and focus does not move

### Requirement: Inputs
An input SHALL frame a text input, as defined by the `text-input` capability, with the input colour as its border. It SHALL show the ring when focused, and the destructive colour as its border when marked invalid. Pressing anywhere in the frame SHALL focus the text input.

#### Scenario: Invalid input
- **WHEN** an input is marked invalid
- **THEN** its border is drawn in the destructive colour

### Requirement: Checkboxes and switches
A checkbox and a switch SHALL each show an on or off state set by their owner, and SHALL ask the owner to flip it on a primary click or on space while focused. A disabled one SHALL be drawn muted and SHALL NOT ask.

#### Scenario: Toggling a switch with space
- **WHEN** an off switch has focus and the user presses space
- **THEN** the owner is asked to turn it on, and the switch shows on once the owner does

### Requirement: Dialogs
A dialog SHALL be a modal overlay centred in the window over a dimmed backdrop, with an optional title, description, body and footer. Escape and a press on the backdrop SHALL ask the dialog to dismiss. The dialog SHALL close only when it agrees, so it can keep itself open, for example to show an error.

#### Scenario: Dismiss from the backdrop
- **WHEN** a dialog that accepts dismissal is open and the user presses its backdrop
- **THEN** the dialog closes and focus returns to what had it before

### Requirement: Popovers
A popover SHALL open when its trigger is activated and close when the trigger is activated again. It SHALL be a non-modal, anchored overlay. Escape and outside presses SHALL close it, and its content SHALL be able to close it.

#### Scenario: Toggling from the trigger
- **WHEN** a popover is open and the user clicks its trigger
- **THEN** the popover closes

### Requirement: Selects
A select SHALL show its selected option's label, or a placeholder when none is selected. Activating it, or pressing up or down while it has focus, SHALL open a menu of its options below the trigger, highlighting the selected option. The menu SHALL NOT wrap.

Enter or a click on an option SHALL select it and close the menu. Escape SHALL close the menu without changing the selection. The owner SHALL be told only when the selected option actually changes.

#### Scenario: Choosing an option
- **WHEN** a select shows `bash`, the user opens it, presses down to `fish` and presses enter
- **THEN** the select shows `fish`, its owner is told of the change, and the trigger has focus

#### Scenario: Re-choosing the same option
- **WHEN** a select shows `bash` and the user opens it and clicks `bash`
- **THEN** the menu closes and the owner is not told of a change

### Requirement: Command filtering
A command list SHALL show a filter input above its items, which may be grouped under headings. Typed text SHALL filter items by a case-insensitive subsequence match on their labels, keeping their order. Groups left with no items SHALL be hidden, and a message SHALL show when nothing matches. Each change to the filter SHALL highlight the first match.

#### Scenario: Filtering
- **WHEN** a command list holds `Split Right` and `Close Tab`, and the user types `spr`
- **THEN** only `Split Right` is listed and it is highlighted

#### Scenario: Nothing matches
- **WHEN** the user types text that matches no item
- **THEN** no group heading is shown and the empty message is

### Requirement: Command navigation
While the filter input has focus, up and down (also ctrl-p and ctrl-n) SHALL move the highlight, wrapping at the ends. Enter or a click on an item SHALL confirm it to the owner. Escape SHALL tell the owner the list was cancelled.

#### Scenario: Wrapping
- **WHEN** the last item is highlighted and the user presses down
- **THEN** the first item is highlighted

### Requirement: Command separators
A command list's owner SHALL be able to place a separator between two adjacent groups. A separator SHALL be drawn as a 1px line in the border colour across the full width of the list, past the groups' padding. A separator SHALL show only while both groups beside it list items under the current filter, so it SHALL never show above the first row, below the last row, or next to another separator. A separator SHALL NOT be highlighted or confirmed, and up and down SHALL pass over it.

#### Scenario: Separator between groups
- **WHEN** a command list has groups `Tabs` and `Panes` with a separator between them, and the filter is empty
- **THEN** a line in the border colour spans the list's width between the last row of `Tabs` and the heading of `Panes`

#### Scenario: A group beside the separator is filtered out
- **WHEN** a command list has groups `Tabs` and `Panes` with a separator between them, and the user types text that matches items in `Panes` only
- **THEN** no separator shows, and the `Panes` heading is the first row

#### Scenario: Moving past a separator
- **WHEN** the last item of `Tabs` is highlighted, a separator follows `Tabs`, and the user presses down
- **THEN** the first enabled item of the next group is highlighted
