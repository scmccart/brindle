# Spec Delta

## ADDED Requirements

### Requirement: Revealing an item in a message scroller
The owner SHALL be able to ask a message scroller to reveal an item. An item wholly in view SHALL stay where it is. Otherwise the scroller SHALL scroll the least distance that shows it whole, or its top when it is taller than the view, measured from what the view showed last, and SHALL follow its end again when that shows its end.

#### Scenario: Above the view
- **WHEN** a message scroller is at its end and the owner asks it to reveal an item above the view
- **THEN** that item's top is at the view's top, and the scroll button shows

#### Scenario: Already in view
- **WHEN** the owner asks a message scroller to reveal an item that is wholly in view
- **THEN** nothing in view moves

#### Scenario: The last item
- **WHEN** a message scroller is scrolled away and the owner asks it to reveal its last item, which fits in the view
- **THEN** the last item's end is at the view's bottom, and the scroller follows its end again

## MODIFIED Requirements

### Requirement: Message scroller
A message scroller SHALL show a list of items of different heights, scrolled to its end when it first shows them. While it is at its end it SHALL stay there as items are added or grow; once scrolled away it SHALL keep the items in view where they are and show a scroll button, which scrolls to the end and follows again. It SHALL lay out only the items in view and near it.

#### Scenario: Growing last item
- **WHEN** a message scroller is at its end and its last item grows by ten lines
- **THEN** the end of that item is in view

#### Scenario: Scrolled away
- **WHEN** the user scrolls a message scroller up and an item is added at the end
- **THEN** the items in view don't move, and the scroll button shows

#### Scenario: The item at the top grows
- **WHEN** a message scroller is scrolled away with an item's middle at the view's top, and the owner replaces that item with a taller one
- **THEN** the item's lines in view don't move

#### Scenario: The item at the top shrinks above the view
- **WHEN** a message scroller is scrolled away 400 pixels into an item, and the owner replaces that item with one 100 pixels tall
- **THEN** the item after it is at the view's top

#### Scenario: Items inserted at the start
- **WHEN** a message scroller, at its end or scrolled away with its top row out of view, has items inserted before its first item
- **THEN** the items in view don't move, and the owner learns whether the top row is in view only from the next frame drawn

#### Scenario: Every item measured again
- **WHEN** a message scroller is scrolled away with an item's middle at the view's top, and the owner has every item measured again after their text size doubled
- **THEN** that item is still at the view's top, at the same fraction of its height, and the items above and below have their new heights as they scroll into view
