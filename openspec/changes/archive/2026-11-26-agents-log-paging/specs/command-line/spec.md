## MODIFIED Requirements

### Requirement: Debug hooks in an agent zone
When the active zone is an agent zone, `--dump-screen-after` SHALL print, in place of the terminal's lines, a `--- agent` line (zone, state, name, turn, activity, status), a `--- entry` line per entry the zone holds, in order (id, kind, fields, text on one line), a `--- working` line while the working row shows (its text), a `--- queued` line while the slot holds a prompt (waiting, or held and why), a `--- conversation` line (the count of entries the zone holds, at the end or away, the earlier entries not yet loaded as a count, `loading` or `none`, and a failed fetch's reason) and a `--- prompt` line (status, refusal). A permission's `--- entry` line SHALL list its options in the agent's order. `--wait-for` SHALL match those lines.

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

#### Scenario: Earlier entries in the dump
- **WHEN** an agent zone shows the newest page of a conversation with 300 entries before it, and the dump is taken
- **THEN** the dump has `--- entry` lines only for the entries the zone holds, and its `--- conversation` line has `earlier=300`

#### Scenario: A short conversation in the dump
- **WHEN** the dump is taken of an agent zone whose whole log fits in one page, at its end
- **THEN** its `--- conversation` line reads `entries=N at=end earlier=none`, so waits and checks on `conversation entries=N` and `entries=N at=end` still match

#### Scenario: Waiting for earlier entries
- **WHEN** Brindle scrolls an agent zone to its top with `--key shift-pageup` steps, then runs `--wait-for 'earlier=none'`
- **THEN** the later steps run once the zone holds the session's whole log
