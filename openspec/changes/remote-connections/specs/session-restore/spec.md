## ADDED Requirements

### Requirement: Remote endpoint identity
A recorded local tab backed by a remote profile SHALL store the SSH or container target identity used when its CWD and zone directories were observed. Restore SHALL reuse those directories only when the current profile resolves to the same endpoint. State written before endpoint identity was recorded, a missing/renamed profile, or a profile whose endpoint changed SHALL discard the recorded directories and use the current profile's remote directory default.

#### Scenario: SSH profile changes host
- **WHEN** a tab recorded `/srv/old` for profile `Remote` on SSH host `old-host`, and the profile now points to `new-host` with `cwd = "/home/new"`
- **THEN** restore starts on `new-host` in `/home/new` and never sends `/srv/old`

#### Scenario: Existing local state
- **WHEN** an older state file has no endpoint identity and resolves to a local profile
- **THEN** its local directory is restored under the existing local-directory fallback rules
