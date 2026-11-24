# Proposal

**Depends on:** nothing.
**Next:** `agents-sessions`. It is independent of this one, so the two can also be applied in either order or side by side; once both are archived, `agents-profiles` adds `brindle agent` to the subcommand form this change builds.

## Why

Brindle's launch options are flat: `-p PROFILE`, `-d DIR` and `-e COMMAND…` describe one kind of tab, a terminal. The agent series will add a second kind, an agent tab, whose launch takes a profile, a directory and an initial prompt but never a command. Folding both into one flat option set would make `-e` meaningless for an agent and `-p` ambiguous (profile or prompt), and would leave the parser able to represent a terminal launch with a prompt. Splitting the launch into one subcommand per tab kind, before any agent code lands, gives each kind its own grammar and lets `agents-profiles` add `brindle agent` without reshaping the parser, the hand-off to a running Brindle or the harnesses again.

## What Changes

- Add `brindle terminal [PROFILE] [-d DIR] [-e COMMAND [ARGS]...]`, which opens a terminal tab: the named profile (or the default one), in DIR, running COMMAND. `-e` still takes every argument after it, so it comes last. `--working-directory` and `--command` stay as long forms of `-d` and `-e` under the subcommand.
- **BREAKING** Remove the flat `-p/--profile`, `-d/--working-directory` and `-e/--command`, with no deprecation period. Each is now a usage error (status 2, before any window opens) that says what to type instead, for example `-p work` → `brindle terminal work`. Scripts, desktop launchers and shortcuts that pass them must change.
- Reserve `brindle agent`: until `agents-profiles` adds it, `brindle agent …` is a usage error saying this Brindle has no agent tabs. Any other first bare word (`brindle work`) is a usage error that points at `brindle terminal`.
- Bare `brindle` is unchanged: it opens (or raises) the window with the default profile when nothing is restored. `brindle terminal` with no profile asks for a new default-profile tab, including when handed to a running Brindle, which a bare launch does not.
- **BREAKING** An unknown profile name is a usage error: `brindle terminal wrk` exits with status 2, before any window opens, listing the profile names (on Windows without a console, in a message box). The launch's fallback to the default profile and its dim line are gone. When the launch is handed to a running Brindle, the running Brindle checks the name against its own config and its refusal makes the launch exit 2 too. The profile menu and picker keep their handling of a profile that has gone since they were shown.
- A profile whose kind doesn't match the subcommand is a usage error. Every profile is a terminal profile until `agents-profiles` adds kinds, so the check never refuses here; it exists so that change only adds a kind.
- A launch whose hand-off request would exceed the running Brindle's 1 MiB request limit is refused before anything is sent, saying so, and a running Brindle answers an oversized request with that reason instead of a JSON error.
- `--attach ID` stays a top-level option and cannot be combined with a subcommand (each asks for the launch's tab).
- The debug options, the informational options, the desktop install options, `--config` and `--no-restore` work before or after the subcommand, and the hand-off to a running Brindle works with a subcommand and without one, across versions: the hand-off request gains an optional `kind` field (additive; omitted by a bare launch) that a running Brindle from this change on decodes as a closed set, refusing a kind it doesn't know instead of opening something else.
- Every caller of the flat options moves to the subcommand: the e2e harness (`Run`, the `instance-*` launcher, the Windows cases), the perf cases, the throwaway scripts, README, `CLAUDE.md`, `docs/code-map.md` and the clipboard manual check. The release note for the breaking change goes in the squash title (`feat(cli)!: …`).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `command-line`: the launch options become the `terminal` subcommand and the reserved `agent` word; the flat options are removed and refused with what to type instead; an unknown profile name is refused instead of falling back; the restore-then-open and hand-off requirements name the subcommand, the new `brindle terminal` default tab, the hand-off across versions and its size limit; the notification-log scenario's `-e` launch is rewritten.
- `tabs`: the quit requirement's scenario launches with `brindle terminal -e`.
- `terminal-emulation`: the terminal-server requirement's crash scenario launches with `brindle terminal -e`.
- `remote-connections`: the command terminal grace period's scenario launches with `brindle terminal A8 -e htop`.
- `session-restore`: the command tab requirement's scenario launches with `brindle terminal -e htop`.

## Impact

- **Code:** `src/main.rs` (the parser, `Args`, the usage text and its unit tests; the launch's profile check before any window), `src/instance.rs` (the request's `kind` field, its decoding into a closed launch, the `usage:` answer, the size check, transcript tests), `src/workspace.rs` (`profile_for` takes the kind and refuses an unknown name; `Chosen`'s missing name and note and `LaunchRequest.note` go), `docs/code-map.md`. `src/session.rs` and `src/desktop.rs` don't change: no launch's options are recorded, and the desktop entry, Start Menu shortcut and WinGet link run Brindle with no arguments.
- **Tests:** `tests/e2e/brindle.rs` (`Run`), `tests/e2e/cases/{cli,instance,win}.rs`, `tests/perf/cases.rs`.
- **Scripts and docs:** `scripts/run-throwaway.sh` and `.ps1` (their usage examples), README's Usage, Session restore and Remote tabs sections, `CLAUDE.md` (the throwaway example and the `Run::command` note), `docs/manual-checks/clipboard.md`. No CI workflow, installer or packaging file passes the flat options.
- **Behaviour:** anything that runs `brindle -p/-d/-e` stops working with status 2 and a message saying what to type, and so does a launch naming a profile the config doesn't have. A newer launch handed to an older running Brindle still opens a terminal tab with a profile, directory or command; `brindle terminal` alone only raises an older window, and an older window still falls back to its default profile for an unknown name.
- **Release:** a breaking change; the PR title and squash commit carry `!`, and the release notes give the old-to-new mapping.
