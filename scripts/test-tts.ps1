# Tests the local TTS server directly (no Discord): times each request and plays the result.
#   powershell -ExecutionPolicy Bypass -File scripts\test-tts.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\test-tts.ps1 -Text "Chào buổi sáng!" -Voice "Mai Anh"
#   powershell -ExecutionPolicy Bypass -File scripts\test-tts.ps1 -ListVoices

param(
    [string]$Text = 'Xin chào các bạn, mình là Dapan nè!',
    [string]$Voice = '',
    [string]$Url = 'http://127.0.0.1:5005',
    [switch]$ListVoices,
    [switch]$NoPlay
)

try {
    Invoke-RestMethod "$Url/health" -TimeoutSec 3 | Out-Null
} catch {
    Write-Host "TTS server is not answering at $Url. Is the 'Dapan TTS Server' task running? See logs\tts-*.log" -ForegroundColor Red
    exit 1
}

if ($ListVoices) {
    $v = Invoke-RestMethod "$Url/voices"
    Write-Host "Default: $($v.default)"
    $v.voices | ForEach-Object { Write-Host " - $_" }
    exit 0
}

$body = @{ text = $Text }
if ($Voice) { $body.voice = $Voice }
$json = [Text.Encoding]::UTF8.GetBytes(($body | ConvertTo-Json -Compress))
$out = Join-Path $env:TEMP 'dapan-tts-test.wav'

$sw = [Diagnostics.Stopwatch]::StartNew()
try {
    Invoke-WebRequest "$Url/tts" -Method Post -ContentType 'application/json; charset=utf-8' `
        -Body $json -OutFile $out -TimeoutSec 60 -UseBasicParsing
} catch {
    Write-Host "Request failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
$sw.Stop()
$bytes = (Get-Item $out).Length
$audioSecs = [Math]::Max(0, $bytes - 44) / (48000 * 2)
Write-Host ("{0:N2}s to make {1:N1}s of audio ({2} chars) -> {3}" -f $sw.Elapsed.TotalSeconds, $audioSecs, $Text.Length, $out)

if (-not $NoPlay) {
    $player = New-Object Media.SoundPlayer $out
    $player.PlaySync()
}
