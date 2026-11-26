---
paths:
  - "remote-agent/src/**"
  - "remote-agent/build.rs"
  - "remote-agent/Cargo.toml"
---
# brindle-remote-agent: the terminal server's roles, the relay, the CLI

You are editing the `brindle-remote` helper, which holds the user's running programs. Read `docs/architecture/process-architecture.md` and `docs/architecture/ports-and-adapters.md` before non-trivial changes, then `railway-oriented-programming.md`.

- The holder parses nothing and has no policy; what it stores is opaque bytes the server core owns. Don't add features to it (`process R1`).
- Session state in the server core must survive a core restart: checkpoint it or send it with `SET_META`, or say what a restart does to it (`process R2`).
- A call on outside data is confined per session; a panic breaks one emulator, not the server (`process R3`).
- `server/core.rs`, `server/core/` and `server/holder.rs` name no `libc::`, `windows::` or `std::os::` item. What they need from the system is an item in `server/platform/{unix,windows}`, under the same name in both (`ports R5`).
- Platform modules are thin: they gather, translate handles and call. What they decide is in an ungated pure module beside them, tested on every platform: `server/platform/{ready_arg,ensure_dir,pipe_state,wait,piped}.rs` (`piped`: a pipe session's Windows flags, handle list and job, its program and the batch-file hint, its hang-up steps), `conpty/{cmdline,choice,hosts}.rs` (`launch_line`, the program, the environment, which ConPTY, which processes a hang-up ends) and `pty/{program,stat}.rs`. A new decision goes into one of them, or a new one in `fcis`'s "Whole pure modules" table and `PURE_MODULES` (`ports R7`, `fcis R7`).
- The relay runs on Linux only: `relay` (and `liveness`) are declared under `target_os = "linux"`, and every other platform, macOS included, refuses `relay`. Platform `cfg`s in production code stay in their seams: `server/platform/`, `pty`, `conpty` and the relay's `server/relay.rs` and `liveness.rs`, with the files on `docs/architecture/README.md`'s `ports R4` row excepted until `port-to-macos` (`ports R4`).
- One thread per role on the readiness loop; handlers do IO through a source's owner; queues have marks; closes are graceful (`process R8`–`R10`).
- Liveness is stamped on the reader thread, never where output is copied (`process R18`).
- No `gpui`, no `anyhow`, no dependency beyond the three shared crates it builds on (`brindle-remote-protocol`, `brindle-term-core`, `brindle-transcript`), `agent-client-protocol-schema`, `similar` (only in `src/acp/diff.rs`), `libc`, `windows` and `embed-resource` (`ports R3`).
- ACP is decoded only in `src/acp/`: no other file names `agent_client_protocol_schema`, so the core's shell reaches ACP through `acp::reduce` and never decodes it itself (`ports R3`). Transcript events are built and encoded with `brindle-transcript`, never redefined here.
- One module per ACP version (`ports R14`): `acp/v1/` alone names `agent_client_protocol_schema::v1` and v1's method and field names; the session's `Dialect` is matched only in `acp/dialect.rs`; the reducer, `rpc.rs` and the checkpoint read `acp/forms.rs` and `acp/envelope.rs`, never a version. A new message is a form in `forms.rs`, a `Dialect` method, and that version's decoder or encoder.

If the app needs something from here, move it to `brindle-remote-protocol`, `brindle-term-core` or `brindle-transcript`: the app never depends on this crate.

Enforced by: `tests/architecture.rs` (`acp_versions_stay_in_their_modules`, `the_server_core_and_holder_name_no_platform_type`, `platform_cfgs_stay_in_their_seams`, `the_neutral_code_is_never_gated`, which keeps the pure modules beside the seams free of platform `cfg`s, `the_pure_modules_read_nothing_ambient`, `crates_below_the_app_name_only_their_allowed_dependencies`, `only_the_acp_modules_name_the_schema_crate`) and the real-role tests in `remote-agent/tests/`.
