# Builds Brindle, its terminal server helper and the Linux remote helper, then
# installs them together with a Start Menu shortcut, so SSH and container
# profiles upload a helper that matches this build.
#
#   scripts/install.ps1                          # into %LOCALAPPDATA%\Programs\brindle
#   scripts/install.ps1 -Prefix D:\Tools\brindle
#   scripts/install.ps1 -RemoteAgent PATH        # use this Linux helper instead of building it
#
# The Linux helper is built in Docker (Linux containers) from this checkout,
# with the Rust version in rust-toolchain.toml. Re-run it to update; a Brindle
# that is running keeps its old build until it restarts. The install directory
# is added to your user PATH. scripts/uninstall.ps1 removes it all again.

param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\brindle'),
    [string]$RemoteAgent
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $rust = (Select-String -Path rust-toolchain.toml -Pattern '^channel = "(.+)"').Matches[0].Groups[1].Value

    # Fail before the long builds if the Linux helper can't be had.
    if ($RemoteAgent) {
        $RemoteAgent = (Resolve-Path $RemoteAgent).Path
    } else {
        if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
            throw "install: docker not found; install Docker Desktop, or pass -RemoteAgent with a Linux brindle-remote $version"
        }
        $os = docker info --format '{{.OSType}}' 2>$null
        if ($LASTEXITCODE -ne 0) { throw 'install: the Docker daemon is not running; start Docker Desktop' }
        if ($os -ne 'linux') { throw "install: Docker runs $os containers; switch Docker Desktop to Linux containers" }
    }

    & "$PSScriptRoot\fetch-conpty.ps1"
    if ($LASTEXITCODE) { throw 'install: fetch-conpty.ps1 failed' }
    cargo build --release --locked -p brindle -p brindle-remote-agent
    if ($LASTEXITCODE) { throw 'install: cargo build failed' }

    if (-not $RemoteAgent) {
        # A target directory of its own keeps the container's host artifacts
        # apart from the Windows build's; the registry volume keeps downloads.
        $target = 'target/linux-helper'
        $mounts = @('-v', "${root}:/src", '-v', 'brindle-cargo-registry:/usr/local/cargo/registry', '-w', '/src')
        $build = "apt-get update -qq && apt-get install -y -qq musl-tools >/dev/null && " +
            "rustup target add x86_64-unknown-linux-musl && " +
            "CARGO_TARGET_DIR=$target cargo build --locked --release -p brindle-remote-agent --target x86_64-unknown-linux-musl"
        docker run --rm @mounts "rust:$rust" sh -c $build
        if ($LASTEXITCODE) { throw 'install: building the Linux helper in Docker failed' }
        $RemoteAgent = Join-Path $root "$target\x86_64-unknown-linux-musl\release\brindle-remote"
    }

    # The helper must be a Linux binary of this version, or remote profiles
    # upload it and then refuse it on the host.
    $bytes = [System.IO.File]::ReadAllBytes($RemoteAgent)
    if ($bytes.Length -lt 4 -or $bytes[0] -ne 0x7f -or $bytes[1] -ne 0x45 -or $bytes[2] -ne 0x4c -or $bytes[3] -ne 0x46) {
        throw "install: $RemoteAgent is not a Linux ELF binary"
    }
    if (Get-Command docker -ErrorAction SilentlyContinue) {
        $dir = Split-Path -Parent $RemoteAgent
        $said = docker run --rm -v "${dir}:/helper:ro" "rust:$rust" /helper/brindle-remote --version 2>&1
        if ($LASTEXITCODE -or "$said".Trim() -ne $version) {
            throw "install: $RemoteAgent prints '$said' for --version, not $version"
        }
    } else {
        Write-Warning "install: can't run $RemoteAgent to check its version; it must be $version"
    }

    New-Item -ItemType Directory -Force $Prefix | Out-Null
    # Windows locks a running exe, but lets it be renamed: move the old one
    # aside so a running Brindle doesn't block the update.
    function Install-File([string]$From, [string]$Name) {
        $to = Join-Path $Prefix $Name
        $old = "$to.old"
        Remove-Item $old -Force -ErrorAction SilentlyContinue
        if (Test-Path $to) { Move-Item $to $old -Force }
        Copy-Item $From $to
        Remove-Item $old -Force -ErrorAction SilentlyContinue
    }
    Install-File 'target\release\brindle.exe' 'brindle.exe'
    Install-File 'target\release\brindle-remote.exe' 'brindle-remote.exe'
    Install-File $RemoteAgent 'brindle-remote-linux-x86_64'

    # Run the installed copy, so the shortcut points at it.
    & (Join-Path $Prefix 'brindle.exe') --install-desktop
    if ($LASTEXITCODE) { throw 'install: brindle --install-desktop failed' }

    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($path -split ';' | Where-Object { $_ })
    if ($entries -notcontains $Prefix) {
        [Environment]::SetEnvironmentVariable('Path', (($entries + $Prefix) -join ';'), 'User')
        Write-Output "added $Prefix to your user PATH (new terminals pick it up)"
    }
    Write-Output "installed Brindle $version in $Prefix"
} finally {
    Pop-Location
}
