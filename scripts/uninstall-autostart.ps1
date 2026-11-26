# Stops Dapan and removes the login task.
# Run:  powershell -ExecutionPolicy Bypass -File scripts\uninstall-autostart.ps1

$taskName = 'Dapan Discord Bot'
if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    Write-Host "Removed task '$taskName'."
} else {
    Write-Host "Task '$taskName' is not installed."
}
Get-Process dapan -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Host 'Dapan stopped.'
