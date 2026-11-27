# Restarts Dapan and the TTS server in the background, e.g. after editing .env.
# With -Build it also rebuilds Dapan first (after changing the code).
#   powershell -ExecutionPolicy Bypass -File scripts\restart-dapan.ps1 [-Build]

param([switch]$Build)

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    if ($Build) { $argList += '-Build' }
    Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList $argList
    exit
}

$root = Split-Path -Parent $PSScriptRoot
& (Join-Path $PSScriptRoot 'stop-dapan.ps1') -NoElevate

if ($Build) {
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [Environment]::GetEnvironmentVariable('Path', 'User') + ';' +
                (Join-Path $env:USERPROFILE '.cargo\bin')
    Push-Location $root
    cargo build --release
    $ok = $LASTEXITCODE -eq 0
    Pop-Location
    if (-not $ok) { Write-Host 'Build failed; starting the previous version.' }
}

foreach ($taskName in 'Dapan TTS Server', 'Dapan Discord Bot') {
    if (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue) {
        Start-ScheduledTask -TaskName $taskName
        Write-Host "Started '$taskName'."
    }
}
Start-Sleep -Seconds 3
