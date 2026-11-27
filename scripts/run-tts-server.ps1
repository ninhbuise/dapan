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
# The model is already downloaded: don't ask Hugging Face for updates at every start
# (that went through a dead system proxy and made startup take minutes). If an offline
# start fails quickly (e.g. first install, model not downloaded yet), the next try is online.
$offline = $true

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
    if ($offline) { $env:HF_HUB_OFFLINE = '1'; $env:TRANSFORMERS_OFFLINE = '1' }
    else { Remove-Item Env:HF_HUB_OFFLINE, Env:TRANSFORMERS_OFFLINE -ErrorAction SilentlyContinue }
    Write-Log ("Starting TTS server ({0})..." -f $(if ($offline) { 'offline' } else { 'online' }))
    $started = Get-Date
    cmd /c "`"$python`" server.py >> `"$log`" 2>&1"
    $code = $LASTEXITCODE
    $ranFor = (Get-Date) - $started
    if ($ranFor.TotalSeconds -gt 120) { $delay = 5; $offline = $true }
    else { $delay = [Math]::Min($delay * 2, 300); $offline = -not $offline }
    Write-Log ("TTS server exited with code {0} after {1:hh\:mm\:ss}. Restarting in {2}s." -f $code, $ranFor, $delay)
    Start-Sleep -Seconds $delay
}
