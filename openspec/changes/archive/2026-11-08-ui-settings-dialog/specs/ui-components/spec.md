## ADDED Requirements

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

## MODIFIED Requirements

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
