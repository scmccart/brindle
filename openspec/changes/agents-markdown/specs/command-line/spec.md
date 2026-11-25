## ADDED Requirements

### Requirement: Rendered blocks in the dump
In an agent zone's dump, each agent message's and thought's `--- entry` line SHALL keep its Markdown source and SHALL be followed by one `--- block entry=N part=K kind=KIND "TEXT"` line per top-level block, in order: KIND is `paragraph`, `heading:L`, `code` or `code:LANG`, `list`, `quote`, `table` or `rule`, and TEXT the block's shown text escaped onto one line. `--wait-for` SHALL match them.

#### Scenario: A fence in the dump
- **WHEN** entry 2, an agent message, is `Try:` then a blank line and a fence opened with three backticks and `rust` holding `fn main() {}`, and the dump is taken
- **THEN** the dump has `--- block entry=2 part=0 kind=paragraph "Try:"` and `--- block entry=2 part=1 kind=code:rust "fn main() {}\n"` right after entry 2's line

#### Scenario: User messages have no blocks
- **WHEN** entry 1 is the user's message `**hi**` and the dump is taken
- **THEN** the dump has `--- entry 1 user "**hi**"` and no `--- block entry=1` line

### Requirement: Links in the dump
Each link in a block SHALL add, after that block's line and in order, a `--- link entry=N part=K target="URI" text="TEXT" action=ACTION` line, ACTION being what a ctrl-click would do on this platform: `open`, `confirm`, or `refuse:no-scheme`, `refuse:scheme`, `refuse:other-host` or `refuse:no-drive`. `--wait-for` SHALL match them.

#### Scenario: Three links
- **WHEN** entry 2's text is `[docs](https://example.com) <mailto:a@example.com> [x](javascript:void)` and the dump is taken
- **THEN** the dump has, after its paragraph's block line, link lines for `https://example.com` with `action=open`, `mailto:a@example.com` with `action=confirm`, and `javascript:void` with `action=refuse:scheme`

### Requirement: Driving the link confirmation
While the link confirmation is open, `--dump-screen-after` SHALL print a `--- confirm link "URI"` line, and the `--- focus` line SHALL report `confirm stop=cancel` or `confirm stop=open`. `--key` SHALL drive it as real keys do.

#### Scenario: Confirmation in the dump
- **WHEN** the user has ctrl-clicked a `mailto:` link in an agent message and the dump is taken
- **THEN** the dump has `--- confirm link "mailto:dev@example.com"` and `--- focus confirm stop=cancel modality=pointer`

### Requirement: Pointer steps on window elements
Besides a terminal cell's `COL,ROW`, the `POS` of `--hover`, `--click`, `--right-click`, `--press`, `--release` and a new repeatable `--double-click` SHALL accept `[MODS-]@NAME[+FX,FY]`: the element painted under NAME in the window's last drawn frame, at fractions FX,FY of its bounds (0 to 1, the centre by default). Such a step SHALL go through the window's own input dispatch, as real pointer input does, and SHALL NOT use synthetic OS-level input.

#### Scenario: Copying a code block with a press
- **WHEN** entry 2's part 1 is a code block holding `fn main() {}` and Brindle runs with `--click @copy-code:2.1.0 --action text_input_paste --dump-screen-after 2` in that agent zone
- **THEN** the dump's `--- input` line holds `fn main() {}`

#### Scenario: Ctrl-pressing a link
- **WHEN** entry 2's part 0 holds the link `<mailto:dev@example.com>` and Brindle runs with `--click ctrl-@link:2.0.0 --dump-screen-after 2`, with no `--modality` step
- **THEN** the dump has `--- confirm link "mailto:dev@example.com"` and `--- focus confirm stop=cancel modality=pointer`

#### Scenario: A plain press on a link
- **WHEN** `BRINDLE_LINK_OPENER` names a recording script, and Brindle runs with `--click @link:2.0.0` over an `https://example.com` link
- **THEN** the script records nothing and no confirmation opens

#### Scenario: Backdrop press
- **WHEN** the link confirmation is open and Brindle runs with `--click @backdrop+0.02,0.02 --dump-screen-after 2`
- **THEN** the dump has no `--- confirm link` line and nothing was opened

#### Scenario: A button shown on hover
- **WHEN** an agent zone shows agent message entry 2 and Brindle runs with `--hover @entry:2 --click @copy-message:2 --action text_input_paste --dump-screen-after 2`
- **THEN** the dump's `--- input` line holds entry 2's source

### Requirement: Element names for pointer steps
NAME SHALL be one of a closed set of kinds, numbered as the dump numbers them: the window's `tab:N`, `profile-menu`, `menu-item:N`, `zone:ID`, `zone-header:ID`, `zone-split-right:ID`, `zone-split-down:ID`, `confirm:cancel`, `confirm:close`, `confirm:open`, `backdrop`, `dialog-close`, `settings-section:NAME`, `settings-row:NAME`, `settings-edit` and `settings-duplicate`, and an agent zone's names.

#### Scenario: A zone's body
- **WHEN** the active tab has agent zone 1 and shell zone 2, zone 2 active, and Brindle runs with `--click @zone:1 --dump-screen-after 2`
- **THEN** the dump has `--- zones count=2 active=1` and `--- focus agent stop=prompt modality=pointer`

### Requirement: Agent zone names resolve in the active zone
An agent zone's names SHALL be `entry:N`, `copy-message:N`, `copy-code:N.P.K`, `link:N.P.K`, `thought:N`, `permission:N:OPTION`, `stop`, `prompt`, `queued:remove`, `queued:send-again`, `queued:discard` and `scroll-to-bottom`, and MAY be qualified `ZONE/NAME`, naming the zone as the dump does. An unqualified name SHALL resolve to the element under it in the active zone, else to the only one in the window.

#### Scenario: A qualified stop
- **WHEN** agent zones 1 and 2 are both in a turn, zone 1 active, and Brindle runs with `--click @2/stop --dump-screen-after 2`
- **THEN** zone 2's turn is cancelled and zone 1's still runs

### Requirement: How an element step presses
An element step SHALL hold its modifiers for the step, let a frame draw between its move, press and release, give a double-click's second press a click count of 2, and hold the left button from `--press` to `--release`, moves between them dragging. It SHALL set no focus-visible modality itself: the press sets one where a real press would. `--double-click` with a cell SHALL give the terminal's handlers presses with click counts 1 and 2.

#### Scenario: Double-clicking an agent profile's row
- **WHEN** the settings dialog shows the Profiles list with agent profile `fake`, and Brindle runs with `--double-click @settings-row:fake --dump-screen-after 2`
- **THEN** the dump's settings line still has `mode=list`, and no editor opened

### Requirement: Element steps that find nothing
An element step whose NAME isn't painted in the last frame, names more than one element, or whose window draws no frame within the step interval SHALL press nothing, and the steps after it SHALL still run.

#### Scenario: A missed step
- **WHEN** no menu is open and Brindle runs with `--click @menu-item:0 --dump-screen-after 2`
- **THEN** the dump has `--- pointer-step missed "@menu-item:0" reason=absent`

### Requirement: Pointer targets in the dump
In a run with an element step, `--dump-screen-after` SHALL print one `--- hit NAME rect=X,Y,W,H` line per named element of the last frame (logical window pixels, sorted by name, ` hidden` when clipped out of view) before the `--- focus` line, and for each element step that pressed nothing a `--- pointer-step missed "POS" reason=absent|ambiguous|no-frame` line beside the `--- wait-for` lines. A run without one SHALL print neither.

#### Scenario: Hit lines
- **WHEN** Brindle runs with two tabs and `--hover @tab:0 --dump-screen-after 2`
- **THEN** the dump has a `--- hit tab:0 rect=` line and a `--- hit tab:1 rect=` line before its focus line

#### Scenario: No hit lines without an element step
- **WHEN** Brindle runs with `--click 0,0 --dump-screen-after 2`
- **THEN** the dump has no `--- hit` and no `--- pointer-step` line

## MODIFIED Requirements

### Requirement: Debug steps are checked before the run
An `--action` naming no action, a `--key` that isn't a keystroke Brindle can parse, and a `--hover`, `--click`, `--right-click`, `--double-click`, `--press` or `--release` position that is neither `COL,ROW` nor `@NAME[+FX,FY]` with a known name kind, well-formed arguments, fractions from 0 to 1 and known modifiers SHALL be refused as usage errors naming the step, before any window opens, as `--step-interval` and `--modality` values are. So SHALL a `--release` with no `--press` held before it, a `--press` while another is held, and a press and release of which one names a cell and the other an element.

#### Scenario: Misspelled action
- **WHEN** Brindle runs with `--action split_zone_rihgt --dump-screen-after 3`
- **THEN** it prints usage naming `split_zone_rihgt` and exits with status 2, without opening a window

#### Scenario: Malformed right-click position
- **WHEN** Brindle runs with `--right-click 3 --dump-screen-after 3`
- **THEN** it prints usage naming `--right-click` and `3` and exits with status 2, without opening a window

#### Scenario: Unknown element kind
- **WHEN** Brindle runs with `--click @copy-mesage:1 --dump-screen-after 3`
- **THEN** it prints usage naming `--click` and `@copy-mesage:1` and exits with status 2, without opening a window

#### Scenario: Fraction out of range
- **WHEN** Brindle runs with `--click @tab:0+2,0 --dump-screen-after 3`
- **THEN** it prints usage naming `--click` and exits with status 2, without opening a window

#### Scenario: Release without a press
- **WHEN** Brindle runs with `--release @tab:0 --dump-screen-after 3`
- **THEN** it prints usage naming `--release` and exits with status 2, without opening a window
