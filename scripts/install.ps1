# Builds Brindle, its terminal server helper and the Linux remote helper, then
# installs them together with a Start Menu shortcut, so SSH and container
# profiles upload a helper that matches this build.
#
#   scripts/install.ps1                          # into %LOCALAPPDATA%\Programs\brindle
#   scripts/install.ps1 -Prefix D:\Tools\brindle
#   scripts/install.ps1 -RemoteAgent PATH        # use this Linux helper instead of building it
#
# The Linux helper is built in Docker (Linux containers) from this checkout,
# with the Rust version in rust-toolchain.toml, by scripts/linux-helper.ps1,
# which run-throwaway.ps1 shares. Re-run it to update; a Brindle
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
    # The Docker build and the checks are shared with run-throwaway.ps1.
    $linuxHelper = Join-Path $PSScriptRoot 'linux-helper.ps1'

    # Fail before the long builds if the Linux helper can't be had.
    if ($RemoteAgent) {
        $RemoteAgent = (Resolve-Path $RemoteAgent).Path
    } else {
        $docker = & $linuxHelper -ProbeDocker
        switch ($docker.State) {
            'Ready' {}
            'Unchecked' {}
            'Unavailable' { throw "install: $($docker.Message), or pass -RemoteAgent with a Linux brindle-remote $version" }
            'Failed' { throw "install: $($docker.Message)" }
        }
    }

    & "$PSScriptRoot\fetch-conpty.ps1"
    if ($LASTEXITCODE) { throw 'install: fetch-conpty.ps1 failed' }
    cargo build --release --locked -p brindle -p brindle-remote-agent
    if ($LASTEXITCODE) { throw 'install: cargo build failed' }

    # The helper must be a Linux binary of this build's release, or remote
    # profiles upload it and then refuse it on the host. A given one is
    # checked (rules 1 and 3 in linux-helper.ps1); otherwise it is built
    # every time, then checked.
    $bin = Join-Path $root 'target\release\brindle.exe'
    $linux = if ($RemoteAgent) {
        & $linuxHelper -Root $root -Bin $bin -Helper $RemoteAgent
    } else {
        & $linuxHelper -Root $root -Bin $bin -Build Always
    }
    switch ($linux.State) {
        'Ready' {}
        'Unchecked' { Write-Warning "install: $($linux.Message)" }
        'Unavailable' { throw "install: $($linux.Message)" }
        'Failed' { throw "install: $($linux.Message)" }
    }
    $RemoteAgent = $linux.Path

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

    # Run the installed copy, so the shortcut points at it. brindle.exe is a
    # GUI program, so PowerShell waits for it, and sets $LASTEXITCODE, only
    # when its output is piped.
    & (Join-Path $Prefix 'brindle.exe') --install-desktop | Write-Host
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
