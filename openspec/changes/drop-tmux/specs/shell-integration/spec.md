## REMOVED Requirements

### Requirement: tmux panes
**Reason**: Brindle has no tmux control-mode panes. Inside an ordinary tab, tmux does not pass OSC 133 to the outer terminal unless configured to, so the marks are whatever the outer terminal receives, as for any program.
**Migration**: None.
