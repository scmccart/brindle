# Brindle shell integration for PowerShell: OSC 133 semantic prompt marks,
# and the current directory reported with OSC 7.
#
# Load it at the end of your $PROFILE, after anything that defines `prompt`:
#
#     brindle --shell-integration pwsh | Out-String | Invoke-Expression
#
# or, from a checkout:
#
#     . /path/to/brindle/shell-integration/brindle.ps1
#
# It works in PowerShell 7 (pwsh) and Windows PowerShell 5.1. It marks the
# prompt (A), where the command line starts (B), where the command's output
# starts (C; needs PSReadLine, which both load by default) and the command's
# end with its exit status (D). The directory report is what new tabs and
# session restore start in on Windows. Terminals that don't know these
# sequences ignore them.

# Only in an interactive console: not for `-Command`, `-File` or a script
# (unless `-NoExit` keeps the session open), and not in other hosts.
$__brindleArgs = [Environment]::GetCommandLineArgs() | Select-Object -Skip 1
$__brindleBatch = ($__brindleArgs | Where-Object { $_ -match '^-(c|command|f|file|e|ec|encodedcommand|noni|noninteractive)$' }) -and
    -not ($__brindleArgs | Where-Object { $_ -match '^-(noe|noexit)$' })
if ($Host.Name -eq 'ConsoleHost' -and -not $__brindleBatch -and -not $global:__BrindleShellIntegration) {
    $global:__BrindleShellIntegration = $true
    # The history id of the last command a D was sent for.
    $global:__BrindleLastId = (Get-History -Count 1).Id
    $global:__BrindlePrompt = $function:prompt

    function global:prompt {
        # Read first: everything below changes $?.
        $success = $?
        $exitCode = $global:LASTEXITCODE
        $esc = [char]27
        $bel = [char]7
        $out = ''

        # End the previous command, if a line actually ran (an empty line
        # adds no history, and got no C).
        $last = Get-History -Count 1
        if ($last -and $last.Id -ne $global:__BrindleLastId) {
            $global:__BrindleLastId = $last.Id
            $status = if ($success) { 0 } elseif ($exitCode) { $exitCode } else { 1 }
            $out += "$esc]133;D;$status$bel"
        }

        # The directory, as a file URI: C:\My Files is /C:/My%20Files.
        $location = $executionContext.SessionState.Path.CurrentLocation
        if ($location.Provider.Name -eq 'FileSystem') {
            $segments = $location.ProviderPath.Replace('\', '/').Split('/') | ForEach-Object { [Uri]::EscapeDataString($_) }
            $path = ($segments -join '/').TrimStart('/')
            $out += "$esc]7;file://$env:COMPUTERNAME/$path$esc\"
        }

        # The user's prompt, with $? and $LASTEXITCODE as the command left them.
        $global:LASTEXITCODE = $exitCode
        if (-not $success) { Write-Error '' -ErrorAction Ignore }
        $text = (& $global:__BrindlePrompt) -join ''
        $global:LASTEXITCODE = $exitCode

        "$out$esc]133;A$bel$text$esc]133;B$bel"
    }

    # The command line is read: its output starts here.
    if (Get-Command PSConsoleHostReadLine -ErrorAction Ignore) {
        $global:__BrindleReadLine = $function:PSConsoleHostReadLine
        function global:PSConsoleHostReadLine {
            $line = & $global:__BrindleReadLine
            if ($line) { [Console]::Write("$([char]27)]133;C$([char]7)") }
            $line
        }
    }
}
Remove-Variable __brindleArgs, __brindleBatch -ErrorAction Ignore
