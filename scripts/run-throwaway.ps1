# Runs Brindle with throwaway state, for trying a build by hand without
# touching your real config or saved session (the Windows twin of
# run-throwaway.sh):
#
#   scripts/run-throwaway.ps1                      # a default config
#   scripts/run-throwaway.ps1 -Config FILE         # start from a copy of FILE
#   scripts/run-throwaway.ps1 -Keep                # keep the directory afterwards
#   scripts/run-throwaway.ps1 -BrindleArgs '-d','C:/Temp'   # arguments for brindle
#
# It makes a private directory under %TEMP% and points BRINDLE_CONFIG and
# BRINDLE_STATE_DIR into it. So the settings dialog writes the throwaway
# config, and the launch doesn't hand itself to a Brindle you already have
# running (the single-instance lock and its named pipe are keyed by the
# state directory). The directory is removed when Brindle exits. Windows has
# no terminal server, so there is no BRINDLE_SERVER_DIR to set.
#
# The binary is $env:BRINDLE_BIN, else target\release\brindle.exe, else
# target\debug\brindle.exe.
#
# -Config copies a file in, so a copied config's profile that runs tmux would
# attach to the tmux server it names (a WSL one is yours unless the profile's
# args name another socket with `-L`). Check it first.
#
# This opens a window on your desktop and takes focus: don't start it while
# typing elsewhere. brindle.exe is a GUI program, which PowerShell does not
# wait for by itself, so it is started with Start-Process -Wait.

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
        Select-Object -First 26
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

$dir = Join-Path ([System.IO.Path]::GetTempPath()) "brindle-throwaway-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
New-Item -ItemType Directory -Path (Join-Path $dir 'state') -Force | Out-Null
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
}
$exit = 0
try {
    Write-Host "run-throwaway: $bin (state in $dir)"
    $env:BRINDLE_CONFIG = Join-Path $dir 'config.toml'
    $env:BRINDLE_STATE_DIR = Join-Path $dir 'state'
    $start = @{ FilePath = $bin; Wait = $true; PassThru = $true }
    if ($passed.Count -gt 0) {
        $start.ArgumentList = ($passed | ForEach-Object { Quote-Argument $_ }) -join ' '
    }
    $process = Start-Process @start
    $exit = $process.ExitCode
} finally {
    $env:BRINDLE_CONFIG = $previous.Config
    $env:BRINDLE_STATE_DIR = $previous.State
    if ($Keep) {
        Write-Host "run-throwaway: kept $dir"
    } else {
        Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
exit $exit
