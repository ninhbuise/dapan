# Stops Dapan and the TTS server right now (they start again at the next boot,
# or with scripts\restart-dapan.ps1).
#   powershell -ExecutionPolicy Bypass -File scripts\stop-dapan.ps1

param([switch]$NoElevate)

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin -and -not $NoElevate) {
    Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    exit
}

foreach ($taskName in 'Dapan Discord Bot', 'Dapan TTS Server') {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
}
# Stopping a task ends its runner script; also end the programs it started.
Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
    Where-Object { $_.CommandLine -like '*run-dapan.ps1*' -or $_.CommandLine -like '*run-tts-server.ps1*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Get-Process dapan -ErrorAction SilentlyContinue | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name = 'python.exe'" |
    Where-Object { $_.CommandLine -like '*server.py*' } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Write-Host 'Dapan and the TTS server are stopped.'
