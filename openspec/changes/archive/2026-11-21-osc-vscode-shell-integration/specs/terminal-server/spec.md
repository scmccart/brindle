# Spec Delta

## ADDED Requirements

### Requirement: Command lines survive attach
The server SHALL keep each command's reported line (`OSC 633 ; E`) with its prompt marks, and an attaching or reconnecting client SHALL receive it with the marks. A client attached to a server that predates command-line tracking SHALL have the marks without the reported lines.

#### Scenario: Copy after attach
- **WHEN** a detached session's shell reported `make test` with `633;E`, and a client attaches and runs Copy Last Command Line
- **THEN** the clipboard holds `make test`
