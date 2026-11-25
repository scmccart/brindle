## ADDED Requirements

### Requirement: Where a local agent's attachments are kept
The config SHALL control, with `attachments_dir` in the `[agent]` table, the directory that keeps the text attached to the prompts of agents that run on this machine. Unset, it SHALL be `attachments` under Brindle's cache directory. A leading `~` SHALL mean the user's home directory.

#### Scenario: Default
- **WHEN** the config sets no `attachments_dir`
- **THEN** a local agent's attached text is kept under `~/.cache/brindle/attachments/`

#### Scenario: A directory for this machine
- **WHEN** the config sets `attachments_dir = "~/agent-files"` under `[agent]` and the user attaches a command's output to a local agent's prompt and sends it
- **THEN** the text is kept under `agent-files` in the user's home directory

### Requirement: How long attachments are kept
The config SHALL control, with `attachments_keep_days` in the `[agent]` table, how many days kept text stays after its agent session ends: 14 by default, with 0 removing it when the session ends. A value outside 0 to 3650 SHALL be clamped, and the window's config line SHALL say so, as for an out-of-range font size. It SHALL apply to agents on every host.

#### Scenario: Default
- **WHEN** the config sets no `attachments_keep_days`
- **THEN** a session's attached text stays 14 days after it ends

#### Scenario: A keep time
- **WHEN** the config sets `attachments_keep_days = 2` under `[agent]`
- **THEN** sessions started from then on keep their attached text for 2 days after they end

#### Scenario: Out of range
- **WHEN** the config sets `attachments_keep_days = 100000`
- **THEN** 3650 is used, and the config line says `agent.attachments_keep_days` 100000 is out of range and 3650 is used

### Requirement: Attachment settings apply to sessions started afterwards
A reload SHALL apply the attachment settings to agent sessions started after it. A session that is already running SHALL keep the directory and the keep time it was started with.

#### Scenario: Changed while a session runs
- **WHEN** an agent session is running, and the user sets `attachments_dir` to another directory and saves
- **THEN** that session's attached text is still kept where it was, and a new agent session keeps its text in the new directory
