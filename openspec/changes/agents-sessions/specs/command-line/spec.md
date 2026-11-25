## MODIFIED Requirements

### Requirement: Fault switch listing
`brindle --list-fault-switches` and, where the helper has server roles (Linux and Windows), `brindle-remote --list-fault-switches` SHALL print the `BRINDLE_TEST_*` fault switches that build honours, one name per line, and exit with status 0 without opening a window or starting a server role. A build without fault switches, such as a release build, SHALL print nothing and exit with status 0. `--version` output SHALL NOT change.

#### Scenario: Debug build of Brindle
- **WHEN** a debug build runs `brindle --list-fault-switches`
- **THEN** it prints `BRINDLE_TEST_SINK_PANIC` and `BRINDLE_TEST_LAUNCH_LISTING`, each on its own line, and exits 0

#### Scenario: Debug build of the helper
- **WHEN** a debug build runs `brindle-remote --list-fault-switches`
- **THEN** it prints `BRINDLE_TEST_HOLDER_PANIC`, `BRINDLE_TEST_FEED_PANIC` and `BRINDLE_TEST_ACP_PANIC`, each on its own line, and exits 0

#### Scenario: Release build
- **WHEN** a release build runs `brindle --list-fault-switches` or `brindle-remote --list-fault-switches`
- **THEN** it prints nothing and exits 0

#### Scenario: Extra arguments to the helper
- **WHEN** `brindle-remote --list-fault-switches extra` runs
- **THEN** it prints an error saying the option takes no arguments and exits with a failure status
