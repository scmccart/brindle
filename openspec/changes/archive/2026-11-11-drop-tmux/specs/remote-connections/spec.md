## REMOVED Requirements

### Requirement: Remote tmux control mode
**Reason**: Brindle no longer has a tmux control mode, local or remote. A remote profile runs its command in an ordinary remote terminal like any other.
**Migration**: Set `command = "tmux"` and `args = ["new-session", "-A", "-s", "main"]` on the remote profile to run tmux in the remote terminal. The profile's `tmux` fields are ignored, with a warning.
