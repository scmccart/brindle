## MODIFIED Requirements

### Requirement: Debug hooks in an agent zone
When the active zone is an agent zone, `--dump-screen-after` SHALL print, in place of the terminal's lines, a `--- agent` line (zone, state, name, turn, activity, status), a `--- entry` line per entry in order (id, kind, fields, text on one line), a `--- queued` line while the slot holds a prompt (waiting, or held and why), a `--- conversation` line (entry count, the earlier entries not yet loaded as a count, `loading` or `none`, a failed fetch's reason, at the end or away) and a `--- prompt` line (status, refusal). `--wait-for` SHALL match those lines.

#### Scenario: A turn in the dump
- **WHEN** Brindle runs `brindle agent Fake -p hi --wait-for 'Hello!' --dump-screen-after 2` against a fake ACP agent that answers `Hello!`
- **THEN** the dump has `--- entry 1 user "hi"` and `--- entry 2 agent "Hello!"`, and a `--- agent` line with `turn=ended:end_turn activity=done`

#### Scenario: A pending permission in the dump
- **WHEN** the fake ACP agent asks permission with options `allow-once` and `reject-once`, and the dump is taken
- **THEN** the permission's `--- entry` line names both options and says it is pending

#### Scenario: A held prompt in the dump
- **WHEN** `next` was queued, the user stopped the turn, and the dump is taken
- **THEN** the dump has `--- queued held reason=cancelled "next"`

#### Scenario: Earlier entries in the dump
- **WHEN** an agent zone shows the newest page of a conversation with 300 entries before it, and the dump is taken
- **THEN** the dump has `--- entry` lines only for the entries the zone holds, and its `--- conversation` line has `earlier=300`

#### Scenario: Waiting for earlier entries
- **WHEN** Brindle scrolls an agent zone to its top with `--key shift-pageup` steps, then runs `--wait-for 'earlier=none'`
- **THEN** the later steps run once the zone holds the session's whole log
