# Spec Delta

## MODIFIED Requirements

### Requirement: Lost connections reconnect
Brindle SHALL notice a remote connection that has gone silent, even when the transport never closes, within about half a minute. It SHALL then show that the connection was lost in each of that endpoint's terminals, and reconnect with growing pauses (at most 30 s apart) for as long as any terminal uses the endpoint. Where Brindle reports the loss (each terminal's line, its log, and the endpoint's row in the sessions palette), it SHALL say what ended the link:
- the SSH or container engine process exiting, with its exit status and what it wrote, or that its exit status couldn't be read, and why;
- nothing arriving within Brindle's own limit;
- or the far end closing the link.

#### Scenario: Network drop
- **WHEN** the network to `A8` is cut for two minutes while a tab there runs `htop`, and then restored
- **THEN** the tab says the connection was lost and why, and within 30 s of the restore it shows `htop` again, with the same pid

#### Scenario: ssh gives up
- **WHEN** the network to `A8` is cut, and `ssh` exits because its keepalives go unanswered
- **THEN** Brindle's log and the tab's line say the connection to `A8` ended because `ssh` exited, with its exit status and anything it wrote, and neither says that the terminal server closed the connection

#### Scenario: Silent link
- **WHEN** the link to `A8` delivers nothing for Brindle's limit, while `ssh` keeps running
- **THEN** Brindle's log and the tab's line say that nothing was read from `A8` in that time, and the `ssh` process is ended
