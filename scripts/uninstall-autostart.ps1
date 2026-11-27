# Stops Dapan and the TTS server and removes their scheduled tasks.
# Run (asks for administrator permission):
#   powershell -ExecutionPolicy Bypass -File scripts\uninstall-autostart.ps1

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    exit
}

foreach ($taskName in 'Dapan Discord Bot', 'Dapan TTS Server') {
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        Write-Host "Removed task '$taskName'."
    }
}
& (Join-Path $PSScriptRoot 'stop-dapan.ps1') -NoElevate
Write-Host 'Dapan stopped.'
Start-Sleep -Seconds 2
