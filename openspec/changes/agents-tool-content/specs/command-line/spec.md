## ADDED Requirements

### Requirement: Tool call rows in the dump
In an agent zone's dump, a tool call's `--- entry` line SHALL say, after its status and before its title, whether its row is open (`open=yes`, `open=no`, `open=permission` while a pending permission request shows its detail, or `open=none` when it has nothing to open) and how many content items it holds (`items=N`).

#### Scenario: A running tool call
- **WHEN** the fake ACP agent reports a tool call with one diff as in progress, and the dump is taken
- **THEN** the tool call's `--- entry` line has `open=yes items=1`

#### Scenario: A finished tool call
- **WHEN** the fake ACP agent then reports that tool call as completed, and the dump is taken
- **THEN** the tool call's `--- entry` line has `open=no items=1`

### Requirement: Permission details in the dump
A permission request's `--- entry` line SHALL say, after its outcome, how many parts of its tool call's detail it shows (`detail=N`), and SHALL print nothing more when it shows none, so its line reads as before tool call content existed.

#### Scenario: A pending permission's detail
- **WHEN** the fake ACP agent asks permission for a tool call that holds one file change, and the dump is taken
- **THEN** the permission's `--- entry` line has `detail=` with a count above zero after `outcome=pending`, and the tool call's line has `open=permission`

#### Scenario: A permission with nothing to show
- **WHEN** the fake ACP agent asks permission `Write a file` for a tool call with no content, and the dump is taken
- **THEN** the permission's `--- entry` line ends with `outcome=pending "Write a file"`, and the tool call's line has `open=none items=0`

### Requirement: Tool call content in the dump
After a tool call's `--- entry` line, whether its row is open or closed, the dump SHALL print a `--- item` line per content item (the entry, the item's index, its kind and that kind's fields), a `--- hunk` line per hunk after its diff's line, a `--- location` line per location, and a `--- raw` line for each raw value the detail shows. Every text SHALL be escaped onto one line, and `--wait-for` SHALL match these lines.

#### Scenario: A diff in the dump
- **WHEN** the fake ACP agent reports a tool call whose diff of `/p/main.rs` replaces `old();` with `new();`, and the dump is taken
- **THEN** the dump has a line `--- item entry=N index=0 kind=diff op=modify path="/p/main.rs" added=1 removed=1 omitted=0` and a `--- hunk` line whose text holds `-    old();` and `+    new();`

#### Scenario: Command output in the dump
- **WHEN** the fake ACP agent reports a command `cargo test` whose output is `ok\n` with exit status 0
- **THEN** the dump has `--- item entry=N index=0 kind=output command="cargo test" exit=0 omitted=0 "ok\n"`

#### Scenario: Output with no command line
- **WHEN** the fake ACP agent reports a command's output `/p\n` with exit status 0 and no command line, as Codex does
- **THEN** the dump has `--- item entry=N index=0 kind=output exit=0 omitted=0 "/p\n"`, with no `command=` field

#### Scenario: Waiting for content
- **WHEN** Brindle runs with `--wait-for 'kind=diff'` against a fake ACP agent that sends a diff after a delay
- **THEN** the later steps run once the diff has arrived

#### Scenario: Raw input only as the fallback
- **WHEN** a tool call has a file change and a raw input
- **THEN** the dump has no `--- raw` line for it
