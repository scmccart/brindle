# Spec Delta

## ADDED Requirements

### Requirement: Output commands need shell integration
Select Last Command Output and Copy Last Command Output SHALL do nothing, and the command palette SHALL NOT list them, until the terminal has received at least one prompt mark (`A`, from OSC 133 or 633), as for Copy Last Command Line. Once a prompt mark has been received they stay listed for that terminal.

#### Scenario: Hidden without a prompt mark
- **WHEN** a terminal has received no prompt mark, as with a shell that loaded no integration script, and the user opens the command palette and filters for "last command"
- **THEN** the palette lists none of Select Last Command Output, Copy Last Command Output and Copy Last Command Line, and running the `select_last_output` or `copy_last_output` action leaves the selection and the clipboard unchanged

#### Scenario: Listed once integration loads
- **WHEN** a shell without integration loads Brindle's script with `eval "$(brindle --shell-integration bash)"`, and its next prompt arrives
- **THEN** the palette lists Select Last Command Output and Copy Last Command Output for that terminal
