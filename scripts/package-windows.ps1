# Packs the release build into the Windows release zip,
# target/dist/brindle-<version>-x86_64-pc-windows-msvc.zip: brindle.exe,
# Brindle's licences and the notice for the ConPTY that brindle.exe embeds.
# The exe needs nothing beside it, so this is the whole release.
#
#   scripts/fetch-conpty.ps1
#   cargo build --release --locked
#   scripts/package-windows.ps1
#
# It refuses an exe that doesn't embed the pinned ConPTY, which a build run
# before scripts/fetch-conpty.ps1 would be.

param(
    [string]$Exe = 'target\release\brindle.exe'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $conpty = (Select-String -Path scripts\fetch-conpty.ps1 -Pattern "^\`$Version = '(.+)'").Matches[0].Groups[1].Value
    $fetched = ".conpty\$conpty"

    # build.rs embeds both files byte for byte, so look for them in the exe.
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
    $exeBytes = [System.IO.File]::ReadAllBytes((Resolve-Path $Exe))
    foreach ($name in 'conpty.dll', 'OpenConsole.exe') {
        $file = Join-Path $fetched $name
        if (-not (Test-Path $file)) { throw "package-windows: no $file; run scripts/fetch-conpty.ps1, then rebuild" }
        if (-not [Embedded]::Contains($exeBytes, [System.IO.File]::ReadAllBytes((Resolve-Path $file)))) {
            throw "package-windows: $Exe doesn't embed $file; rebuild it after scripts/fetch-conpty.ps1"
        }
    }

    $name = "brindle-$version-x86_64-pc-windows-msvc"
    $dist = 'target\dist'
    $stage = Join-Path $dist $name
    Remove-Item -Recurse -Force $stage, "$stage.zip" -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item $Exe, LICENSE-MIT, LICENSE-APACHE, packaging\windows\THIRD-PARTY-NOTICES.txt $stage
    Compress-Archive -Path "$stage\*" -DestinationPath "$stage.zip"
    Remove-Item -Recurse -Force $stage
    Write-Output (Resolve-Path "$stage.zip").Path
} finally {
    Pop-Location
}
