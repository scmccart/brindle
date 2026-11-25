## ADDED Requirements

### Requirement: Attachments in the dump
In an agent zone's dump, a user message's `--- entry` line SHALL say how many attachments it has (`attachments=N`) when it has any, followed by one `--- attached` line each: the entry, the index, the kind, and a link's path and name or a text's name, which end was cut, its size and its text. Texts SHALL be escaped onto one line, and `--wait-for` SHALL match these lines.

#### Scenario: A sent attachment
- **WHEN** the user sent `why?` with a link to `src/main.rs`, and the dump is taken
- **THEN** the dump has a user entry line with `attachments=1` and a line `--- attached entry=N index=0 kind=link path="src/main.rs" name="main.rs"`

### Requirement: Waiting attachments in the dump
Before the `--- prompt` line, an agent zone's dump SHALL print one `--- draft` line per attachment waiting in the prompt input, in the form of an `--- attached` line without the entry. A `--- queued` line SHALL say how many attachments its prompt has (`attachments=N`) when it has any. `--wait-for` SHALL match these lines.

#### Scenario: A draft
- **WHEN** a text attachment named `cargo test` holding `ok\n` waits in the prompt input, and the dump is taken
- **THEN** the dump has `--- draft index=0 kind=text name="cargo test" cut=none bytes=3 "ok\n"`

#### Scenario: A queued prompt
- **WHEN** a prompt `next` with one attachment waits in the queue, and the dump is taken
- **THEN** the dump has `--- queued waiting attachments=1 "next"`

### Requirement: Driving attachments
`--action` SHALL run `agent_attach_file`, whose path prompt takes `--send` and is confirmed with `picker_confirm`, `agent_clear_attachments`, and, in a terminal zone, `send_selection_to_agent` and `send_last_output_to_agent`, whose list of agent zones is driven with the list actions. After a terminal action the dump SHALL be the agent zone's, since it is then the active one.

#### Scenario: Output to an agent, end to end
- **WHEN** Brindle runs with a terminal tab and an agent tab, runs `printf 'one\ntwo\n'` in the terminal, waits for the prompt, and runs `--action send_last_output_to_agent --dump-screen-after 2`
- **THEN** the dump is the agent zone's, with a `--- draft` line of kind `text` whose text holds `one` and `two`, and its focus line reads `agent stop=prompt`

#### Scenario: Attach a file by path
- **WHEN** Brindle runs `--action agent_attach_file --send 'src/main.rs' --action picker_confirm --dump-screen-after 2` in an agent zone
- **THEN** the dump has `--- draft index=0 kind=link path="src/main.rs" name="main.rs"`
