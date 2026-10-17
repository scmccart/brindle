# Fetches Microsoft's packaged ConPTY (conpty.dll and OpenConsole.exe, the
# console host Windows Terminal ships; MIT) into .conpty/<version>/, where
# build.rs embeds it in brindle.exe. Without it Brindle falls back to the
# ConPTY built into Windows, which repaints output instead of passing it
# through.
#
# The version and the package's sha256 are pinned here and moved by hand.
# build.rs reads $Version from this file.
#
#   scripts/fetch-conpty.ps1             # fetch the pinned version
#   scripts/fetch-conpty.ps1 -Sha256 X   # expect another hash (for testing)

param(
    [string]$Sha256 = '02B07B349AF66D801159BDF9E440D4A1CE78BB951F37FC8609731665AFDAE7EE'
)

$ErrorActionPreference = 'Stop'
$Version = '1.25.260930003'

$dest = Join-Path (Split-Path -Parent $PSScriptRoot) ".conpty\$Version"
if ((Test-Path (Join-Path $dest 'conpty.dll')) -and (Test-Path (Join-Path $dest 'OpenConsole.exe'))) {
    Write-Output "fetch-conpty: $dest is already there"
    exit 0
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) "brindle-conpty-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $package = Join-Path $work 'conpty.nupkg'
    $url = "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/$Version/microsoft.windows.console.conpty.$Version.nupkg"
    Invoke-WebRequest -Uri $url -OutFile $package
    $actual = (Get-FileHash -Algorithm SHA256 -Path $package).Hash
    if ($actual -ne $Sha256.ToUpperInvariant()) {
        throw "fetch-conpty: $url has sha256 $actual, expected $Sha256"
    }
    $unpacked = Join-Path $work 'package'
    Expand-Archive -Path $package -DestinationPath $unpacked
    New-Item -ItemType Directory -Force -Path $dest | Out-Null
    Copy-Item (Join-Path $unpacked 'runtimes\win-x64\native\conpty.dll') $dest
    Copy-Item (Join-Path $unpacked 'build\native\runtimes\x64\OpenConsole.exe') $dest
    Write-Output "fetch-conpty: ConPTY $Version is in $dest"
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
