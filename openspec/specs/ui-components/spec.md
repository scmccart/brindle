# ui-components Specification

## Purpose
Give Brindle's interface one shared set of themed components and overlay behaviours, modelled on shadcn/ui, so every dialog, popover, list and control looks and responds to keys and the pointer the same way.

## Requirements

### Requirement: Components draw from the active tokens
Every component SHALL take its colours, corner radius and text sizes only from the active token set: background, foreground, card, popover, primary, secondary, muted, accent, destructive (each with its foreground), border, input, ring and radius. The active token set SHALL be that of the innermost theme scope the component is drawn in, else the window's. Replacing the window's token set SHALL redraw every visible component outside a theme scope with the new values, with no other step.

#### Scenario: Switching to dark tokens
- **WHEN** the gallery shows a default button with the light token set active, and the dark token set is made active
- **THEN** the button is redrawn in the dark set's primary and primary-foreground colours

#### Scenario: Inside a scope
- **WHEN** the window's token set is the light one and a default button is drawn inside a theme scope of the dark set
- **THEN** the button is drawn in the dark set's primary and primary-foreground colours, and a button beside the scope in the light set's

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
A checkbox and a switch SHALL each show an on or off state set by their owner, and SHALL ask the owner to flip it on a primary click or on space while focused. A switch SHALL also be flipped by enter while focused; a checkbox SHALL NOT be (enter passes to what is around it). A disabled one SHALL be drawn muted and SHALL NOT ask.

#### Scenario: Toggling a switch with space
- **WHEN** an off switch has focus and the user presses space
- **THEN** the owner is asked to turn it on, and the switch shows on once the owner does

#### Scenario: Toggling a switch with enter
- **WHEN** an off switch has focus and the user presses enter
- **THEN** the owner is asked to turn it on

#### Scenario: Enter on a checkbox
- **WHEN** a checkbox has focus and the user presses enter
- **THEN** the owner is not asked, and the key passes to what is around the checkbox

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

### Requirement: Overlays can ignore outside presses
A modal overlay SHALL be openable so that a pointer press outside it does not request its dismissal. Escape and every other way of closing SHALL be unaffected, and the backdrop SHALL still block the pointer from reaching what is beneath. Overlays opened above such an overlay SHALL still dismiss on their own outside presses.

#### Scenario: Press on the backdrop
- **WHEN** a dialog opened to ignore outside presses is showing and the user presses its backdrop
- **THEN** the dialog stays open, focus stays where it was, and the press reaches nothing beneath

#### Scenario: Escape still dismisses
- **WHEN** the same dialog is showing and the user presses Escape
- **THEN** dismissal is requested, as for any dialog

#### Scenario: Menu inside such a dialog
- **WHEN** the dialog has a select's menu open and the user presses the dialog's body outside the menu
- **THEN** only the menu closes

### Requirement: Components follow shadcn and Radix behaviour
Where a component has a shadcn/ui counterpart (itself built on Radix), its keyboard and pointer behaviour SHALL match that counterpart's. Every difference SHALL be listed, with its reason, in the crate's documentation, and a behaviour not listed there is a defect. An app SHALL NOT add keys to a component's behaviour (for example by binding keys around it that change its value); a behaviour the app needs and shadcn lacks is added to the component and listed as a difference, or is not added.

#### Scenario: Closed select trigger keys
- **WHEN** a select has focus with its menu closed
- **THEN** enter, space, up and down open the menu on the selected option, and left, right, home and end do nothing

#### Scenario: A difference is documented
- **WHEN** a component differs from its shadcn counterpart
- **THEN** the crate's documentation lists the difference and its reason

### Requirement: Select pointer behaviour
A select's trigger SHALL open its menu on a primary pointer press, and the menu SHALL pick the option under the pointer on release, so press, drag onto an option and release is one gesture. As in Radix, the release ending the opening press SHALL be ignored after a move of at most 10px and SHALL close the menu after a larger one. A press outside an open menu SHALL only close it.

#### Scenario: One gesture
- **WHEN** the user presses the trigger, drags onto the third option and releases
- **THEN** the third option is selected, the menu is closed and the owner is told of the change

#### Scenario: Release where the press began
- **WHEN** the user presses the trigger and releases it without moving
- **THEN** the menu is open, with the selected option highlighted

#### Scenario: Dragged release outside the menu
- **WHEN** the user presses the trigger, moves the pointer more than 10px while holding it, and releases over the trigger or elsewhere outside the menu
- **THEN** the menu is closed, the selection is unchanged and the owner is not told of a change

#### Scenario: Press outside an open menu
- **WHEN** a menu is open and the user presses a button outside it
- **THEN** the menu closes and the button is not activated

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
- **WHEN** a menu item has the label `Shell` and the description `default shell`
- **THEN** the item shows both lines, and typing `s` highlights it

### Requirement: Cards
A card SHALL be a bordered, rounded surface with an optional header row above its content, after shadcn's Card. The header SHALL hold a title, an optional leading adornment (a status dot) and an optional action slot at its end, and be separated from the content by a border line. The content SHALL fill the rest of the card with no padding of its own. A card SHALL NOT know of terminals.

#### Scenario: Header optional
- **WHEN** a card is drawn without a header
- **THEN** its content fills the whole card inside the border

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

### Requirement: Rounded corners over content
A card SHALL keep its rounded corners when its content paints to the corners, by painting the area outside the rounded edge in a surround colour given by its owner, above the content and below the border.

#### Scenario: Content with a square background
- **WHEN** a card's content fills its box with a square background
- **THEN** the card's corners show the surround colour outside the rounded border and none of the content's background

### Requirement: Textareas
A textarea SHALL frame a text area, as defined by `text-input`, as an input frames a text input: the input colour as its border, the ring when focused, the destructive colour when marked invalid, and a press anywhere in the frame focusing the text area.

#### Scenario: Press in the frame
- **WHEN** the user presses in a textarea's padding below its text
- **THEN** the text area has focus, with the cursor at the end of its text

### Requirement: Message scroller
A message scroller SHALL show a list of items of different heights, scrolled to its end when it first shows them. While it is at its end it SHALL stay there as items are added or grow; once scrolled away it SHALL keep the items in view where they are and show a scroll button, which scrolls to the end and follows again. It SHALL lay out only the items in view and near it.

#### Scenario: Growing last item
- **WHEN** a message scroller is at its end and its last item grows by ten lines
- **THEN** the end of that item is in view

#### Scenario: Scrolled away
- **WHEN** the user scrolls a message scroller up and an item is added at the end
- **THEN** the items in view don't move, and the scroll button shows

### Requirement: Messages
A message SHALL show one turn's text in a bubble, aligned to the end for the person and to the start for the other side, the person's in the muted colours and the other's on the card. The other side's bubble SHALL fill the width of its row. The person's bubble SHALL be as wide as its text, up to four fifths of the row. A message drawn in parts SHALL give every part the same width, so its parts form one bubble with straight edges. A message SHALL keep its text's line breaks and wrap long lines, and SHALL be able to carry actions shown while the pointer is over it.

#### Scenario: Two sides
- **WHEN** a person's message and an agent's message are shown
- **THEN** the person's bubble sits at the end in the muted colours, and the agent's at the start

#### Scenario: The other side fills the row
- **WHEN** an agent's message holding only the word `ok` is shown in a row 600 pixels wide
- **THEN** its bubble is as wide as the row

#### Scenario: The person's bubble fits its text
- **WHEN** a person's message holding only the word `ok` is shown in a row 600 pixels wide
- **THEN** its bubble is narrower than half the row and sits at the row's end

#### Scenario: Parts of different lengths
- **WHEN** an agent's message is drawn in two parts, the first the word `short` and the second a paragraph wider than the row
- **THEN** both parts are as wide as the row, the paragraph wraps inside it, and the two parts' left and right edges line up

### Requirement: Prompt input
A prompt input SHALL hold a textarea and a toolbar with a submit button. The owner SHALL give it a status: ready (an arrow; disabled while the text is blank), submitted (a spinner, disabled), streaming (a stop square) or disabled. A press on the submit button, or enter in the textarea, SHALL ask the owner to submit when ready; in streaming, the button SHALL ask the owner to stop.

#### Scenario: Stop while streaming
- **WHEN** a prompt input's status is streaming and the user clicks its button
- **THEN** the owner is asked to stop, not to submit

#### Scenario: Blank text
- **WHEN** a prompt input is ready and its textarea is blank
- **THEN** its submit button is disabled, and enter asks nothing

### Requirement: Prompt input queue row
Given a queued text, a prompt input SHALL show it in a row above the textarea: as waiting, with a remove button, or as not sent, with the owner's reason and send-again and discard buttons. Each button SHALL be a keyboard stop and SHALL ask the owner; the row SHALL never change the textarea's text.

#### Scenario: Queued row
- **WHEN** the owner gives a prompt input queued text `next`
- **THEN** a row above the textarea shows `next` with a remove button, and pressing the button asks the owner to remove it

#### Scenario: Held row
- **WHEN** the owner gives a prompt input the text `next` as not sent, with the reason `the turn was stopped`
- **THEN** the row shows `next`, `not sent`, the reason, and send-again and discard buttons, each asking the owner, and the textarea's text is unchanged

### Requirement: Reasoning blocks
A reasoning block SHALL show a trigger with a label and a chevron over collapsible content. The owner SHALL hold whether it is open; a press on the trigger SHALL ask the owner to flip it. While closed, only the trigger SHALL show.

#### Scenario: Open on press
- **WHEN** a closed reasoning block's trigger is pressed
- **THEN** the owner is asked to open it, and once it does the content shows below the trigger

### Requirement: Tool rows
A tool row SHALL be one line: an icon the owner chooses, a title cut with an ellipsis, and a status badge reading pending, running, done or failed, the failed one in the destructive colour. It SHALL take no focus.

#### Scenario: Long title
- **WHEN** a tool row's title is wider than the row
- **THEN** the title is cut with an ellipsis, and the status badge stays whole

### Requirement: Confirmations
A confirmation SHALL show a title and, while requested, one small button per action the owner gives, each a keyboard stop activated by enter or space or a primary click. Once the owner marks it accepted, rejected or cancelled, it SHALL show that outcome with the chosen action's label and no buttons.

#### Scenario: Requested
- **WHEN** a confirmation is requested with actions `Allow once` and `Reject`
- **THEN** both buttons show, and pressing space on the focused `Reject` button asks the owner for `Reject`

#### Scenario: Answered
- **WHEN** the owner marks a confirmation accepted with `Allow once`
- **THEN** it shows `Allow once` as the outcome and no buttons

### Requirement: Alerts
An alert SHALL be a bordered block with an icon, a title and a description, in the default colours or, for the destructive variant, with its title and icon in the destructive colour. It SHALL take no focus.

#### Scenario: Destructive alert
- **WHEN** an alert of the destructive variant is shown
- **THEN** its title and icon are drawn in the destructive colour

### Requirement: Responses
A response SHALL draw a list of blocks the owner gives as plain data: paragraphs and headings of styled runs (strong, emphasis, strikethrough, inline code, muted, link), code blocks, bulleted, numbered and task lists (which may nest blocks), quotes, tables and rules. It SHALL interpret no markup of its own, and SHALL take the code font family from its owner.

#### Scenario: Styled runs
- **WHEN** the owner gives a paragraph whose runs are `plain`, `bold` (strong) and `code` (inline code)
- **THEN** `bold` is drawn in a bold weight and `code` in the owner's code font on the muted surface, all on one line

#### Scenario: Nested list
- **WHEN** the owner gives a numbered list starting at 3 whose second item holds a bulleted list
- **THEN** the items are numbered 3 and 4, and the bulleted list is indented under item 4

### Requirement: Response code blocks
A response's code block SHALL draw its text in the owner's code font on the muted surface inside a border, keeping line breaks and spaces, with its label (when it has one) and a copy button above the code. The label SHALL stay on one line: it SHALL NOT wrap, and when it is wider than the room beside the copy button it SHALL be cut with an ellipsis. The copy button SHALL be pointer-only, taking no keyboard focus, and SHALL give the owner the block's text; the response SHALL NOT write the clipboard itself.

#### Scenario: Copy button
- **WHEN** the user presses a code block's copy button
- **THEN** the owner is given that block's text, and keyboard focus doesn't move

#### Scenario: A short label inside a message
- **WHEN** a code block labelled `rust` is drawn inside an agent's message in a row 600 pixels wide
- **THEN** the label `rust` shows on one line, as wide as its text, above the code

#### Scenario: A label too long for its row
- **WHEN** a code block's label is wider than the room left beside its copy button
- **THEN** the label is one line cut with an ellipsis, and the copy button stays whole

### Requirement: Response links
A response SHALL draw each link run in the link colour, underlined. A press released over the same link SHALL be reported to the owner with the link and the modifiers held; the response SHALL show a pointing hand over a link only while the modifiers the owner names are held, and a tooltip with the owner's text for a link that has one.

#### Scenario: Press on a link
- **WHEN** the user presses and releases on a link run with ctrl held
- **THEN** the owner is told which link, with ctrl among the modifiers

#### Scenario: Release elsewhere
- **WHEN** the user presses on a link, moves off it and releases
- **THEN** nothing is reported

### Requirement: Response tables
A response's table SHALL be drawn as a grid of equal-width columns, one per header cell, with the header row set apart by the border colour and each column aligned as the owner says. A cell's text SHALL wrap within its column, and a row with fewer cells than the header SHALL leave the rest empty.

#### Scenario: Short row
- **WHEN** the owner gives a three-column table whose second row has two cells
- **THEN** that row's third cell is empty and the grid keeps three columns

### Requirement: Prompt input reason action
Given an action label, a prompt input SHALL show a button with that label after its reason line's text. The button SHALL be a keyboard stop right after the textarea and SHALL ask the owner when pressed; it SHALL never change the textarea's text. Without a label, no button SHALL show and it SHALL be no stop.

#### Scenario: Action after the reason
- **WHEN** the owner gives a disabled prompt input the reason `the agent has exited` and the action `Continue conversation`, and the user presses Tab in the textarea, then enter
- **THEN** the button has focus after the Tab, the enter asks the owner for the action, and the textarea's text is unchanged

#### Scenario: Action taken away
- **WHEN** the owner takes the action away
- **THEN** no button shows after the reason, and Tab from the textarea doesn't reach one

### Requirement: Confirmation action descriptions
A confirmation's action MAY carry a description, shown as plain text under its button, and MAY be disabled. When any action has a description, the actions SHALL stack in the owner's order, each with its description under it. A disabled action SHALL be drawn muted, SHALL NOT be a keyboard stop and SHALL ask the owner for nothing.

#### Scenario: Action descriptions
- **WHEN** a confirmation is requested with actions `Log in with Google`, `API Key` described `Use an API key`, and a disabled `Log in from the terminal`
- **THEN** the three buttons show one under another in that order, `Use an API key` shows under `API Key`, tab from `Log in with Google` reaches `API Key`, and tab from `API Key` leaves the confirmation

### Requirement: Text wraps at its container's width
A component's text that may wrap SHALL wrap only at the width its container gives it, whatever the text measured before in the same frame and whatever width the container had in the frame before. A one-line label SHALL stay on one line, giving way with an ellipsis rather than wrapping.

#### Scenario: Reason line in a narrow input
- **WHEN** a prompt input 240 pixels wide shows the reason `connecting to the terminal server`
- **THEN** the reason wraps at the input's width, its first line holding at least the word `connecting`, and no line holds a single letter

#### Scenario: After the width changes
- **WHEN** a prompt input showing a reason is drawn 600 pixels wide and then, in the next frame, 240 pixels wide
- **THEN** the reason wraps at 240 pixels as it would have had it been drawn at that width first

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
