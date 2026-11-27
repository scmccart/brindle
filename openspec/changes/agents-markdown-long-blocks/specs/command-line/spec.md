## MODIFIED Requirements

### Requirement: Rendered blocks in the dump
In an agent zone's dump, each agent message's and thought's `--- entry` line SHALL keep its Markdown source and SHALL be followed by one `--- block entry=N part=K kind=KIND "TEXT"` line per top-level block, in order: KIND is `paragraph`, `heading:L`, `code` or `code:LANG`, `list`, `quote`, `table` or `rule`, and TEXT the block's shown text escaped onto one line. A top-level table or list SHALL be one line however long it is, and K SHALL count top-level blocks. `--wait-for` SHALL match them.

#### Scenario: A fence in the dump
- **WHEN** entry 2, an agent message, is `Try:` then a blank line and a fence opened with three backticks and `rust` holding `fn main() {}`, and the dump is taken
- **THEN** the dump has `--- block entry=2 part=0 kind=paragraph "Try:"` and `--- block entry=2 part=1 kind=code:rust "fn main() {}\n"` right after entry 2's line

#### Scenario: User messages have no blocks
- **WHEN** entry 1 is the user's message `**hi**` and the dump is taken
- **THEN** the dump has `--- entry 1 user "**hi**"` and no `--- block entry=1` line

#### Scenario: A long table is one block
- **WHEN** entry 2, an agent message, is `Rows:`, a blank line, and a two-column table with header `| n | sq |` and 120 body rows `| 1 | 1 |` to `| 120 | 14400 |`, followed by a blank line and `Done.`, and the dump is taken
- **THEN** the dump has `--- block entry=2 part=1 kind=table` with the header and all 120 rows in order, one per line, and then `--- block entry=2 part=2 kind=paragraph "Done."`
