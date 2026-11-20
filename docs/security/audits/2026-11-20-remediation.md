# Remediation of the 2026-11-20 security audit

The OpenSpec change `fix-security-audit-2026-11-20` (archived under `openspec/changes/archive/`) fixes every high and medium finding in [`2026-11-20.md`](2026-11-20.md).

| # | Finding | Fix |
|---|---|---|
| 1 | High: program-reported UNC directories force NTLM authentication on Windows | `shell_integration::is_absolute_path`: a reported, recorded or revealed Windows path must be a plain drive-letter path. The existence check runs off the UI thread (`DirCheck`), and so does the spawn probe. |
| 2 | Medium: an elevated Brindle runs and deletes code in Medium-writable directories | The elevated server, helper copies and ConPTY files live in Administrators-only directories with a High label (`run-elevated`, `server-elevated`, `conpty-elevated`). Brindle holds them open while it uses them, and checks their final paths. Locks and logs open only as the role's own plain files. Bare program names resolve through `PATH` only (`.exe`/`.com`). |
| 3 | Medium: ctrl-click opens any scheme, UNC `file:` links and programs | `links::plan`: only detected schemes open. `file:` links must name this machine. On Windows, a file is revealed in Explorer instead of opened, and only on a drive-letter path. |
| 4 | Medium: an endpoint's listed session size aborts Brindle | Grids are limited to 2048×1024 (`MAX_COLS`, `MAX_ROWS`). A listed size outside the limits counts as no size, and the core clamps too. |
| 5 | Medium: the Windows instance pipe client trusts whoever serves it | The client opens with `SECURITY_IDENTIFICATION` and runs `check_server` (same SID and integrity) before writing. On Linux, it checks `SO_PEERCRED`. |

The `run-elevated` low finding is fixed with finding 2. The remaining lows are still open.

**Still to verify by hand:**
- the elevated real-process tests (`remote-agent/tests/launch.rs`, skipped unless elevated), run once on an elevated Windows shell;
- the manual checks in the change's task 4.3.
