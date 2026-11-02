# Proposal

Outline only: the cause is not known yet.

## Why

The e2e case `windows-close-mid-output` checks that closing a tab whose program is flooding output doesn't block Brindle. It floods `cmd` with `for /L %i in (1,0,2) do @echo flood %i`, opens a second tab, goes back to the first, closes it, and expects the dump on time.

On one Windows 11 desktop it fails every time with "Brindle didn't exit". Brindle never reaches its `--dump-screen-after` deadline, and the harness gives up after about 36 s. `close.log` holds only startup lines, ending at font fallback.

- It failed 6 runs out of 6 on the `feat/remoting` branch, and 3 out of 3 on a build of `main` (`d9c1801`), so it predates that branch.
- The `main` baseline run didn't have the packaged `conpty.dll` copied beside it, so it may have used a different ConPTY than the branch build.
- It passes on the CI `windows-latest` runner, including at `c800343`.
- It is not known when it last passed on that desktop. Its previous output there is from 2026-10-17.

A machine-specific hang when closing a flooding tab could still be a real bug: the case exists because closing such a tab once blocked.

## What Changes

To be decided once the cause is found. First steps:

- Reproduce with the same ConPTY on both builds: the packaged `conpty.dll`, then `BRINDLE_CONPTY=inbox`.
- Find where the process stops: which thread blocks, and whether the GPUI thread is still running.
- Compare the desktop with the CI runner: ConPTY version, Windows build, debug versus release, and whether a display is attached.

## Capabilities

To be decided. Possibly none, if this is a test or environment fix.
