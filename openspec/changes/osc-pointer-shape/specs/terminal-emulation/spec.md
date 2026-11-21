# Spec Delta

## ADDED Requirements

### Requirement: Pointer shape sequences
A terminal SHALL recognise `ESC ] 22 ; <body> ST`, ended by BEL or ST, however the output is split into reads. The body is an optional first character and a comma-separated list of names. With `=` or no first character, each name SHALL set the top of the stack in turn (pushing onto an empty stack); an empty name, as in an empty body, sets Brindle's own pointer. With `>`, each name SHALL be pushed in turn. With `<`, the top SHALL be popped (if any) and the names ignored.

#### Scenario: Push and pop
- **WHEN** a program prints `ESC ] 22 ; >pointer ESC \`, then `ESC ] 22 ; >wait,progress,default ESC \`, then `ESC ] 22 ; < ESC \`
- **THEN** the terminal's pointer shape is `pointer`

#### Scenario: Several names pushed at once
- **WHEN** a program prints `ESC ] 22 ; >pointer,crosshair ESC \`, then `ESC ] 22 ; < ESC \`
- **THEN** the terminal's pointer shape is `pointer`

#### Scenario: Several names set at once
- **WHEN** a program prints `ESC ] 22 ; pointer,crosshair ESC \`
- **THEN** the terminal's pointer shape is `crosshair`, and one pop leaves Brindle's own

#### Scenario: Reset
- **WHEN** a program pushed `crosshair` and prints `ESC ] 22 ; ESC \`
- **THEN** the pointer over the terminal is Brindle's own again

#### Scenario: Reset keeps the entries below
- **WHEN** a program pushed `pointer` and `crosshair`, prints `ESC ] 22 ; ESC \`, and then `ESC ] 22 ; < ESC \`
- **THEN** the terminal's pointer shape is `pointer`

### Requirement: Pointer shape stacks
Each terminal SHALL keep a separate stack for its primary and alternate screen, of up to 16 entries; a push onto a full stack SHALL drop the bottom entry. A sequence SHALL act on the screen active when it arrived. A full reset and the process exiting SHALL empty both. A terminal attached or reconnected to its session SHALL start with both empty.

#### Scenario: Alternate screen keeps its own
- **WHEN** a shell pushes `pointer`, a full-screen program then switches to the alternate screen and pushes `crosshair`, and then exits it
- **THEN** the pointer shape over the terminal is `pointer`

#### Scenario: Seventeen pushes
- **WHEN** a program pushes 17 shapes and then pops 16 times
- **THEN** the stack is empty and the pointer is Brindle's own

### Requirement: Supported pointer shapes
Brindle SHALL show `default`, `text`, `vertical-text`, `pointer`, `crosshair`, `grab`, `grabbing`, `not-allowed`, `no-drop`, `alias`, `copy`, `context-menu`, `ew-resize`, `ns-resize`, `e-resize`, `w-resize`, `n-resize`, `s-resize`, `nwse-resize`, `nesw-resize`, `col-resize` and `row-resize`, each as the platform pointer of that meaning. Other names are skipped, so a list falls back to the names Brindle shows, and a sequence naming none of them SHALL leave the stack as it was.

#### Scenario: Fallback list
- **WHEN** a program prints `ESC ] 22 ; zoom-in,pointer ESC \`
- **THEN** the pointer over the terminal is the pointing hand

#### Scenario: Nothing supported
- **WHEN** a program has set `crosshair` and prints `ESC ] 22 ; zoom-in ESC \`
- **THEN** the pointer over the terminal stays the crosshair

### Requirement: Pointer shape precedence
Over a terminal's grid, the pointer SHALL be the hand while a link is ctrl-hovered, the text beam while a selection is being dragged, and otherwise the top of the active screen's stack. With an empty stack, or Brindle's own on top, it SHALL be Brindle's own: the arrow while the program reports the mouse, the text beam otherwise. The program's shape SHALL NOT apply outside the grid.

#### Scenario: Link over a program shape
- **WHEN** a program has set `crosshair` and the user holds ctrl over a link in that terminal
- **THEN** the pointer is the pointing hand, and the crosshair returns when ctrl is released

### Requirement: Pointer shape queries
A body starting with `?` SHALL be a query, answered on that terminal's PTY as `ESC ] 22 ; <reply> ESC \`, with one comma-separated answer per name: for a shape name `1` or `0`, whether Brindle shows it; for `__current__` the top of the active screen's stack, or `0` when the stack is empty or Brindle's own is on top; for `__default__` `text`, and for `__grabbed__` `default`.

#### Scenario: Support query
- **WHEN** a program prints `ESC ] 22 ; ?pointer,crosshair,no-such-name,wait ESC \`
- **THEN** it reads `ESC ] 22 ; 1,1,0,0 ESC \`

#### Scenario: Current
- **WHEN** a program pushed `grab` and prints `ESC ] 22 ; ?__current__ ESC \`
- **THEN** it reads `ESC ] 22 ; grab ESC \`

#### Scenario: Mixed query on an empty stack
- **WHEN** a program that set no shape prints `ESC ] 22 ; ?__current__,pointer,__default__ ESC \`
- **THEN** it reads `ESC ] 22 ; 0,1,text ESC \`
