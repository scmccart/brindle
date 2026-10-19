# Writes the WinGet manifest for a release zip from scripts/package-windows.ps1
# into target/dist/winget/<version>/: the templates in packaging/winget/ with
# the version, the zip's download URL and its sha256 filled in. Check the
# result with `winget validate --manifest <dir>`.
#
#   scripts/winget-manifest.ps1 -Zip target/dist/brindle-0.1.0-x86_64-pc-windows-msvc.zip
#   scripts/winget-manifest.ps1 -Zip <zip> -Url http://localhost:8000/<zip name>   # a local test
#
# Without -Url, the URL is the zip's asset on the GitHub Release tagged
# v<version>, which is where releases publish it.

param(
    [Parameter(Mandatory)] [string]$Zip,
    [string]$Url
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$zipPath = (Resolve-Path $Zip).Path
$name = Split-Path -Leaf $zipPath
if ($name -notmatch '^brindle-(.+)-x86_64-pc-windows-msvc\.zip$') {
    throw "winget-manifest: $name isn't a zip from scripts/package-windows.ps1"
}
$version = $Matches[1]
if (-not $Url) { $Url = "https://github.com/scmccart/brindle/releases/download/v$version/$name" }
$sha256 = (Get-FileHash -Algorithm SHA256 -Path $zipPath).Hash

$out = Join-Path $root "target\dist\winget\$version"
New-Item -ItemType Directory -Force $out | Out-Null
foreach ($template in Get-ChildItem (Join-Path $root 'packaging\winget') -Filter '*.yaml') {
    $text = (Get-Content -Raw $template.FullName).Replace('{{version}}', $version).Replace('{{url}}', $Url).Replace('{{sha256}}', $sha256)
    # WinGet reads UTF-8; keep the templates' LF endings.
    [System.IO.File]::WriteAllText((Join-Path $out $template.Name), $text, [System.Text.UTF8Encoding]::new($false))
}
Write-Output $out
