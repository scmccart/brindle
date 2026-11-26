# Provides the static x86-64 Linux helper (brindle-remote) that SSH and
# container profiles upload, for a Windows build from this checkout.
# scripts/install.ps1 and scripts/run-throwaway.ps1 both run it:
#
#   scripts/linux-helper.ps1 -Root . -Bin target\release\brindle.exe -Check
#   scripts/linux-helper.ps1 -Root . -Bin target\release\brindle.exe -Build Auto
#   scripts/linux-helper.ps1 -Root . -Bin target\release\brindle.exe -Helper PATH
#   scripts/linux-helper.ps1 -ProbeDocker
#
# The helper is right when all of these hold:
#   1. it is a Linux ELF;
#   2. it is no older than its sources (remote-agent/src, remote-protocol/src,
#      term-core/src, transcript/src), remote-agent/Cargo.toml or Cargo.lock;
#   3. its --version, run in Docker, is the release -Bin prints
#      (`brindle X.Y.Z` for a helper that prints X.Y.Z).
#
# It is built in Docker (Linux containers) with the Rust version in
# rust-toolchain.toml, into target/linux-helper (apart from the Windows
# build's artifacts), keeping downloads in the brindle-cargo-registry volume.
#
# -Build Auto builds it when it is missing or breaks rule 2, and once more when
# a helper that keeps rule 2 breaks rule 1 or 3; -Build Never builds nothing;
# -Build Always builds it every time (install.ps1). -Helper PATH checks that
# file by rules 1 and 3 only, building nothing. -Check prints what it found and
# what a run with -Build would do, building nothing. -ProbeDocker only says
# whether Docker can build and run the helper.
#
# Output is one object, @{ Path; State; Message }, State one of:
#   Ready        the helper at Path keeps rules 1-3 (for -ProbeDocker: Docker
#                is usable, and Path is empty);
#   Unchecked    the helper at Path keeps rules 1 and 2, but Docker can't run
#                it to check rule 3: usable, with Message as a warning;
#   Unavailable  there is no right helper and none can be built (Message says
#                why); Path is empty;
#   Failed       a build failed, or a built or named helper breaks rule 1 or
#                3 (Message says which): stop.

param(
    [string]$Root = (Split-Path -Parent $PSScriptRoot),
    [string]$Bin,
    [ValidateSet('Auto', 'Never', 'Always')]
    [string]$Build = 'Auto',
    [string]$Helper,
    [switch]$Check,
    [switch]$ProbeDocker
)

$ErrorActionPreference = 'Stop'

# The helper's inputs for rule 2. The four directories are the ones
# tests/common/server.rs `check_helper` names, and scripts/run-throwaway.sh
# checks the same list: change the three together. The manifest and the
# lockfile catch a version bump or a dependency change that touches no source.
$sourceDirs = @('remote-agent/src', 'remote-protocol/src', 'term-core/src', 'transcript/src')
$sourceFiles = @('remote-agent/Cargo.toml', 'Cargo.lock')

$target = 'target/linux-helper'

function Result([string]$State, [string]$Path, [string]$Message) {
    [pscustomobject]@{ Path = $Path; State = $State; Message = $Message }
}

# Why Docker can't build or run the helper, or $null when it can.
function Test-Docker {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        return 'docker not found; install Docker Desktop'
    }
    $ErrorActionPreference = 'Continue'
    $os = docker info --format '{{.OSType}}' 2>$null
    if ($LASTEXITCODE -ne 0) { return 'the Docker daemon is not running; start Docker Desktop' }
    $os = "$os".Trim()
    if ($os -ne 'linux') { return "Docker runs $os containers; switch Docker Desktop to Linux containers" }
    return $null
}

function Get-Rust {
    $toolchain = Join-Path $Root 'rust-toolchain.toml'
    (Select-String -Path $toolchain -Pattern '^channel = "(.+)"').Matches[0].Groups[1].Value
}

# The release -Bin is (its --version minus `brindle `), as the e2e harness's
# `same_release` reads it. Read from the binary, so a stale brindle.exe shows
# up as a mismatch.
function Get-Release {
    if (-not $Bin) { throw '-Bin is required' }
    $ErrorActionPreference = 'Continue'
    # brindle.exe is a GUI program: PowerShell waits for it only when its
    # output is piped.
    $said = (& $Bin --version 2>$null | Out-String).Trim()
    if ($LASTEXITCODE -or -not $said.StartsWith('brindle ') -or $said.Length -le 8) {
        throw "$Bin prints '$said' for --version, not brindle X.Y.Z"
    }
    return $said.Substring(8)
}

# Rule 1.
function Test-Elf([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $head = New-Object byte[] 4
        $read = $stream.Read($head, 0, 4)
    } finally {
        $stream.Dispose()
    }
    return $read -eq 4 -and $head[0] -eq 0x7f -and $head[1] -eq 0x45 -and $head[2] -eq 0x4c -and $head[3] -eq 0x46
}

# Rule 2: the first input newer than the helper, relative to the root, or
# $null when there is none.
function Find-Newer([string]$Path) {
    $built = (Get-Item -LiteralPath $Path).LastWriteTimeUtc
    $inputs = @(foreach ($dir in $sourceDirs) { Get-ChildItem -LiteralPath (Join-Path $Root $dir) -Recurse -File }) +
        @(foreach ($file in $sourceFiles) { Get-Item -LiteralPath (Join-Path $Root $file) })
    $newer = $inputs | Where-Object { $_.LastWriteTimeUtc -gt $built } | Select-Object -First 1
    if ($newer) { return $newer.FullName.Substring($Root.Length).TrimStart('\', '/') }
    return $null
}

# Rules 1 and 3: why the helper isn't right, or $null when it is.
function Test-Helper([string]$Path) {
    if (-not (Test-Elf $Path)) { return "$Path is not a Linux ELF binary" }
    $release = Get-Release
    $dir = Split-Path -Parent $Path
    $leaf = Split-Path -Leaf $Path
    $ErrorActionPreference = 'Continue'
    $said = (docker run --rm -v "${dir}:/helper:ro" "rust:$(Get-Rust)" "/helper/$leaf" --version | Out-String).Trim()
    if ($LASTEXITCODE -or $said -ne $release) {
        return "$Path prints '$said' for --version, not $release"
    }
    return $null
}

# Builds the helper, saying why first; $true when the build succeeded.
function Invoke-Build([string]$Why) {
    Write-Host "linux-helper: building the Linux helper in Docker ($Why); a first build takes several minutes"
    $mounts = @('-v', "${Root}:/src", '-v', 'brindle-cargo-registry:/usr/local/cargo/registry', '-w', '/src')
    $build = "apt-get update -qq && apt-get install -y -qq musl-tools >/dev/null && " +
        "rustup target add x86_64-unknown-linux-musl && " +
        "CARGO_TARGET_DIR=$target cargo build --locked --release -p brindle-remote-agent --target x86_64-unknown-linux-musl"
    $ErrorActionPreference = 'Continue'
    # Out-Host keeps the build's output off this script's output, which is
    # the result alone.
    docker run --rm @mounts "rust:$(Get-Rust)" sh -c $build | Out-Host
    return $LASTEXITCODE -eq 0
}

# With -Check, where a run would build: says so instead.
function Skip-Build([string]$Path, [string]$Why) {
    Result 'WouldBuild' $Path "$Why; a run with -Build $Build builds it"
}

# Builds, then checks rules 1 and 3.
function Build-Checked([string]$Path, [string]$Why) {
    if ($Check) { return Skip-Build $Path $Why }
    if (-not (Invoke-Build $Why)) { return Result 'Failed' '' 'building the Linux helper in Docker failed' }
    $wrong = Test-Helper $Path
    if ($wrong) { return Result 'Failed' '' $wrong }
    return Result 'Ready' $Path "$Path is built from this checkout"
}

function Resolve-Helper {
    if ($ProbeDocker) {
        $noDocker = Test-Docker
        if ($noDocker) { return Result 'Unavailable' '' $noDocker }
        return Result 'Ready' '' 'Docker runs Linux containers'
    }

    if ($Helper) {
        $path = (Resolve-Path -LiteralPath $Helper).ProviderPath
        if (-not (Test-Elf $path)) { return Result 'Failed' '' "$path is not a Linux ELF binary" }
        if (Test-Docker) {
            return Result 'Unchecked' $path "can't run $path to check its version; it must be $(Get-Release)"
        }
        $wrong = Test-Helper $path
        if ($wrong) { return Result 'Failed' '' $wrong }
        return Result 'Ready' $path "$path is this release's"
    }

    $path = [System.IO.Path]::Combine($Root, 'target', 'linux-helper', 'x86_64-unknown-linux-musl', 'release', 'brindle-remote')
    $noDocker = Test-Docker
    $stale = if ($Build -eq 'Always') {
        'install.ps1 builds it every time'
    } elseif (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        "$path is missing"
    } else {
        $newer = Find-Newer $path
        if ($newer) { "$path is older than $newer" } else { $null }
    }

    if ($stale) {
        # A stale helper isn't used without Docker: it would upload old code.
        if ($noDocker) { return Result 'Unavailable' '' "$stale, and $noDocker" }
        if ($Build -eq 'Never') { return Result 'Unavailable' '' "$stale; -Build Auto builds it" }
        return Build-Checked $path $stale
    }

    if ($noDocker) {
        if (-not (Test-Elf $path)) { return Result 'Unavailable' '' "$path is not a Linux ELF binary, and $noDocker" }
        return Result 'Unchecked' $path "can't run $path to check its version ($noDocker); it must be $(Get-Release)"
    }
    $wrong = Test-Helper $path
    if (-not $wrong) { return Result 'Ready' $path "$path is built from this checkout" }
    if ($Build -eq 'Never') { return Result 'Failed' '' $wrong }
    # Fresh by its time but wrong (say, built before a checkout was switched
    # back): build once, then a second failure stops.
    return Build-Checked $path $wrong
}

$Root = (Resolve-Path -LiteralPath $Root).ProviderPath.TrimEnd('\', '/')
if ($Bin) { $Bin = (Resolve-Path -LiteralPath $Bin).ProviderPath }

try {
    $result = Resolve-Helper
} catch {
    $result = Result 'Failed' '' "$_"
}
if ($Check) {
    $what = switch ($result.State) {
        'Ready' { 'ready' }
        'Unchecked' { 'usable, version unchecked' }
        'Unavailable' { 'unavailable' }
        'Failed' { 'failed' }
        'WouldBuild' { 'would build' }
    }
    Write-Host "linux-helper: ${what}: $($result.Message)"
} else {
    $result
}
