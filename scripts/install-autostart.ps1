# Registers scheduled tasks that run Dapan and the local TTS server in the background
# from Windows startup, whether or not you are logged in. They run in a background
# session, so no console window ever appears.
#
# Run once (asks for administrator permission):
#   powershell -ExecutionPolicy Bypass -File scripts\install-autostart.ps1

$ErrorActionPreference = 'Stop'

# Registering boot-time tasks needs administrator rights: relaunch elevated if needed.
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    $user = "$env:USERDOMAIN\$env:USERNAME"
    Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-ForUser', "`"$user`"")
    exit
}

$forUser = "$env:USERDOMAIN\$env:USERNAME"
$i = [Array]::IndexOf($args, '-ForUser')
if ($i -ge 0 -and $i + 1 -lt $args.Count) { $forUser = $args[$i + 1] }

$root = Split-Path -Parent $PSScriptRoot

function Register-Runner([string]$taskName, [string]$script, [string]$description) {
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' `
        -Argument "-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$script`"" `
        -WorkingDirectory $root
    # At boot, plus at logon as a backup (a second start is ignored while one is running).
    $triggers = @(
        (New-ScheduledTaskTrigger -AtStartup),
        (New-ScheduledTaskTrigger -AtLogOn -User $forUser)
    )
    # S4U = "run whether user is logged on or not" without storing your password.
    $principal = New-ScheduledTaskPrincipal -UserId $forUser -LogonType S4U -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
        -MultipleInstances IgnoreNew -StartWhenAvailable
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    }
    Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $triggers -Principal $principal `
        -Settings $settings -Description $description | Out-Null
    Write-Host "Installed task '$taskName' (starts with Windows, runs in the background)."
}

# Stop copies started the old way (from a logon session) so the new tasks can take over.
Get-Process dapan -ErrorAction SilentlyContinue | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
    Where-Object { $_.CommandLine -like '*run-dapan.ps1*' -or $_.CommandLine -like '*run-tts-server.ps1*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Get-CimInstance Win32_Process -Filter "Name = 'python.exe'" |
    Where-Object { $_.CommandLine -match 'python\.exe"?\s+server\.py' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Seconds 2

if (Test-Path (Join-Path $root 'tts-server\.venv\Scripts\python.exe')) {
    Register-Runner 'Dapan TTS Server' (Join-Path $PSScriptRoot 'run-tts-server.ps1') `
        'Local VieNeu text-to-speech server for Dapan.'
    Start-ScheduledTask -TaskName 'Dapan TTS Server'
} else {
    Write-Host 'Local TTS server not installed (tts-server\.venv missing) - Dapan will use Edge TTS.'
}

Register-Runner 'Dapan Discord Bot' (Join-Path $PSScriptRoot 'run-dapan.ps1') `
    'Runs the Dapan Discord bot in the background and keeps it running.'
Start-ScheduledTask -TaskName 'Dapan Discord Bot'

if (-not (Test-Path (Join-Path $root '.env'))) {
    Write-Host 'Note: .env is missing. Copy .env.example to .env and fill it in; the runner waits until it exists.'
}
Write-Host "Started. Logs: $root\logs"
Start-Sleep -Seconds 3
