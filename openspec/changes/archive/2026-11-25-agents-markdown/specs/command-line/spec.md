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
