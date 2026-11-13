# Runs Brindle with throwaway state, for trying a build by hand without
# touching your real config or saved session (the Windows twin of
# run-throwaway.sh):
#
#   scripts/run-throwaway.ps1                      # a default config
#   scripts/run-throwaway.ps1 -Config FILE         # start from a copy of FILE
#   scripts/run-throwaway.ps1 -Keep                # keep the directory afterwards
#   scripts/run-throwaway.ps1 -BrindleArgs '-d','C:/Temp'   # arguments for brindle
#
# It makes a private directory under %TEMP% and points BRINDLE_CONFIG,
# BRINDLE_STATE_DIR and BRINDLE_SERVER_DIR into it. So the settings dialog
# writes the throwaway config, the launch doesn't hand itself to a Brindle
# you already have running (the single-instance lock and its named pipe are
# keyed by the state directory), and the sessions run in their own terminal
# server, which is stopped (and the directory removed) when Brindle exits;
# -Keep leaves both, so stop it yourself with brindle-remote stop.
#
# The binary is $env:BRINDLE_BIN, else target\release\brindle.exe, else
# target\debug\brindle.exe. BRINDLE_SERVER_HELPER is set to the
# brindle-remote.exe beside it when there is one.
#
# -Config copies a file in, so a copied config's tmux profiles would attach
# to the tmux server they name (a WSL one is yours unless the profile's
# `tmux_args` names another socket with `-L`). Check it first.
#
# This opens a window on your desktop and takes focus: don't start it while
# typing elsewhere. brindle.exe is a GUI program, which PowerShell does not
# wait for by itself, so it is started with Start-Process and waited on.

param(
    [string]$Config,
    [switch]$Keep,
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
        Select-Object -First 29
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
if ($Config -and -not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    Fail "$Config is not a file" 1
}

$helper = Join-Path (Split-Path -Parent $bin) 'brindle-remote.exe'
$haveHelper = Test-Path -LiteralPath $helper

$dir = Join-Path ([System.IO.Path]::GetTempPath()) "brindle-throwaway-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
New-Item -ItemType Directory -Path (Join-Path $dir 'state') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $dir 'server') -Force | Out-Null
if ($Config) {
    Copy-Item -LiteralPath $Config -Destination (Join-Path $dir 'config.toml')
}

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
    Helper = $env:BRINDLE_SERVER_HELPER
}
$exit = 0
try {
    Write-Host "run-throwaway: $bin (state in $dir)"
    $env:BRINDLE_CONFIG = Join-Path $dir 'config.toml'
    $env:BRINDLE_STATE_DIR = Join-Path $dir 'state'
    $env:BRINDLE_SERVER_DIR = Join-Path $dir 'server'
    if ($haveHelper) { $env:BRINDLE_SERVER_HELPER = $helper }
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
    if ($haveHelper -and -not $Keep) {
        # Stop this directory's server before it is removed; never one of yours.
        try {
            $env:BRINDLE_SERVER_DIR = Join-Path $dir 'server'
            & $helper stop *> $null
        } catch {}
    }
    $env:BRINDLE_SERVER_DIR = $previous.Server
    $env:BRINDLE_SERVER_HELPER = $previous.Helper
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
