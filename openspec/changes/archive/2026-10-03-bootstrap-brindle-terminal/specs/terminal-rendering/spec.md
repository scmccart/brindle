# Spec Delta

## Purpose

Draw a terminal grid with the GPU renderer, faithfully and legibly: colors, text attributes, cursor, selection, wide and box-drawing characters, IME pre-edit, and font selection.

## ADDED Requirements

### Requirement: GPU rendering
All terminal content and window chrome SHALL be drawn with GPUI's GPU renderer (Vulkan on Linux).

#### Scenario: Window opens with GPU rendering
- **WHEN** Brindle starts on a Linux desktop with Vulkan available
- **THEN** a window appears and the terminal grid is drawn through GPUI

### Requirement: Colors and attributes
The renderer SHALL show:
- the 16 theme colors, the 256-color palette and 24-bit color;
- bold, italic, dim, inverse, hidden and strikethrough text;
- single, double and curly (undercurl) underlines in their own color.

Colors a program changes at runtime (OSC 4/10/11/12) SHALL override the theme.

#### Scenario: True color
- **WHEN** a program prints text with `ESC [ 38;2;255;128;0 m`
- **THEN** the text is drawn in that orange

#### Scenario: Undercurl
- **WHEN** a program sets a curly underline in red (as editors do for diagnostics)
- **THEN** a red wavy underline is drawn under the text

### Requirement: Grid alignment
Every glyph SHALL be placed at its cell's column. A glyph with an unusual advance width SHALL NOT shift the glyphs after it. Wide (CJK) characters SHALL take two cells, and color emoji and fallback glyphs SHALL render.

#### Scenario: Mixed-width line
- **WHEN** a line contains ASCII text, `漢字`, an emoji, and more ASCII text
- **THEN** the trailing ASCII text starts exactly at its expected column

### Requirement: Box drawing and block elements
Box-drawing characters (light, heavy, double, rounded and dashed lines) and block elements (U+2580–U+259F) SHALL be drawn as geometry that fills the cell, so adjacent characters join without gaps at any line height.

#### Scenario: tmux borders join
- **WHEN** tmux draws vertical pane borders with `│` on consecutive rows
- **THEN** they appear as one continuous line, with no gaps between rows

### Requirement: Cursor
The renderer SHALL draw the cursor shape the program requests (block, beam or underline, via DECSCUSR), falling back to the configured default. The cursor SHALL be a hollow block when the terminal is not focused. A focused cursor SHALL blink when blinking is enabled, but stay solid while there is recent typing or output.

#### Scenario: Unfocused cursor
- **WHEN** the window loses focus
- **THEN** the cursor is drawn as an outline

#### Scenario: Beam cursor from the program
- **WHEN** a shell sends `ESC [ 6 q`
- **THEN** the cursor is drawn as a thin bar

### Requirement: Selection highlight
Selected cells SHALL be drawn with the theme's selection background (and selection foreground, if the theme sets one).

#### Scenario: Selection shown
- **WHEN** the user drags across text
- **THEN** the dragged cells are highlighted

### Requirement: IME pre-edit
Text that an input method is still composing SHALL be drawn at the cursor with an underline. The input method's candidate window SHALL be placed at the cursor.

#### Scenario: Composing with an IME
- **WHEN** the user composes text with an input method
- **THEN** the in-progress text appears underlined at the cursor until it is committed

### Requirement: Font selection
The terminal font SHALL be the first installed family from `font.family`. If none is installed, a common monospace font SHALL be used. Missing glyphs SHALL fall back to symbol, emoji and CJK fonts. Ligatures SHALL be disabled.

#### Scenario: Preferred font missing
- **WHEN** `font.family = ["Nonexistent", "DejaVu Sans Mono"]`
- **THEN** text is drawn in DejaVu Sans Mono

### Requirement: Font zoom
The user SHALL be able to increase, decrease and reset the font size across all tabs (ctrl-=, ctrl--, ctrl-0). The grid SHALL re-fit to the new cell size.

#### Scenario: Zoom in
- **WHEN** the user presses ctrl-=
- **THEN** text gets larger and the terminal reports fewer columns and rows
