# Proposal

## Why

`fcis R7` says a pure helper compiles and is tested on every platform, whichever platform's data it handles, and `ports R7` says a seam translates handles and calls while its decisions come from such helpers. The 2026-11-16 audit (`docs/architecture/audits/2026-11-16.md`, finding 9: `fcis R7` 14, `ports R7` 7; change 10 of `2026-11-16-remediation.md`) found decisions that break both. They are written inside `cfg(windows)` or Linux-only modules, or inline in seam code next to the system calls, so:
- Linux CI never compiles or tests the Windows ones, which run only in the `windows` job;
- the Windows build never tests the Linux ones;
- some can only be reached with real pipes, processes, DACLs or directories.

The model already exists: `remote-agent/src/conpty/cmdline.rs` (`program_stem`, `launch_line`) is pure, ungated, and tested on Linux, and only the Windows pseudoconsole calls it. Change 5 (`enforce-platform-seams`) fixed where a platform `cfg` may go. This change moves the logic out from behind those `cfg`s, so `cargo test` on either platform checks the logic of both.

## What Changes

- **Each decision moves into platform-neutral code beside its seam.** The seam keeps only the gathering and the call, and the decision's unit tests run in `cargo test` on Linux and Windows. No behaviour changes.
  - **Shared crate (`brindle-remote-protocol`), because both binaries use these or their seam lives there:**
    - `access.rs` takes `Dacl`, `Ace`, `allows_only` and `SYSTEM_SID` from `win.rs`, with tests built from values (b21 #1, #2). It also takes the server's client check, `check` from the agent's `windows/listen.rs` (b14 #1).
    - `server_dir.rs` gains:
      - the pipe-name check, from `listen.rs`'s `pipe_name` (b14 #1);
      - the Unix holder-socket name parser, from `endpoint.rs`'s `#[cfg(not(windows))] shown` (b20 #3);
      - the helper copies' rules from `win.rs`: which copy is current, how a lost rename race ends, and which build directories a prune removes (b21 #3).
    - A new `hresult.rs` holds the one rule for the Win32 code inside an `HRESULT`. `win.rs`'s `os`, the agent's `event_loop::win32` and `pty::plain` (b15) use it.
  - **`brindle-remote-agent`:**
    - `server/platform/ready_arg.rs`: both platforms' `--ready-fd`/`--ready-handle` parsing and role arguments (b13 #1 and its Linux twin).
    - `server/platform/ensure_dir.rs`: what making sure of the Windows server directory does with what it found (b13 #6).
    - `server/platform/pipe_state.rs`: the bookkeeping of a Windows overlapped pipe, without its handles: what a finished read or write means, and when the pipe is readable, writable, sent or hung up (b14 #7).
    - `conpty/cmdline.rs` gains the Windows program choice and the environment merge (b15). `conpty/choice.rs` (new) holds the inbox switch and where the packaged ConPTY loads from (b09 #3, b15).
    - `conpty/hosts.rs` (new): which console host is a session's, and which processes a hang-up ends (b15).
    - `pty/program.rs` (new): the Linux login shell, home directory, `~` expansion, start directory and default `TERM`. `pty/stat.rs` (new): the `/proc/<pid>/stat` field parser (b10 #3, #4).
  - **`brindle` (the app):**
    - `desktop.rs`: stripping the `\\?\` prefix (b01 #7);
    - `notify/mod.rs`: `escape_markup`, moved beside `toast_xml` (b02 #4);
    - `workspace.rs`: `terminal_automatic_title` takes the local `PathStyle` from change 5 instead of `cfg!(windows)` (b07 #10);
    - `terminal/cmdline.rs`: `unix_default_program` (b28 #3);
    - `instance.rs`: the socket directory and the pipe name's user part (b23, which the plan missed).
- **Check:** `tests/architecture.rs` gains `the_neutral_code_is_never_gated`:
  - every whole pure module (change 7's `PURE_MODULES`, the new modules included) and `server_dir.rs` carry no platform `cfg` in production code, and neither do their `mod` declarations;
  - the neutral items left in seam or shell files carry no platform `cfg` either.

  The new modules join `fcis`'s "Whole pure modules" table. Planted-text tests show that the check finds a gated module and a gated item.
- **Docs:**
  - `functional-core-imperative-shell.md`: the new row, and Enforcement names the check;
  - `ports-and-adapters.md`: R7's examples;
  - `.claude/rules/{remote-agent,remote-protocol}.md`: "Enforced by".
- **Not in this change:**
  - b07 #11, the `#[cfg(unix)]` `shell_quote`, which change 7 (`keep-pure-modules-pure`) moves into `cmdline.rs`;
  - b31 #2, term-core's `cfg!(windows)`, which change 5 fixes;
  - b11 #1, the core's `client_reparent` pair, whose `fcis R7` half is the same lines as a `ports R4` site that change 5 lists until `port-to-macos`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This is a refactor with tests, so `.openspec.yaml` sets `skip_specs: true`. These requirements hold as written:
- `profiles`: the default program on Linux and Windows;
- `tabs`: "Tab titles", where a console host's default title is no title;
- `terminal-emulation`: "Program launch environment", "Pseudoconsole host on Windows" and "Built-in ConPTY limits on Windows";
- `terminal-server`: "Only the owning user can connect", "Server outlives the starter's job on Windows" and "Installed helper stays replaceable on Windows";
- `desktop-notifications`, `desktop-integration` ("Start Menu shortcut on Windows") and `command-line` ("A launch goes to the running Brindle").

## Impact

- **Code:**
  - `remote-protocol/src/{access,server_dir,endpoint,win,lib}.rs` and the new `hresult.rs`;
  - `remote-agent/src/server/platform/{mod.rs, unix/daemon.rs, windows/{daemon,dir,listen,pipe,pty,event_loop}.rs}` and the new `ready_arg.rs`, `ensure_dir.rs` and `pipe_state.rs`;
  - `remote-agent/src/conpty/{mod,cmdline,api}.rs` and the new `choice.rs` and `hosts.rs`;
  - `remote-agent/src/pty.rs`, `pty/unix.rs` and the new `pty/program.rs` and `pty/stat.rs`;
  - `src/{desktop,instance,workspace,zone_view}.rs`, `src/notify/{mod,dbus}.rs` and `src/terminal/{cmdline,mod}.rs`.
- **Check:** `tests/architecture.rs`.
- **Docs:** `docs/architecture/{functional-core-imperative-shell,ports-and-adapters}.md` and `.claude/rules/{remote-agent,remote-protocol}.md`.
- **Unchanged:** wire messages, protocol versions, config keys, state files, dependencies, and the `ports R4` exception list.
- **Depends on:**
  - **Change 5:** its scanner and seam set, and `PathStyle` with `process::LOCAL_PATH_STYLE`.
  - **Change 7:**
    - its `fcis R5` probe wording: three moved functions take a read-only `Fn(&Path) -> bool`, as change 7 does for `windows_default_program`;
    - `PURE_MODULES` and `production_lines` in `tests/architecture.rs`;
    - the `fcis` table split;
    - `program_title`.

    The plan's order lands 7 first.
- **Overlap:**
  - **`port-to-macos`:** its macOS shell, home and socket-directory choices become arguments to the neutral functions here (`unix_default_program`, `pty::program`, `instance::socket_dir`), instead of more `cfg` arms. This change doesn't edit `port-to-macos`.
  - **Remediation siblings:**
    - change 4 moves `environment_block` out of `conpty/cmdline.rs`;
    - change 7 edits `terminal/cmdline.rs`, `server_dir.rs`, `instance.rs` and `terminal_automatic_title`, and reshapes the `fcis` table;
    - change 8 edits `notify/mod.rs`;
    - change 11 edits the Windows test modules here;
    - change 12 will make `prune_helper_copies`, `packaged` and `kill_remaining` report their failures;
    - change 13 takes b13 #2's `millis` and b14 #6's flags.

    The design lists who adapts to whom.
  - **`add-pocket-client`:** `access.rs` stays pure values, which that change relies on.
