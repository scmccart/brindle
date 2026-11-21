# Spec Delta

## ADDED Requirements

### Requirement: Pointer shape sequences
A terminal SHALL recognise `ESC ] 22 ; <body> ST`, ended by BEL or ST, however the output is split into reads. A body of comma-separated names, optionally after `=`, SHALL replace the top of the stack (or push when it's empty); `>` before the names SHALL push; `<` SHALL pop (an empty stack stays empty); an empty body SHALL empty the stack. Names are `[a-z0-9_-]`; a sequence with other characters SHALL be ignored.

#### Scenario: Push and pop
- **WHEN** a program prints `ESC ] 22 ; >pointer ESC \`, then `ESC ] 22 ; >wait,progress,default ESC \`, then `ESC ] 22 ; < ESC \`
- **THEN** the terminal's pointer shape is `pointer`

#### Scenario: Reset
- **WHEN** a program pushed `crosshair` and prints `ESC ] 22 ; ESC \`
- **THEN** the pointer over the terminal is Brindle's own again

### Requirement: Pointer shape stacks
Each terminal SHALL keep a separate stack for its primary and alternate screen, of up to 16 entries; a push onto a full stack SHALL drop the bottom entry. A sequence SHALL act on the screen active when it arrived. A full reset and the process exiting SHALL empty both. A terminal attached or reconnected to its session SHALL start with both empty.

#### Scenario: Alternate screen keeps its own
- **WHEN** a shell pushes `pointer`, a full-screen program then switches to the alternate screen and pushes `crosshair`, and then exits it
- **THEN** the pointer shape over the terminal is `pointer`

#### Scenario: Seventeen pushes
- **WHEN** a program pushes 17 shapes and then pops 16 times
- **THEN** the stack is empty and the pointer is Brindle's own

### Requirement: Supported pointer shapes
Brindle SHALL show `default`, `text`, `vertical-text`, `pointer`, `crosshair`, `grab`, `grabbing`, `not-allowed`, `no-drop`, `alias`, `copy`, `context-menu`, `ew-resize`, `ns-resize`, `e-resize`, `w-resize`, `n-resize`, `s-resize`, `nwse-resize`, `nesw-resize`, `col-resize` and `row-resize`, each as the platform pointer of that meaning. A name list SHALL use its first supported name, and an entry with none SHALL show `default`.

#### Scenario: Fallback list
- **WHEN** a program prints `ESC ] 22 ; zoom-in,pointer ESC \`
- **THEN** the pointer over the terminal is the pointing hand

#### Scenario: Nothing supported
- **WHEN** a program prints `ESC ] 22 ; zoom-in ESC \`
- **THEN** the pointer over the terminal is the arrow

### Requirement: Pointer shape precedence
Over a terminal's grid, the pointer SHALL be the hand while a link is ctrl-hovered, the text beam while a selection is being dragged, and otherwise the top of the active screen's stack. With an empty stack it SHALL be Brindle's own: the arrow while the program reports the mouse, the text beam otherwise. The program's shape SHALL NOT apply outside the grid.

#### Scenario: Link over a program shape
- **WHEN** a program has set `crosshair` and the user holds ctrl over a link in that terminal
- **THEN** the pointer is the pointing hand, and the crosshair returns when ctrl is released

### Requirement: Pointer shape queries
A body starting with `?` SHALL be a query, answered on that terminal's PTY as `ESC ] 22 ; <reply> ESC \`. For a list of names the reply SHALL be comma-separated `1` or `0`, whether each is supported. `__current__` SHALL give the top of the active screen's stack, or `0` when it is empty; `__default__` SHALL give `text`, and `__grabbed__` `default`.

#### Scenario: Support query
- **WHEN** a program prints `ESC ] 22 ; ?pointer,crosshair,no-such-name,wait ESC \`
- **THEN** it reads `ESC ] 22 ; 1,1,0,0 ESC \`

#### Scenario: Current
- **WHEN** a program pushed `grab` and prints `ESC ] 22 ; ?__current__ ESC \`
- **THEN** it reads `ESC ] 22 ; grab ESC \`
