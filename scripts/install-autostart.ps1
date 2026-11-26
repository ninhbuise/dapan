# Registers scheduled tasks that start Dapan (and the local TTS server, if installed)
# hidden every time you log in to Windows.
# Run once:  powershell -ExecutionPolicy Bypass -File scripts\install-autostart.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Register-Runner([string]$taskName, [string]$script, [string]$description) {
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' `
        -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$script`"" `
        -WorkingDirectory $root
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
    $settings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
        -MultipleInstances IgnoreNew -StartWhenAvailable
    Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings `
        -Description $description -Force | Out-Null
    Start-ScheduledTask -TaskName $taskName
    Write-Host "Installed and started task '$taskName'."
}

if (Test-Path (Join-Path $root 'tts-server\.venv\Scripts\python.exe')) {
    Register-Runner 'Dapan TTS Server' (Join-Path $PSScriptRoot 'run-tts-server.ps1') `
        'Local VieNeu text-to-speech server for Dapan.'
} else {
    Write-Host 'Local TTS server not installed (tts-server\.venv missing) - Dapan will use Edge TTS.'
}

Register-Runner 'Dapan Discord Bot' (Join-Path $PSScriptRoot 'run-dapan.ps1') `
    'Starts the Dapan Discord bot at login and keeps it running.'

if (-not (Test-Path (Join-Path $root '.env'))) {
    Write-Host 'Note: .env is missing. Copy .env.example to .env and fill it in; the runner waits until it exists.'
}
Write-Host "Logs: $root\logs"
