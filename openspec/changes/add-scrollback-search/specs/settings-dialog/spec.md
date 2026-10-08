# Spec Delta

## MODIFIED Requirements

### Requirement: Theme editor colors
The editor SHALL show every theme color with a swatch and an editable hex field: foreground, background, cursor, cursor text, selection background, selection foreground, accent, the four search highlight colors (match background and foreground, focused match background and foreground), and the 16 ANSI colors labelled by name. Fields SHALL accept `#rrggbb` and `#rgb`. Each valid edit SHALL be previewed at once, as in the theme list. An invalid field SHALL be marked, SHALL keep the last valid color in the preview, and SHALL block saving.

#### Scenario: Edit previews immediately
- **WHEN** the user changes the background field to `#000000`
- **THEN** the swatch and the terminals behind the dialog turn black

#### Scenario: Invalid color
- **WHEN** the user types `#12345` in the cursor field
- **THEN** the field is marked invalid, the preview keeps the previous cursor color, and Save is unavailable

#### Scenario: Search colors listed
- **WHEN** the user opens the editor on `tokyo-night`
- **THEN** the search highlight fields are shown, pre-filled with the colors `tokyo-night` derives for them

### Requirement: Optional selection foreground
Selection foreground SHALL be clearable. A cleared selection foreground SHALL mean that selected cells keep their own colors. Cursor text, accent and the four search highlight colors SHALL always hold an explicit color, pre-filled with the effective color of the theme the editor was opened from.

#### Scenario: Clear selection foreground
- **WHEN** the user clears the selection foreground and saves
- **THEN** the saved theme has no `selection_foreground`, and selected text keeps its colors

#### Scenario: Search colors saved explicitly
- **WHEN** the user copies a built-in theme and saves it without touching the search fields
- **THEN** the saved theme has all four `search_*` keys, holding the colors the built-in theme derived
