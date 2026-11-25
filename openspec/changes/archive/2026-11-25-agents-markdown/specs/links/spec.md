## ADDED Requirements

### Requirement: Links in agent messages
Links in an agent message or thought (inline, reference, autolinks, bare URLs with a scheme from "URL detection", and images) SHALL be drawn in the link colour, underlined. A left click with ctrl held (without alt), released over the same link, SHALL open it as "Opening links" says, with its refusals, opener and config-line notices; the pointer SHALL be a pointing hand while ctrl is held over a link. Text in code SHALL NOT be a link.

#### Scenario: Ctrl-click a link
- **WHEN** the agent's message holds `[docs](https://example.com/docs)` and the user ctrl-clicks `docs`
- **THEN** the opener runs with `https://example.com/docs`, and nothing asks first

#### Scenario: Plain click
- **WHEN** the user clicks `docs` without ctrl
- **THEN** nothing is opened

#### Scenario: Bare URL
- **WHEN** the agent's message is `see https://example.com/a for more`
- **THEN** `https://example.com/a` is drawn as a link, and ` for more` is not part of it

#### Scenario: Refused scheme
- **WHEN** the agent's message holds `[run](javascript:alert(1))` and the user ctrl-clicks `run`
- **THEN** nothing is opened or asked, and the config line says the link was not opened because Brindle doesn't open that scheme

#### Scenario: URL in code
- **WHEN** the agent's message holds `https://example.com` inside backticks
- **THEN** it is drawn as code, not as a link

### Requirement: A link's target on hover
While the pointer is over a link in an agent message or thought whose shown text differs from its target, a tooltip SHALL show the whole target. A link whose text is its target SHALL show no tooltip.

#### Scenario: Text unlike its target
- **WHEN** the agent's message holds `[the docs](https://evil.example/login)` and the pointer rests on `the docs`
- **THEN** a tooltip shows `https://evil.example/login`

#### Scenario: Bare URL has no tooltip
- **WHEN** the pointer rests on the bare URL `https://example.com/a` in an agent message
- **THEN** no tooltip shows

### Requirement: Confirming links an agent wrote
A link from an agent message or thought whose scheme is one Brindle opens but is neither `http` nor `https` SHALL be opened only after the user confirms it in a dialog that shows the whole link, with Cancel focused when it opens and an Open button. Cancel, escape and a press on the backdrop SHALL open nothing. Links in terminals SHALL NOT ask.

#### Scenario: Mail link
- **WHEN** the agent's message holds `<mailto:dev@example.com>` and the user ctrl-clicks it
- **THEN** a dialog shows `mailto:dev@example.com` with Cancel focused, and the opener runs only after the user presses Open

#### Scenario: Enter by reflex
- **WHEN** that dialog has just opened and the user presses enter
- **THEN** the dialog closes and nothing is opened

#### Scenario: Refused file link
- **WHEN** the agent's message holds `file://otherhost/share/x` and the user ctrl-clicks it
- **THEN** no dialog opens, and the config line says the link was not opened because it names another host
