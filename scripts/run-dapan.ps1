# Runs Dapan forever: restarts it when it exits or crashes, and writes logs to .\logs.
# Started at Windows login by the scheduled task from install-autostart.ps1,
# but you can also run it by hand:  powershell -ExecutionPolicy Bypass -File scripts\run-dapan.ps1

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Only one runner at a time (e.g. if you log in twice or start it by hand).
$created = $false
$mutex = New-Object System.Threading.Mutex($true, 'Global\DapanDiscordBotRunner', [ref]$created)
if (-not $created) {
    Write-Host 'Dapan is already running.'
    exit 0
}

$logDir = Join-Path $root 'logs'
New-Item -ItemType Directory -Force $logDir | Out-Null
$exe = Join-Path $root 'target\release\dapan.exe'

function Write-Log([string]$msg) {
    $line = '{0:yyyy-MM-dd HH:mm:ss} [runner] {1}' -f (Get-Date), $msg
    Add-Content -Path (Join-Path $logDir ('dapan-{0:yyyy-MM-dd}.log' -f (Get-Date))) -Value $line -Encoding UTF8
    Write-Host $line
}

function Update-PathFromRegistry {
    # Pick up tools installed after this session started (yt-dlp, ffmpeg, cargo).
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [Environment]::GetEnvironmentVariable('Path', 'User') + ';' +
                (Join-Path $env:USERPROFILE '.cargo\bin')
}

$delay = 5
while ($true) {
    Update-PathFromRegistry

    # Delete logs older than 14 days.
    Get-ChildItem $logDir -Filter 'dapan-*.log' |
        Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-14) } |
        Remove-Item -Force -ErrorAction SilentlyContinue

    if (-not (Test-Path (Join-Path $root '.env'))) {
        Write-Log '.env not found - copy .env.example to .env and fill it in. Checking again in 60s.'
        Start-Sleep -Seconds 60
        continue
    }

    if (-not (Test-Path $exe)) {
        Write-Log 'Release build not found, building (this takes a few minutes)...'
        $log = Join-Path $logDir ('dapan-{0:yyyy-MM-dd}.log' -f (Get-Date))
        cmd /c "cargo build --release >> `"$log`" 2>&1"
        if ($LASTEXITCODE -ne 0) {
            Write-Log "Build failed (exit $LASTEXITCODE). Retrying in 5 minutes."
            Start-Sleep -Seconds 300
            continue
        }
    }

    $log = Join-Path $logDir ('dapan-{0:yyyy-MM-dd}.log' -f (Get-Date))
    Write-Log 'Starting Dapan...'
    $started = Get-Date
    cmd /c "`"$exe`" >> `"$log`" 2>&1"
    $code = $LASTEXITCODE
    $ranFor = (Get-Date) - $started

    # Crashing right away (no internet yet, bad token...) backs off up to 5 minutes;
    # a run that lasted a while restarts quickly.
    if ($ranFor.TotalSeconds -gt 120) { $delay = 5 } else { $delay = [Math]::Min($delay * 2, 300) }
    Write-Log ("Dapan exited with code {0} after {1:hh\:mm\:ss}. Restarting in {2}s." -f $code, $ranFor, $delay)
    Start-Sleep -Seconds $delay
}
