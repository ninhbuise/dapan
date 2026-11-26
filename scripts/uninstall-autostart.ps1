# Stops Dapan and the TTS server and removes their login tasks.
# Run:  powershell -ExecutionPolicy Bypass -File scripts\uninstall-autostart.ps1

foreach ($taskName in 'Dapan Discord Bot', 'Dapan TTS Server') {
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
        Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        Write-Host "Removed task '$taskName'."
    }
}
Get-Process dapan -ErrorAction SilentlyContinue | Stop-Process -Force
$server = Join-Path (Split-Path -Parent $PSScriptRoot) 'tts-server'
Get-CimInstance Win32_Process -Filter "Name = 'python.exe'" |
    Where-Object { $_.CommandLine -like '*server.py*' -and $_.ExecutablePath -like "$server*" } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Write-Host 'Dapan stopped.'
