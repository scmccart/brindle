# Packs the release build into the Windows release zip,
# target/dist/brindle-<version>-x86_64-pc-windows-msvc.zip: brindle.exe, the
# terminal server helper brindle-remote.exe (which must sit beside it), the
# Linux x86-64 remote helper, Brindle's licences and the notice for the
# packaged ConPTY.
#
#   scripts/fetch-conpty.ps1
#   cargo build --release --locked
#   scripts/package-windows.ps1
#
# It refuses a build whose terminal server helper, brindle-remote.exe,
# doesn't embed the pinned ConPTY, which a build run before
# scripts/fetch-conpty.ps1 would be.

param(
    [string]$Exe = 'target\release\brindle.exe',
    [string]$Helper = 'target\release\brindle-remote.exe',
    [string]$RemoteAgent = 'target\release\brindle-remote-linux-x86_64'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $conpty = (Select-String -Path scripts\fetch-conpty.ps1 -Pattern "^\`$Version = '(.+)'").Matches[0].Groups[1].Value
    $fetched = ".conpty\$conpty"

    # remote-agent/build.rs embeds both files byte for byte, so look for them
    # in the helper. brindle.exe embeds none, so passing it as the helper fails.
    if (-not $Helper -or -not (Test-Path -PathType Leaf $Helper)) {
        throw "package-windows: no terminal server helper '$Helper'; build brindle-remote.exe (cargo build --release) or pass -Helper"
    }
    Add-Type -TypeDefinition @'
public static class Embedded {
    public static bool Contains(byte[] haystack, byte[] needle) {
        for (int i = 0; i + needle.Length <= haystack.Length; i++) {
            int j = 0;
            while (j < needle.Length && haystack[i + j] == needle[j]) j++;
            if (j == needle.Length) return true;
        }
        return false;
    }
}
'@
    $helperBytes = [System.IO.File]::ReadAllBytes((Resolve-Path $Helper))
    foreach ($name in 'conpty.dll', 'OpenConsole.exe') {
        $file = Join-Path $fetched $name
        if (-not (Test-Path $file)) { throw "package-windows: no $file; run scripts/fetch-conpty.ps1, then rebuild" }
        if (-not [Embedded]::Contains($helperBytes, [System.IO.File]::ReadAllBytes((Resolve-Path $file)))) {
            throw "package-windows: $Helper doesn't embed $file; rebuild it after scripts/fetch-conpty.ps1"
        }
    }

    if (-not (Test-Path $RemoteAgent)) {
        throw "package-windows: no $RemoteAgent; download or build the Linux remote helper"
    }
    $agentBytes = [System.IO.File]::ReadAllBytes((Resolve-Path $RemoteAgent))
    if ($agentBytes.Length -lt 4 -or $agentBytes[0] -ne 0x7f -or $agentBytes[1] -ne 0x45 -or $agentBytes[2] -ne 0x4c -or $agentBytes[3] -ne 0x46) {
        throw "package-windows: $RemoteAgent is not a Linux ELF binary"
    }
    $name = "brindle-$version-x86_64-pc-windows-msvc"
    $dist = 'target\dist'
    $stage = Join-Path $dist $name
    Remove-Item -Recurse -Force $stage, "$stage.zip" -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item $Exe, $RemoteAgent, LICENSE-MIT, LICENSE-APACHE, packaging\windows\THIRD-PARTY-NOTICES.txt $stage
    # Brindle looks for the helper by this name beside brindle.exe.
    Copy-Item $Helper (Join-Path $stage 'brindle-remote.exe')
    Compress-Archive -Path "$stage\*" -DestinationPath "$stage.zip"
    Remove-Item -Recurse -Force $stage
    Write-Output (Resolve-Path "$stage.zip").Path
} finally {
    Pop-Location
}
