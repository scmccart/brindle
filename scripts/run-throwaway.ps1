# Runs Brindle with throwaway state, for trying a build by hand without
# touching your real config, saved session or terminal server (the Windows
# twin of run-throwaway.sh):
#
#   scripts/run-throwaway.ps1                      # a fresh default config
#   scripts/run-throwaway.ps1 -Config FILE         # use FILE in place
#   scripts/run-throwaway.ps1 -Keep                # keep the directory afterwards
#   scripts/run-throwaway.ps1 -NoLinuxHelper       # don't provide the Linux helper
#   scripts/run-throwaway.ps1 -BrindleArgs 'terminal','-d','C:/Temp'   # arguments for brindle
#
# run-throwaway.sh takes the same options, less -NoLinuxHelper: only a
# Windows build lacks the Linux helper.
#
# It makes a private directory under %TEMP% and points BRINDLE_STATE_DIR,
# BRINDLE_SERVER_DIR and BRINDLE_CACHE_DIR (where a local agent's attached
# text is kept) into it. So the launch doesn't hand itself to a Brindle
# you already have running (the single-instance lock and its named pipe are
# keyed by the state directory), and the sessions run in their own terminal
# server, which is stopped (and the directory removed) when Brindle exits.
# -Keep keeps the directory, for its logs and config, but still stops the
# server: the next run makes a new directory, so nothing would use it.
#
# Without -Config, BRINDLE_CONFIG is a fresh config.toml in that directory,
# removed with it. -Config FILE points BRINDLE_CONFIG at FILE itself (its
# absolute path): edits to it apply live, the settings dialog writes it, and
# the cleanup leaves it. Pass a copy to keep an original untouched. A profile
# in it that runs tmux attaches to the tmux server it names (a WSL one is
# yours unless the profile's args name another socket with `-L`). Check it
# first.
#
# The binary is $env:BRINDLE_BIN, else target\release\brindle.exe, else
# target\debug\brindle.exe. BRINDLE_SERVER_HELPER is set to the
# brindle-remote.exe beside it when there is one. A helper built in target\
# without the packaged ConPTY stops the run: the script fetches it
# (scripts/fetch-conpty.ps1) and asks for a rebuild.
#
# SSH and container profiles upload the static Linux helper. A
# BRINDLE_REMOTE_AGENT you set is used as it is, and so is an unpacked
# release's brindle-remote-linux-x86_64 beside the binary. For a binary in
# this checkout's target\, scripts/linux-helper.ps1 builds the helper in
# Docker (Linux containers) when it is missing or older than its sources, or
# isn't the binary's release, and the run sets BRINDLE_REMOTE_AGENT to it.
# Without Docker it warns that remote profiles won't work and runs anyway; a
# failed build stops the run. -NoLinuxHelper skips all of this.
#
# This opens a window on your desktop and takes focus: don't start it while
# typing elsewhere. brindle.exe is a GUI program, which PowerShell does not
# wait for by itself, so it is started with Start-Process and waited on.

param(
    [string]$Config,
    [switch]$Keep,
    [switch]$NoLinuxHelper,
    [switch]$Help,
    # Quote each one: PowerShell reads a bare -d as one of its own parameters.
    [string[]]$BrindleArgs = @()
)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message, [int]$Code) {
    [Console]::Error.WriteLine("run-throwaway: $Message")
    exit $Code
}

if ($Help) {
    Get-Content -Path $PSCommandPath |
        Where-Object { $_ -match '^#' } |
        ForEach-Object { $_ -replace '^# ?', '' } |
        Select-Object -First 47
    exit 0
}

$passed = @($BrindleArgs)

$root = Split-Path -Parent $PSScriptRoot
$bin = $env:BRINDLE_BIN
if (-not $bin) {
    foreach ($candidate in @('target\release\brindle.exe', 'target\debug\brindle.exe')) {
        $path = Join-Path $root $candidate
        if (Test-Path -LiteralPath $path) { $bin = $path; break }
    }
}
if (-not $bin -or -not (Test-Path -LiteralPath $bin)) {
    Fail 'no brindle.exe (cargo build --release, or set BRINDLE_BIN)' 1
}
$bin = (Resolve-Path -LiteralPath $bin).ProviderPath
if ($Config) {
    if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) { Fail "$Config is not a file" 1 }
    # Absolute, since Brindle's working directory can differ from this
    # shell's location.
    $Config = (Resolve-Path -LiteralPath $Config).ProviderPath
}

$binDir = Split-Path -Parent $bin
$helper = Join-Path $binDir 'brindle-remote.exe'
$haveHelper = Test-Path -LiteralPath $helper

# A helper built here without the packaged ConPTY (build.rs copies it beside
# the exe when it embeds it) runs local tabs on the ConPTY built into
# Windows, which repaints output and drops the OSC replies Brindle writes to
# programs: not what a release does. Fetch it and stop, since only a rebuild
# embeds it.
$targetDir = (Join-Path $root 'target') + [System.IO.Path]::DirectorySeparatorChar
$built = ($binDir + [System.IO.Path]::DirectorySeparatorChar).StartsWith($targetDir, [StringComparison]::OrdinalIgnoreCase)
if ($haveHelper -and $built -and -not (Test-Path -LiteralPath (Join-Path $binDir 'conpty.dll'))) {
    & (Join-Path $PSScriptRoot 'fetch-conpty.ps1')
    Fail "$helper was built without the packaged ConPTY, which is now fetched: build again (cargo build), then rerun" 1
}

# The Linux helper remote profiles upload, before the window opens, so a
# build's output is in this console and a failure stops the run first.
$remoteAgent = $null
if ($NoLinuxHelper) {
    Write-Host 'run-throwaway: -NoLinuxHelper: remote profiles need BRINDLE_REMOTE_AGENT'
} elseif ($env:BRINDLE_REMOTE_AGENT) {
    Write-Host "run-throwaway: Linux helper $env:BRINDLE_REMOTE_AGENT (BRINDLE_REMOTE_AGENT)"
} elseif (Test-Path -LiteralPath (Join-Path $binDir 'brindle-remote-linux-x86_64') -PathType Leaf) {
    # An unpacked release: Brindle finds its release's own helper beside it.
} elseif (-not $built) {
    Write-Host "run-throwaway: remote profiles need brindle-remote-linux-x86_64 beside $bin, or BRINDLE_REMOTE_AGENT"
} else {
    $linux = & (Join-Path $PSScriptRoot 'linux-helper.ps1') -Root $root -Bin $bin -Build Auto
    switch ($linux.State) {
        'Ready' {
            $remoteAgent = $linux.Path
            Write-Host "run-throwaway: Linux helper $remoteAgent"
        }
        'Unchecked' {
            $remoteAgent = $linux.Path
            Write-Warning "run-throwaway: $($linux.Message)"
        }
        'Unavailable' {
            Write-Warning "run-throwaway: remote profiles won't work: $($linux.Message), or set BRINDLE_REMOTE_AGENT"
        }
        'Failed' { Fail $linux.Message 1 }
    }
}

$dir = Join-Path ([System.IO.Path]::GetTempPath()) "brindle-throwaway-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
New-Item -ItemType Directory -Path (Join-Path $dir 'state') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $dir 'server') -Force | Out-Null
$configPath = if ($Config) { $Config } else { Join-Path $dir 'config.toml' }

# One argument for Start-Process, which joins them with spaces: quote what
# has whitespace or a quote in it, as the MSVC runtime reads it back.
function Quote-Argument([string]$Argument) {
    if ($Argument -ne '' -and $Argument -notmatch '[\s"]') { return $Argument }
    $escaped = $Argument -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1'
    return '"' + $escaped + '"'
}

$previous = @{
    Config = $env:BRINDLE_CONFIG
    State  = $env:BRINDLE_STATE_DIR
    Server = $env:BRINDLE_SERVER_DIR
    Cache  = $env:BRINDLE_CACHE_DIR
    Helper = $env:BRINDLE_SERVER_HELPER
    Remote = $env:BRINDLE_REMOTE_AGENT
}
$exit = 0
try {
    Write-Host "run-throwaway: $bin (state in $dir)"
    if ($Config) {
        Write-Host "run-throwaway: config $Config (edits apply live; the settings dialog writes it)"
    }
    $env:BRINDLE_CONFIG = $configPath
    $env:BRINDLE_STATE_DIR = Join-Path $dir 'state'
    $env:BRINDLE_SERVER_DIR = Join-Path $dir 'server'
    $env:BRINDLE_CACHE_DIR = Join-Path $dir 'cache'
    if ($haveHelper) { $env:BRINDLE_SERVER_HELPER = $helper }
    if ($remoteAgent) { $env:BRINDLE_REMOTE_AGENT = $remoteAgent }
    $start = @{ FilePath = $bin; PassThru = $true }
    if ($passed.Count -gt 0) {
        $start.ArgumentList = ($passed | ForEach-Object { Quote-Argument $_ }) -join ' '
    }
    $process = Start-Process @start
    # Not -Wait: that waits for the whole process tree, including a terminal
    # server that outlives Brindle by design.
    $process.WaitForExit()
    $exit = $process.ExitCode
} finally {
    $env:BRINDLE_CONFIG = $previous.Config
    $env:BRINDLE_STATE_DIR = $previous.State
    if ($haveHelper) {
        # Stop this directory's server, kept or not; never one of yours.
        try {
            $env:BRINDLE_SERVER_DIR = Join-Path $dir 'server'
            & $helper stop *> $null
        } catch {}
    }
    $env:BRINDLE_SERVER_DIR = $previous.Server
    $env:BRINDLE_CACHE_DIR = $previous.Cache
    $env:BRINDLE_SERVER_HELPER = $previous.Helper
    $env:BRINDLE_REMOTE_AGENT = $previous.Remote
    if ($Keep) {
        Write-Host "run-throwaway: kept $dir"
    } else {
        # The server's per-build helper copy stays locked until its processes
        # have gone, which `stop` doesn't wait for: retry for a few seconds.
        for ($try = 0; $try -lt 20 -and (Test-Path -LiteralPath $dir); $try++) {
            Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $dir) { Start-Sleep -Milliseconds 250 }
        }
    }
}
exit $exit
