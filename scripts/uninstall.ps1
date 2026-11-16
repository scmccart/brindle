# Removes what install.ps1 installed, for the same -Prefix: the Start Menu
# shortcut, the three binaries and the PATH entry. Your config in
# %APPDATA%\brindle and session state in %LOCALAPPDATA%\brindle are kept.
#
#   scripts/uninstall.ps1
#   scripts/uninstall.ps1 -Prefix D:\Tools\brindle

param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'Programs\brindle')
)

$ErrorActionPreference = 'Stop'
$exe = Join-Path $Prefix 'brindle.exe'
if (Test-Path $exe) {
    & $exe --uninstall-desktop
    if ($LASTEXITCODE) { throw 'uninstall: brindle --uninstall-desktop failed' }
} else {
    # Without the binary, remove the shortcut it would have (see src/desktop.rs).
    Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'Brindle.lnk') -Force -ErrorAction SilentlyContinue
}

foreach ($name in 'brindle.exe', 'brindle-remote.exe', 'brindle-remote-linux-x86_64') {
    foreach ($file in (Join-Path $Prefix $name), (Join-Path $Prefix "$name.old")) {
        if (Test-Path $file) { Remove-Item $file -Force }
    }
}
if ((Test-Path $Prefix) -and -not (Get-ChildItem $Prefix -Force)) { Remove-Item $Prefix }

$path = [Environment]::GetEnvironmentVariable('Path', 'User')
$entries = @($path -split ';' | Where-Object { $_ })
if ($entries -contains $Prefix) {
    [Environment]::SetEnvironmentVariable('Path', (($entries | Where-Object { $_ -ne $Prefix }) -join ';'), 'User')
    Write-Output "removed $Prefix from your user PATH"
}
Write-Output "uninstalled Brindle from $Prefix"
