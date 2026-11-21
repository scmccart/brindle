## 1. Pure code

- [x] 1.1 Add `workspace/notices.rs` with `Notices` (`push`, `expire`, `dismiss_all`, `texts`). Unit tests cover:
  - a repeated text replacing the earlier entry;
  - the cap of 3;
  - expiry at a given `now`;
  - the next deadline.
- [x] 1.2 Make `session::state_dir` read `StateEnv::elevated`. Unit tests cover:
  - the elevated default on Windows;
  - `BRINDLE_STATE_DIR` winning over it;
  - Linux ignoring it.

## 2. Shell

- [x] 2.1 `TerminalView` emits `LinkFailed` for a refused link and for an opener's failure, and no longer calls `Terminal::note` for them. `Workspace` pushes each into `Notices`, passes them to `config_line` and runs one expiry timer.
- [x] 2.2 Add the `dismiss_notices` action, and the config line's dismiss button with a tooltip.
- [x] 2.3 `main.rs` passes the elevation into `StateEnv`, and the elevated state directory is made with `win::protected_dir(…, Contents::Executables)`, which sets aside a directory found without its protection.
- [x] 2.4 On Windows, a launch with no console shows a failed hand-off in a message box.

## 3. Tests

- [x] 3.1 Update the e2e links cases:
  - Each refused or failed link appears on the dump's `--- config` line, and not in the screen text.
  - A new case checks that the notice is gone after `NOTICE_LIFETIME`. Time the dump against that limit unscaled, through `Run::dump_after_last_step`.
  - A new case checks that `dismiss_notices` clears the notice.
- [x] 3.2 Add an e2e Windows case: an elevated and an unelevated Brindle run at once, each with its own `session.toml`. It skips without an elevated runner.
- [ ] 3.3 Manual checks:
  - a pointer click on the config line's dismiss button;
  - a Start Menu launch that is refused shows its message box.
