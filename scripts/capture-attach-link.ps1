# Captures how real ACP agents treat an attachment Brindle sends as a
# `resource_link` (OpenSpec change `agents-prompt-context`, task 1.1): a file
# in Brindle's attachments directory, outside the agent's working directory.
#
#   scripts/capture-attach-link.ps1                         # every agent, every run
#   scripts/capture-attach-link.ps1 -Agent claude,codex     # some agents
#   scripts/capture-attach-link.ps1 -Run link,embedded      # some runs
#   scripts/capture-attach-link.ps1 -OutDir DIR -Settle 90  # where logs go, seconds a turn gets
#
# Runs, per agent:
#   link      108 KiB of output (`1..20000`) goes as a `resource_link`.
#   no-dirs   the same, with `additionalDirectories` deleted from the session
#             requests (STRIP_DIRS=1): the agent as it was before Brindle named
#             the attachments directory. Only for agents that advertise it.
#   embedded  `1..10`, small enough to go embedded as a `resource`.
#
# Each run opens a Brindle window (it takes focus: don't type elsewhere) on a
# private state, server and cache directory, with the agent behind
# scripts/acp-log-wrapper.py, which writes the fixtures' `c>a`/`a>c` log. The
# logs are named <agent>-<version>-attach-<run>[-perm-<value>][-at-<location>].log
# in -OutDir (default target\capture), with the screen dump (<same>.dump.txt)
# beside them and a summary.txt of what each agent's screen shows.
#
# The summary.txt in -OutDir collects each run's screen lines, run after run.
#
# The binary is $env:BRINDLE_BIN, else target\release\brindle.exe, else
# target\debug\brindle.exe.

param(
    [ValidateSet('claude', 'codex', 'copilot', 'agy')]
    [string[]]$Agent = @('claude', 'codex', 'copilot', 'agy'),
    [ValidateSet('link', 'no-dirs', 'embedded')]
    [string[]]$Run = @('link', 'no-dirs', 'embedded'),
    # Where the run's files sit (the agent's cwd and Brindle's attachments
    # directory, which is what the link points into):
    #   temp     both under %TEMP%, side by side (the default, and what the
    #            first captures used)
    #   appdata  the cwd under the profile directory, the attachments under
    #            %LOCALAPPDATA%: two separate trees, as in a real install
    #   home     both under the profile directory, side by side
    [ValidateSet('temp', 'appdata', 'home')]
    [string[]]$Location = @('temp'),
    # The agent's permission setting for the run, set through its mode option
    # (Copilot: its Permissions option) as soon as the session exists:
    # `untouched` leaves the agent's own, `all` is every other value it offers,
    # or name values (claude: default acceptEdits plan auto bypassPermissions,
    # codex: read-only workspace-write agent agent-full-access, copilot: off
    # auto on, agy: default accept-edits auto plan).
    [string[]]$Perm = @('untouched'),
    [string]$OutDir,
    # Seconds a run waits for the agent's answer before it dumps the screen.
    [int]$Settle = 75,
    [string]$Prompt = 'What is the last number in the attached file?',
    # Ends on the terminal zone, so the dump shows its screen (for debugging).
    [switch]$ShowTerminal,
    # Stops after attaching the output, before the prompt is typed (for debugging).
    [switch]$NoSubmit
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $root 'target\capture' }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).ProviderPath

$bin = $env:BRINDLE_BIN
if (-not $bin) {
    foreach ($candidate in @('target\release\brindle.exe', 'target\debug\brindle.exe')) {
        $path = Join-Path $root $candidate
        if (Test-Path -LiteralPath $path) { $bin = $path; break }
    }
}
if (-not $bin -or -not (Test-Path -LiteralPath $bin)) { throw 'no brindle.exe (cargo build, or set BRINDLE_BIN)' }
$bin = (Resolve-Path -LiteralPath $bin).ProviderPath
$helper = Join-Path (Split-Path -Parent $bin) 'brindle-remote.exe'
$python = (Get-Command python -ErrorAction Stop).Source
$wrapper = Join-Path $PSScriptRoot 'acp-log-wrapper.py'

# What each agent runs. `dirs` is whether its `initialize` advertises
# sessionCapabilities.additionalDirectories, so `no-dirs` means something.
$agents = @{
    claude  = @{ command = @('cmd', '/c', 'npx', '-y', '@agentclientprotocol/claude-agent-acp'); dirs = $true
                 option = 'mode'; initial = 'auto'; values = @('default', 'acceptEdits', 'plan', 'auto', 'bypassPermissions') }
    codex   = @{ command = @('cmd', '/c', 'npx', '-y', '@agentclientprotocol/codex-acp'); dirs = $true
                 option = 'mode'; initial = 'agent'; values = @('read-only', 'workspace-write', 'agent', 'agent-full-access') }
    copilot = @{ command = @('copilot', '--acp', '--model', 'auto'); dirs = $false
                 option = 'allow_all'; initial = 'off'; values = @('off', 'auto', 'on') }
    agy     = @{ command = @('agy', 'acp'); dirs = $true
                 option = 'mode'; initial = 'default'; values = @('default', 'accept-edits', 'auto', 'plan') }
}

function ConvertTo-Toml([string]$Text) { '"' + ($Text -replace '\\', '\\' -replace '"', '\"') + '"' }

function Invoke-Capture([string]$Name, [string]$Kind, [string]$Where, [string]$Setting) {
    $spec = $agents[$Name]
    if ($Kind -eq 'no-dirs' -and -not $spec.dirs) {
        Write-Host "capture: $Name advertises no additionalDirectories: skipping no-dirs"
        return
    }
    $stem = "$Name-$Kind"
    if ($Setting -ne 'untouched') { $stem += "-perm-$Setting" }
    if ($Where -ne 'temp') { $stem += "-at-$Where" }
    $id = "brindle-capture-$([guid]::NewGuid().ToString('N').Substring(0, 8))"
    $dir = Join-Path ([System.IO.Path]::GetTempPath()) $id
    $home_ = Join-Path $env:USERPROFILE $id
    $local = Join-Path $env:LOCALAPPDATA $id
    # Where the agent's cwd and the attachments directory sit.
    switch ($Where) {
        'temp'    { $project = Join-Path $dir 'project'; $cache = Join-Path $dir 'cache' }
        'appdata' { $project = Join-Path $home_ 'project'; $cache = Join-Path $local 'cache' }
        'home'    { $project = Join-Path $home_ 'project'; $cache = Join-Path $home_ 'cache' }
    }
    foreach ($path in (Join-Path $dir 'state'), (Join-Path $dir 'server'), $project, $cache) {
        New-Item -ItemType Directory -Path $path -Force | Out-Null
    }
    $log = Join-Path $dir 'raw.log'
    $args = @($wrapper, '--log', $log, '--') + $spec.command
    $argsToml = ($args | ForEach-Object { ConvertTo-Toml $_ }) -join ', '
    $vars = @()
    if ($Kind -eq 'no-dirs') { $vars += 'STRIP_DIRS = "1"' }
    if ($Setting -ne 'untouched') { $vars += "SET_OPTIONS = $(ConvertTo-Toml "$($spec.option)=$Setting")" }
    $env_ = if ($vars) { "env = { $($vars -join ', ') }" } else { '' }
    $config = @"
default_profile = "PowerShell"
scrollback_lines = 100000

[[profiles]]
name = "PowerShell"
command = "pwsh"
args = ["-NoLogo", "-NoProfile", "-NoExit", "-Command", $(ConvertTo-Toml "& '$bin' --shell-integration pwsh | Out-String | Invoke-Expression")]

[[profiles]]
name = "Capture"
type = "agent"
command = $(ConvertTo-Toml $python)
args = [$argsToml]
cwd = $(ConvertTo-Toml $project)
$env_
"@
    $configPath = Join-Path $dir 'config.toml'
    Set-Content -LiteralPath $configPath -Value $config -Encoding utf8

    $count = if ($Kind -eq 'embedded') { 10 } else { 20000 }
    $last = "$count"
    $steps = @(
        '--wait-for', 'PS ',
        '--send', "1..$count\r",
        '--wait-for', $last,
        '--action', 'split_agent_right',
        '--wait-for', 'prompt status=ready',
        '--action', 'focus_zone_left',
        '--action', 'send_last_output_to_agent',
        '--action', 'focus_zone_right',
        '--send', $Prompt,
        '--action', 'agent_submit'
    )
    if ($NoSubmit) { $steps = $steps[0..($steps.Count - 5)] }
    if ($ShowTerminal) { $steps += @('--action', 'focus_zone_left') }
    $dump = Join-Path $OutDir "$stem.dump.txt"
    $argList = @("terminal", "PowerShell", "--no-restore", "--step-interval", "3", "-d", $project) + $steps + @('--dump-screen-after', "$Settle")

    $previous = @{}
    $names = 'BRINDLE_CONFIG', 'BRINDLE_STATE_DIR', 'BRINDLE_SERVER_DIR', 'BRINDLE_CACHE_DIR', 'BRINDLE_SERVER_HELPER'
    foreach ($n in $names) { $previous[$n] = [Environment]::GetEnvironmentVariable($n) }
    try {
        Write-Host "capture: $stem (window opens; $Settle s; files in $project and $cache)"
        $env:BRINDLE_CONFIG = $configPath
        $env:BRINDLE_STATE_DIR = Join-Path $dir 'state'
        $env:BRINDLE_SERVER_DIR = Join-Path $dir 'server'
        $env:BRINDLE_CACHE_DIR = $cache
        if (Test-Path -LiteralPath $helper) { $env:BRINDLE_SERVER_HELPER = $helper }
        $quoted = ($argList | ForEach-Object {
            if ($_ -ne '' -and $_ -notmatch '[\s"]') { $_ } else { '"' + ($_ -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' }
        }) -join ' '
        $process = Start-Process -FilePath $bin -ArgumentList $quoted -PassThru -RedirectStandardOutput $dump -RedirectStandardError "$dump.err"
        if (-not $process.WaitForExit(($Settle + 120) * 1000)) {
            $process.Kill()
            Write-Warning "capture: $stem timed out"
        }
    } finally {
        foreach ($n in $names) { [Environment]::SetEnvironmentVariable($n, $previous[$n]) }
        if (Test-Path -LiteralPath $helper) {
            try { $env:BRINDLE_SERVER_DIR = Join-Path $dir 'server'; & $helper stop *> $null } catch {}
            $env:BRINDLE_SERVER_DIR = $previous['BRINDLE_SERVER_DIR']
        }
    }

    $summary = Join-Path $OutDir 'summary.txt'
    Add-Content -LiteralPath $summary -Value "=== $stem (location $Where; permission $Setting; attachments $cache)"
    if (Test-Path -LiteralPath $dump) {
        Get-Content -LiteralPath $dump |
            Where-Object { $_ -match '^--- (agent zone|entry \d+ (user|agent|tool|permission|notice)|attached|location|permission|item entry=\d+ index=\d+ kind=text|wait-for)' } |
            ForEach-Object { if ($_.Length -gt 400) { $_.Substring(0, 400) } else { $_ } } |
            Add-Content -LiteralPath $summary
    }

    # The log is named for the version the agent reports.
    $version = 'unknown'
    $agentName = $Name
    if (Test-Path -LiteralPath $log) {
        $init = Get-Content -LiteralPath $log | Where-Object { $_ -match 'agentInfo' } | Select-Object -First 1
        if ($init -match '"agentInfo":\{[^}]*"version":"([^"]*)"') { $version = $Matches[1] }
        Copy-Item -LiteralPath $log -Destination (Join-Path $OutDir ("$Name-$version-attach-" + $stem.Substring($Name.Length + 1) + '.log')) -Force
    } else {
        Write-Warning "capture: $stem wrote no log"
    }
    foreach ($path in $dir, $home_, $local) {
        for ($try = 0; $try -lt 20 -and (Test-Path -LiteralPath $path); $try++) {
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $path) { Start-Sleep -Milliseconds 250 }
        }
    }
}

foreach ($name in $Agent) {
    $spec = $agents[$name]
    $settings = foreach ($value in $Perm) {
        if ($value -eq 'all') { $spec.values | Where-Object { $_ -ne $spec.initial } }
        elseif ($value -eq 'untouched' -or $spec.values -contains $value) { $value }
        else { Write-Warning "capture: $name has no permission value '$value' (it offers $($spec.values -join ', '))" }
    }
    foreach ($where in $Location) {
        foreach ($setting in $settings) {
            foreach ($kind in $Run) { Invoke-Capture $name $kind $where $setting }
        }
    }
}
Write-Host "capture: logs and dumps in $OutDir"
