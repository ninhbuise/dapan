# Runs the local VieNeu TTS server forever: restarts it if it stops, logs to .\logs.
# Started at login by install-autostart.ps1 (when tts-server\.venv exists).
# Manual run:  powershell -ExecutionPolicy Bypass -File scripts\run-tts-server.ps1

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$server = Join-Path $root 'tts-server'
$python = Join-Path $server '.venv\Scripts\python.exe'
Set-Location $server

$created = $false
$mutex = New-Object System.Threading.Mutex($true, 'Global\DapanTtsServerRunner', [ref]$created)
if (-not $created) { Write-Host 'TTS server is already running.'; exit 0 }

$logDir = Join-Path $root 'logs'
New-Item -ItemType Directory -Force $logDir | Out-Null
$env:PYTHONIOENCODING = 'utf-8'
$env:PYTHONUNBUFFERED = '1'

function Write-Log([string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss} [tts-runner] {1}' -f (Get-Date), $msg
    Add-Content -Path (Join-Path $logDir ('tts-{0:yyyy-MM-dd}.log' -f (Get-Date))) -Value $line -Encoding UTF8
}

if (-not (Test-Path $python)) {
    Write-Log "Python environment not found at $python - see README (Local voice)."
    exit 1
}

$delay = 5
while ($true) {
    Get-ChildItem $logDir -Filter 'tts-*.log' |
        Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-14) } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    $log = Join-Path $logDir ('tts-{0:yyyy-MM-dd}.log' -f (Get-Date))
    Write-Log 'Starting TTS server...'
    $started = Get-Date
    cmd /c "`"$python`" server.py >> `"$log`" 2>&1"
    $code = $LASTEXITCODE
    $ranFor = (Get-Date) - $started
    if ($ranFor.TotalSeconds -gt 120) { $delay = 5 } else { $delay = [Math]::Min($delay * 2, 300) }
    Write-Log ("TTS server exited with code {0} after {1:hh\:mm\:ss}. Restarting in {2}s." -f $code, $ranFor, $delay)
    Start-Sleep -Seconds $delay
}
