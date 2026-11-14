## MODIFIED Requirements

### Requirement: The window stays responsive during continuous output
While any terminal in a window (a local tab or zone, or a remote terminal) receives output continuously, the window SHALL keep repainting that terminal's latest output, SHALL keep handling input and actions, and Brindle SHALL still quit when asked. Updates during continuous output MAY be grouped, but output that arrives after a quiet spell SHALL be shown without added delay.

#### Scenario: Flooding tab keeps painting
- **WHEN** a Windows tab runs `for /L %i in (1,0,2) do @echo flood %i` in `cmd` for several seconds
- **THEN** the window repaints with new `flood` lines while the loop runs and Windows never reports it as not responding

#### Scenario: Quitting while a tab floods
- **WHEN** a tab is printing continuously and Brindle is asked to quit (for example by `--dump-screen-after`)
- **THEN** Brindle prints its dump and exits within a second of the deadline

#### Scenario: Echo after a quiet spell
- **WHEN** a tab has had no output for a while and the user types a character that the shell echoes
- **THEN** the echo is painted on the next frame, with no pacing delay added
