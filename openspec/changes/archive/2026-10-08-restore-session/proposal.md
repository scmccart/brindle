# Proposal

## Why

Each launch starts from one tab with the default profile. People who keep several tabs open in known directories, and tmux control-mode sessions beside them, set that up again after every logout, reboot or crash. tmux keeps the sessions themselves alive, but Brindle forgets which ones it showed and where its local tabs were. An opt-in restore fixes this without changing anything for people who don't want it.

## What Changes

- New config setting `restore_session` (boolean, default `false`). When it is on, Brindle records its windows and tabs while it runs. A **plain launch** (no `-p`, `-d` or `-e`) then reopens what was open when Brindle last quit:
  - every window, in order, with its size (and whether it was maximized);
  - each window's tabs in order, and its active tab;
  - each local tab's profile and the working directory of its foreground process. This uses the same `/proc` lookup that new tabs use to inherit a directory;
  - classic tmux tabs (`tmux = "plain"`) rerun their profile, which reattaches through `new-session -A`;
  - tmux control-mode tabs: each session is reattached **once**, and the attach brings back all of the session's windows as tabs, at the position the session's tabs had. Attaching still keeps tmux's current window and active panes.
- The state is a small versioned TOML file, `session.toml`, in `$BRINDLE_STATE_DIR` when that is set, otherwise in `$XDG_STATE_HOME/brindle/` (default `~/.local/state/brindle/`). It is always written atomically (a temp file renamed into place):
  - within about a second of a change to tabs, windows or directories, so a crash or logout leaves a recent snapshot;
  - again when Brindle quits.
- Closing a window's last tab, so that the window closes, removes that window from the snapshot. Quitting, or closing the last window while it still has tabs, keeps it.
- Restore is robust:
  - a profile that no longer exists falls back to the default profile;
  - a directory that no longer exists falls back to the profile's directory, or the home directory;
  - an unreadable, corrupt or unknown-version state file is ignored with a log line, and a normal launch happens;
  - a tmux session that no longer exists is created again by the attach (`new-session -A`), as opening the profile would do. If tmux can't attach at all, the existing attach-failure tab explains why.
- New command-line flag `--no-restore` skips restoring for one launch.
- Only one Brindle process at a time owns the state file. A second process started while one is running neither restores nor saves, so it can't duplicate tabs or overwrite the first process's snapshot.
- **Out of scope:**
  - **scrollback and screen contents of local tabs are not restored**; restored shells start fresh in their directory;
  - running programs and `-e` commands are not restarted: such a tab comes back as its profile's program in the same directory;
  - window positions are not restored.

## Capabilities

### New Capabilities

- `session-restore`: recording the open windows and tabs, the state file (location, format, atomic writes, when it is written, single owner), restoring on a plain launch, and the fallbacks for missing profiles, directories, sessions and bad state files.

### Modified Capabilities

- `configuration`: the behaviour settings gain `restore_session`.
- `command-line`: a plain launch restores the session when the setting is on, and the `--no-restore` launch option is added.

## Impact

- **Code:**
  - a new `src/session.rs` (state model, serialization, path, atomic write, lock, restore planning; unit-tested);
  - `config.rs` and `assets/default-config.toml`: the `restore_session` setting;
  - `main.rs`: `--no-restore`, restore at launch with window sizes, the periodic writer and a final write on quit;
  - `workspace.rs`: records where each tab came from, takes snapshots, restores a window's tabs (tmux sessions are attached once and placed after the tab that preceded them, and their first window activation is kept from taking over a restored local active tab).
- **Dependencies:** none new. `serde`, `toml` (serialization), `dirs` (`state_dir`) and `libc` (`flock`) are already dependencies.
- **Specs:** a new `session-restore` spec, and deltas to `configuration` and `command-line`.
- **Docs:** README usage and settings.
- **Tests:**
  - unit tests for serialization round trips, version and corruption handling, and the profile and directory fallbacks;
  - e2e cases for a local round trip (save on quit, restore tabs, directories and the active tab), for fallbacks, and for a tmux control-mode restore on a throwaway server that checks the session is attached once and tmux's current window is kept.
