# Installs a Brindle release on Windows: downloads the release's zip, checks
# it against the release's SHA256SUMS, installs brindle.exe, the terminal
# server helper brindle-remote.exe and the Linux remote helper in
# %LOCALAPPDATA%\Programs\brindle, adds that directory to your user PATH and
# creates the Start Menu shortcut. It needs no elevation. Each release
# publishes this script as install.ps1, stamped with its own version. In
# PowerShell 5.1 or 7:
#
#   irm https://github.com/scmccart/brindle/releases/latest/download/install.ps1 | iex
#   $env:BRINDLE_VERSION = '0.4.0'; irm .../install.ps1 | iex   # another release
#   $env:BRINDLE_UNINSTALL = 1; irm .../install.ps1 | iex       # remove it again
#   & ([scriptblock]::Create((irm .../install.ps1))) -Uninstall # the same, as a parameter
#
# BRINDLE_PREFIX (-Prefix) installs in another directory, and BRINDLE_VERSION
# (-Version) another release. A copy that was never stamped (this file in a
# checkout) installs the latest release. BRINDLE_DOWNLOAD_BASE replaces the
# release's download URL, for testing against a directory of built assets
# (file:///C:/DIR). Re-run it to update: a Brindle that is running keeps its
# old build until it quits. Uninstalling keeps your config in %APPDATA%\brindle
# and session state in %LOCALAPPDATA%\brindle.
#
# scripts/install.ps1 and scripts/uninstall.ps1 do the same from a checkout.
# They keep their own copies of these steps, since this script must run on its
# own. Its whole body is one script block, so a download cut off partway runs
# nothing, and nothing it sets stays in your session but the PATH entry. It
# reports a failure by throwing, never with exit, which would close the
# PowerShell that ran it through iex.

& {
    [CmdletBinding(PositionalBinding = $false)]
    param(
        [string]$Version = $env:BRINDLE_VERSION,
        [string]$Prefix = $env:BRINDLE_PREFIX,
        [switch]$Uninstall
    )

    Set-StrictMode -Version 2.0
    $ErrorActionPreference = 'Stop'
    # Windows PowerShell's progress bar slows downloads many times over.
    $ProgressPreference = 'SilentlyContinue'

    if ($PSVersionTable.PSVersion.Major -ge 6 -and -not $IsWindows) {
        throw 'install.ps1: this installs the Windows release; on Linux, use install.sh'
    }
    $remove = $Uninstall -or ($env:BRINDLE_UNINSTALL -and $env:BRINDLE_UNINSTALL -notin '0', 'false', 'no')
    if (-not $Prefix) { $Prefix = Join-Path $env:LOCALAPPDATA 'Programs\brindle' }
    $Prefix = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Prefix)
    $files = 'brindle.exe', 'brindle-remote.exe', 'brindle-remote-linux-x86_64'
    $exe = Join-Path $Prefix 'brindle.exe'

    # brindle.exe is a GUI program, so PowerShell waits for it, and sets
    # $LASTEXITCODE, only when its output is piped.
    function Invoke-Brindle([string[]]$Arguments) {
        & $exe @Arguments | Write-Host
        if ($LASTEXITCODE) { throw "install.ps1: brindle $Arguments failed (exit $LASTEXITCODE)" }
    }

    # The user PATH (for new terminals) and this session's.
    function Set-PathEntry([bool]$Present) {
        $path = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = @($path -split ';' | Where-Object { $_ })
        if ($Present -and $entries -notcontains $Prefix) {
            [Environment]::SetEnvironmentVariable('Path', (($entries + $Prefix) -join ';'), 'User')
            Write-Host "added $Prefix to your user PATH"
        } elseif (-not $Present -and $entries -contains $Prefix) {
            [Environment]::SetEnvironmentVariable('Path', (($entries | Where-Object { $_ -ne $Prefix }) -join ';'), 'User')
            Write-Host "removed $Prefix from your user PATH"
        }
        $session = @($env:Path -split ';' | Where-Object { $_ })
        if ($Present -and $session -notcontains $Prefix) {
            $env:Path = (($session + $Prefix) -join ';')
        } elseif (-not $Present -and $session -contains $Prefix) {
            $env:Path = (@($session | Where-Object { $_ -ne $Prefix }) -join ';')
        }
    }

    if ($remove) {
        # The steps of scripts/uninstall.ps1.
        if (Test-Path -LiteralPath $exe) {
            Invoke-Brindle '--uninstall-desktop'
        } else {
            # Without the binary, remove the shortcut it would have (see src/desktop.rs).
            Remove-Item (Join-Path ([Environment]::GetFolderPath('Programs')) 'Brindle.lnk') -Force -ErrorAction SilentlyContinue
        }
        foreach ($name in $files) {
            foreach ($file in (Join-Path $Prefix $name), (Join-Path $Prefix "$name.old")) {
                if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force }
            }
        }
        if ((Test-Path -LiteralPath $Prefix) -and -not (Get-ChildItem -LiteralPath $Prefix -Force)) {
            Remove-Item -LiteralPath $Prefix
        }
        Set-PathEntry $false
        Write-Host "uninstalled Brindle from $Prefix; your config in $env:APPDATA\brindle is kept"
        return
    }

    if ($PSVersionTable.PSVersion.Major -lt 6) {
        # Windows PowerShell may offer only TLS 1.0 and 1.1, which GitHub refuses.
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }

    # Downloads $Url to the file $To; a file:// URL is copied.
    function Get-Download([string]$Url, [string]$To) {
        try {
            $uri = [Uri]$Url
            if ($uri.IsFile) {
                Copy-Item -LiteralPath $uri.LocalPath -Destination $To
            } else {
                Invoke-WebRequest -UseBasicParsing -Uri $uri -OutFile $To
            }
        } catch {
            throw "install.ps1: can't download ${Url}: $($_.Exception.Message); nothing was installed"
        }
    }

    # The release stamps its version over this placeholder; a version never
    # holds an @, so one still here means the copy was never stamped.
    $stamped = '@VERSION@'
    if ($stamped.Contains('@')) { $stamped = '' }
    if (-not $Version) { $Version = $stamped }
    if (-not $Version) {
        $api = 'https://api.github.com/repos/scmccart/brindle/releases/latest'
        try {
            $Version = (Invoke-WebRequest -UseBasicParsing -Uri $api).Content | ConvertFrom-Json | ForEach-Object { $_.tag_name }
        } catch {
            throw "install.ps1: can't ask $api for the latest release ($($_.Exception.Message)); set `$env:BRINDLE_VERSION"
        }
        if (-not $Version) { throw "install.ps1: $api names no release; set `$env:BRINDLE_VERSION" }
    }
    $Version = $Version -replace '^v', ''
    if ($Version -notmatch '^[0-9A-Za-z.+-]+$') { throw "install.ps1: '$Version' isn't a version" }

    $base = $env:BRINDLE_DOWNLOAD_BASE
    if (-not $base) { $base = "https://github.com/scmccart/brindle/releases/download/v$Version" }
    $base = $base.TrimEnd('/')
    $zip = "brindle-$Version-x86_64-pc-windows-msvc.zip"

    $tmp = Join-Path ([IO.Path]::GetTempPath()) "brindle-install-$([Guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory $tmp | Out-Null
    try {
        Write-Host "downloading Brindle $Version"
        $sums = Join-Path $tmp 'SHA256SUMS'
        $archive = Join-Path $tmp $zip
        Get-Download "$base/SHA256SUMS" $sums
        Get-Download "$base/$zip" $archive
        $want = $null
        foreach ($line in Get-Content -LiteralPath $sums) {
            if ($line -match '^([0-9A-Fa-f]{64}) [ *](.+)$' -and $Matches[2] -eq $zip) {
                $want = $Matches[1].ToLowerInvariant()
                break
            }
        }
        if (-not $want) { throw "install.ps1: SHA256SUMS has no line for $zip; nothing was installed" }
        $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
        if ($got -ne $want) {
            throw "install.ps1: checksum failed for $zip (expected $want, got $got); nothing was installed"
        }

        $stage = Join-Path $tmp 'files'
        Expand-Archive -LiteralPath $archive -DestinationPath $stage
        $have = @($files | Where-Object { Test-Path -LiteralPath (Join-Path $stage $_) -PathType Leaf })
        # Releases up to 0.3.0 have no brindle-remote.exe, the terminal
        # server helper; every one has the other two.
        foreach ($name in 'brindle.exe', 'brindle-remote-linux-x86_64') {
            if ($have -notcontains $name) { throw "install.ps1: $zip has no $name; nothing was installed" }
        }

        New-Item -ItemType Directory -Force $Prefix | Out-Null
        # Windows locks a running exe, but lets it be renamed: move the old
        # one aside so a running Brindle doesn't block the update (as in
        # scripts/install.ps1). A file this release lacks is removed, so the
        # directory holds one release's files.
        foreach ($name in $files) {
            $to = Join-Path $Prefix $name
            $old = "$to.old"
            Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $to) { Move-Item -LiteralPath $to -Destination $old -Force }
            if ($have -contains $name) { Copy-Item -LiteralPath (Join-Path $stage $name) -Destination $to }
            Remove-Item -LiteralPath $old -Force -ErrorAction SilentlyContinue
        }

        # Run the installed copy, so the shortcut points at it.
        Invoke-Brindle '--install-desktop'
        Set-PathEntry $true
        $said = (& $exe --version | Out-String).Trim()
        if ($LASTEXITCODE) { throw "install.ps1: brindle --version failed (exit $LASTEXITCODE)" }
        Write-Host "installed $said in $Prefix"
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
} @args
