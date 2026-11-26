# Registers a scheduled task that starts Dapan (hidden) every time you log in to Windows.
# Run once:  powershell -ExecutionPolicy Bypass -File scripts\install-autostart.ps1

$ErrorActionPreference = 'Stop'
$taskName = 'Dapan Discord Bot'
$runner = Join-Path $PSScriptRoot 'run-dapan.ps1'
$root = Split-Path -Parent $PSScriptRoot

$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$runner`"" `
    -WorkingDirectory $root
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew -StartWhenAvailable

Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings `
    -Description 'Starts the Dapan Discord bot at login and keeps it running.' -Force | Out-Null

Write-Host "Installed task '$taskName'."
if (-not (Test-Path (Join-Path $root '.env'))) {
    Write-Host 'Note: .env is missing. Copy .env.example to .env and fill it in; the runner waits until it exists.'
}

Start-ScheduledTask -TaskName $taskName
Write-Host "Dapan is starting in the background. Logs: $root\logs"
