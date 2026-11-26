## MODIFIED Requirements

### Requirement: Debug hooks in an agent zone
When the active zone is an agent zone, `--dump-screen-after` SHALL print, in place of the terminal's lines, a `--- agent` line (zone, state, name, turn, activity, status), a `--- entry` line per entry in order (id, kind, fields, text on one line), a `--- working` line while the working row shows (its text), a `--- queued` line while the slot holds a prompt (waiting, or held and why), a `--- conversation` line (entry count, at the end or away) and a `--- prompt` line (status, refusal). A permission's `--- entry` line SHALL list its options in the agent's order. `--wait-for` SHALL match those lines.

#### Scenario: A turn in the dump
- **WHEN** Brindle runs `brindle agent Fake -p hi --wait-for 'Hello!' --dump-screen-after 2` against a fake ACP agent that answers `Hello!`
- **THEN** the dump has `--- entry 1 user "hi"` and `--- entry 2 agent "Hello!"`, a `--- agent` line with `turn=ended:end_turn activity=done`, and no `--- working` line

#### Scenario: A pending permission in the dump
- **WHEN** the fake ACP agent asks permission with options `allow-once` and `reject-once`, and the dump is taken
- **THEN** the permission's `--- entry` line names both options and says it is pending

#### Scenario: A held prompt in the dump
- **WHEN** `next` was queued, the user stopped the turn, and the dump is taken
- **THEN** the dump has `--- queued held reason=cancelled "next"`

#### Scenario: Working in the dump
- **WHEN** the fake ACP agent has the prompt `hi`, reports tool call `Read main.rs` as in progress and waits, and the dump is taken
- **THEN** the dump has `--- working "Read main.rs"` after the entry lines

### Requirement: Driving an agent zone
`--send` SHALL type into the agent zone's prompt input, as into any focused text input. `--key` SHALL press keys there (enter sends, shift-enter adds a line, escape stops a turn, tab moves among the zone's stops), and `--action` SHALL run the agent actions. The `--- focus` line SHALL report `agent stop=prompt`, `agent stop=queued:remove`, `agent stop=queued:send-again`, `agent stop=queued:discard`, `agent stop=permission:<entry>:<option>` or `agent stop=stop`.

#### Scenario: Focus on an option
- **WHEN** permission request entry 3 with options `allow-once` and `reject-once` is pending, nothing is queued, the prompt input has focus, and Brindle runs `--key tab --dump-screen-after 2`
- **THEN** the dump's focus line reads `--- focus agent stop=permission:3:reject-once modality=keyboard`

#### Scenario: Answer from the keyboard
- **WHEN** the same request is pending and Brindle runs `--key tab --key enter --dump-screen-after 2`
- **THEN** the dump's permission entry says `reject-once` was chosen

#### Scenario: Allow from the keyboard
- **WHEN** the same request is pending and Brindle runs `--key tab --key tab --key enter --dump-screen-after 2`
- **THEN** the dump's permission entry says `allow-once` was chosen

#### Scenario: Send a held prompt from the keyboard
- **WHEN** `next` is held, no turn runs, the prompt input has focus, and Brindle runs `--key tab --key enter --dump-screen-after 2`
- **THEN** the dump has no `--- queued` line and an entry `user "next"`

## ADDED Requirements

### Requirement: Spinners and motion in the dump
`--dump-screen-after` SHALL print `--- spinner tab=N` for each tab whose face shows a spinner, numbered as the tab lines, then `--- spinner zone=ID` for each zone of the active tab whose header shows one when it has two or more zones, and a `--- motion running` or `--- motion stopped` line saying whether the window's motion clock runs.

#### Scenario: A working agent
- **WHEN** the only tab is an agent zone whose turn is working, and the dump is taken
- **THEN** the dump has `--- spinner tab=1` and `--- motion running`

#### Scenario: Nothing working
- **WHEN** the agent's turn has ended, and the dump is taken
- **THEN** the dump has no `--- spinner` line, and has `--- motion stopped`
