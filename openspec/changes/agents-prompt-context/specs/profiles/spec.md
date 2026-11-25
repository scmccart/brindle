## ADDED Requirements

### Requirement: A remote's attachments directory
A profile's remote endpoint, of either kind, SHALL accept an optional `attachments_dir`: the directory on that host where the text attached to an agent's prompts is kept. An agent profile on a remote that sets none SHALL keep attached text under that host's own cache directory. This machine's `[agent] attachments_dir` SHALL NOT be used for a remote, since a path belongs to one machine.

#### Scenario: Set for a host
- **WHEN** an agent profile has `remote = { type = "ssh", host = "A8", attachments_dir = "/data/brindle" }` and the user sends a prompt with an attached text
- **THEN** the text is kept under `/data/brindle/` on `A8`

#### Scenario: Not set for a host
- **WHEN** an agent profile's remote sets no `attachments_dir`, and this machine's config sets `attachments_dir = "/home/me/agent-files"` under `[agent]`
- **THEN** that profile's attached text is kept under the remote user's cache directory on the host, and nothing is written to `/home/me/agent-files` there

### Requirement: A remote's attachments directory is not part of its endpoint
A remote's `attachments_dir` SHALL NOT be part of the endpoint: profiles that differ only in it SHALL share one connection to the host, and the session state SHALL NOT record it. A terminal profile's remote MAY set it, and it SHALL change nothing for that profile.

#### Scenario: One connection
- **WHEN** two profiles name the same SSH host and differ only in `attachments_dir`
- **THEN** their tabs share one connection to the host

#### Scenario: On a terminal profile
- **WHEN** a terminal profile's remote sets `attachments_dir`
- **THEN** the config is valid, and the key changes nothing for that profile's tabs
