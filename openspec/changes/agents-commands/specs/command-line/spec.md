## ADDED Requirements

### Requirement: Commands and options in the dump
In an agent zone's dump, after the `--- agent` line, the dump SHALL print a `--- commands count=N` line when the session has commands, and one `--- option` line per session option in the order the zone shows them (its id, category, kind, current value, name, and the number of choices for an option with choices). `--wait-for` SHALL match these lines.

#### Scenario: Options in the dump
- **WHEN** the fake ACP agent offers a mode option with five choices and `auto` current, and the dump is taken
- **THEN** the dump has `--- option id="mode" category=mode kind=select current="auto" name="Mode" choices=5`

#### Scenario: A toggle in the dump
- **WHEN** the fake ACP agent offers an on/off option `Fast mode` of category `speed` that is off
- **THEN** the dump has `--- option id="fast" category=other:speed kind=toggle current=off name="Fast mode"`

#### Scenario: Waiting for a change
- **WHEN** Brindle runs `--action agent_next_mode --wait-for 'current="plan"'` against a fake ACP agent that confirms the mode `plan`
- **THEN** the later steps run once the option has changed

### Requirement: The command list in the dump
While the slash command list shows, the dump SHALL print after the `--- prompt` line a `--- completion` line with the typed word and the highlighted row's index, then one row per listed command as palette rows are printed. `--key` SHALL drive the list as real keys do.

#### Scenario: The list in the dump
- **WHEN** the fake ACP agent offers `compact` and `context`, and Brindle runs `--send '/co' --dump-screen-after 2`
- **THEN** the dump has `--- completion query="co" highlighted=0` followed by a row for `/compact` and a row for `/context`

#### Scenario: Accepted with a key
- **WHEN** Brindle runs `--send '/co' --key down --key enter --dump-screen-after 2`
- **THEN** the dump's input line shows the text `/context `, and no `--- completion` line
