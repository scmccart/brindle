## MODIFIED Requirements

### Requirement: Command sessions survive short drops
A remote `-e` command terminal's session SHALL NOT end when its connection drops, if Brindle reconnects within 5 minutes of the host noticing the drop. It SHALL end if Brindle doesn't reconnect in that time.

#### Scenario: Short drop
- **WHEN** a tab opened with `brindle terminal A8 -e htop` loses its connection for one minute
- **THEN** after the reconnect the tab shows the same `htop` process

#### Scenario: Brindle crashed
- **WHEN** Brindle is killed with SIGKILL while that tab runs `htop`
- **THEN** `htop` on `A8` ends within about 6 minutes, and isn't listed after that
