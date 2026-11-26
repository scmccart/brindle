## MODIFIED Requirements

### Requirement: Agent zone title
An agent zone's title SHALL be its conversation's title once the agent has given one, else its profile's name; the name the agent gave in `initialize` SHALL NOT title it. A tab whose active zone is an agent zone SHALL take that title, unless it has a manual title, which SHALL win over every later conversation title.

#### Scenario: Agent's name
- **WHEN** a zone of profile `Fake` runs a fake ACP agent that names itself `Fake Agent` in its answer to `initialize`
- **THEN** the zone's title and its tab's title are `Fake`

#### Scenario: The agent names the conversation
- **WHEN** that agent then sends `session_info_update` with title `Hello.txt file creation`
- **THEN** the zone's title and its tab's title are `Hello.txt file creation`

#### Scenario: Title cleared
- **WHEN** the agent then clears the title with `null`
- **THEN** the zone's title and its tab's title are `Fake` again

#### Scenario: A renamed tab
- **WHEN** the user renames the agent's tab `Build`, and the agent then titles its conversation `Hello.txt file creation`
- **THEN** the tab's title stays `Build`, and the zone's own title is `Hello.txt file creation`
