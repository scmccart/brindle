# Proposal

**Status: outline only.** Each tier below needs a decision before design and tasks are written. Versions were surveyed on 2026-11-11 with `cargo search` and `cargo update --dry-run`.

## Why

`Cargo.lock` was seeded from gpui 0.2.2's own lockfile and has been frozen since, by the rule in CLAUDE.md and README.md against broad `cargo update` ("a newer `libc` breaks `xattr`"). Dependabot only opens security PRs for Cargo. So Brindle has drifted:
- `cargo update --dry-run` finds 335 semver-compatible updates, with 16 more crates behind a major version.
- Several direct dependencies are one or two major versions behind.
- The GPUI fixes Brindle wants (#45) can't arrive through crates.io at all.

Bringing up to date what can move now makes later upgrades smaller, and it tests whether the lockfile rule still holds.

## What Changes

### Tier 1: semver-compatible lockfile updates

- Test the `libc`/`xattr` rule first. `xattr` 0.2.3 comes in only through `gpui` → `gpui_http_client` → `zed-async-tar`, and `libc` would move from 0.2.177 to 0.2.191. If the build and tests pass, the rule is stale. If they don't, record exactly what breaks.
- If the rule holds, update everything except `libc`, or update in groups (`cargo update -p …`), so a failure points at one crate. If it doesn't hold, do one `cargo update`.
- Rewrite the `Cargo.lock` notes in CLAUDE.md, README.md and `.github/dependabot.yml` to match what was found. If the lockfile can now move freely, consider letting Dependabot open version PRs for Cargo, not just security PRs.

### Tier 2: Brindle's own direct dependencies behind a major version

Each is a separate step, checked on Linux and Windows:

| Crate | Locked | Latest | Notes |
|---|---|---|---|
| `toml` | 0.8.23 | 1.2.1 | config parsing; check error messages and the settings tests |
| `toml_edit` | 0.22.27 | 0.25.19 | `config_edit`; check that edits keep comments and layout |
| `dirs` | 5.0.1 | 7.0.0 | config, state and runtime paths on both platforms |
| `getrandom` | 0.3.4 | 0.4.3 | session keys and tokens |
| `sha2` | 0.10.9 | 0.11.1 | helper digests; the deployed-helper cache key must not change, or every remote host redeploys once |
| `wayland-client` | 0.31.11 | 0.31.15 | semver-compatible; comes with tier 1 |
| `wayland-protocols` | 0.31.2 | 0.32.13 | activation tokens; the lock already has 0.32 through GPUI, so this also drops a duplicate |

### Tier 3: pinned to GPUI's versions

Brindle pins these to the versions GPUI builds, so the two share one copy and its types:
- `windows`: 0.61, latest 0.62.2.
- `x11rb`: `=0.13.2`, latest 0.14.0. A dev-dependency for the e2e captures.
- `polling`: `=3.11.0`, already the latest. It names alacritty's types.
- `zbus`: 5.12 locked, 5.19 compatible, latest 6.2.1. GPUI's `ashpd` uses 5.

They move only together with GPUI or alacritty, so this change bumps them only within those crates' requirements.

### Tier 4: GPUI and alacritty_terminal

- **alacritty_terminal**: 0.26.0, already the latest release. Nothing to do.
- **gpui**: 0.2.2 is the latest on crates.io. Zed's in-tree `crates/gpui` still says 0.2.2, so a newer GPUI means a git dependency on a Zed commit, with its own lockfile seed and possibly API changes across Brindle and `brindle-ui`.
- That brings in zed-industries/zed#62081 (a blank new window) and #63089 (a panic on a window-manager close; Brindle's deferred quit in `src/main.rs` may then go), tracked as #45.
- It may also clear the `proc-macro-error2`/`num-bigint-dig` future-incompat warning that CLAUDE.md leaves on purpose.

### Not in scope

- The Rust toolchain: the pin is 1.99.1, already the latest stable.
- GitHub Actions versions: Dependabot opens monthly PRs for these.

## Open questions

- Does tier 4 belong in this change? A git dependency on Zed changes how Brindle is built and published (crates.io publishing, reproducibility, the vendored lockfile). It could be its own change, `upgrade-gpui`, with this change limited to tiers 1–3.
- If tier 1 shows the `libc` rule is stale, should the lockfile follow `cargo update` routinely, for example through monthly Dependabot version PRs?
- How should the work land: one PR per tier, or one PR per major bump in tier 2, so a regression bisects to one crate?
- What checks each step? At least `cargo test --workspace`, the e2e suite under `scripts/with-xvfb`, and the Windows build and tests. Tier 4 adds the manual checks list in CLAUDE.md.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- None expected. These are dependency updates with no behaviour change. If a bump forces a visible change, such as a config error message from `toml` 1.x, the affected capability gets a delta then. Otherwise the change sets `skip_specs: true` once tasks are written.

## Impact

- `Cargo.toml` (root, `term-core`, `remote-protocol`, `remote-agent`, `ui`) and `Cargo.lock`.
- Code that uses the tier 2 crates:
  - `toml`/`toml_edit`: `config.rs`, `config_edit.rs`, `session.rs`, `theme.rs`, `settings_dialog.rs`;
  - `dirs`: config, state and server paths;
  - `getrandom`: `remote/local.rs`, `remote-agent/src/server/core.rs`;
  - `sha2`: `remote/client.rs`'s helper cache;
  - Wayland activation: `notify/activation.rs`.
- Documentation: the lockfile rule in CLAUDE.md, README.md and `.github/dependabot.yml`; the future-incompat note in CLAUDE.md if tier 4 lands.
- CI: no workflow changes expected. The `test` job's version check and the release builds cover the result.
- Issues: #45 (GPUI window fixes) if tier 4 stays in.
