@echo off
setlocal
title Spotify OBS Overlay - Restart and Check
cd /d "%~dp0"

echo [1/3] Closing previous Spotify OBS Overlay sessions...
taskkill /IM spotify-overlay.exe /F >nul 2>&1

if not exist "%~dp0spotify-overlay.exe" (
  echo.
  echo ERROR: spotify-overlay.exe is not beside this file.
  echo Extract the complete release ZIP into one folder, then try again.
  echo.
  pause
  exit /b 1
)

echo [2/3] Starting a fresh session...
start "" "%~dp0spotify-overlay.exe"

echo [3/3] Checking the local service...
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$healthy = $false;" ^
  "for ($attempt = 0; $attempt -lt 20; $attempt++) {" ^
  "  Start-Sleep -Milliseconds 250;" ^
  "  try {" ^
  "    $health = Invoke-RestMethod -Uri 'http://127.0.0.1:18923/health' -TimeoutSec 1;" ^
  "    if ($health.status -eq 'ok') { $healthy = $true; break }" ^
  "  } catch {}" ^
  "}" ^
  "if ($healthy) {" ^
  "  Write-Host '';" ^
  "  Write-Host 'OK: Spotify OBS Overlay is running.' -ForegroundColor Green;" ^
  "  Write-Host 'OBS URL: http://127.0.0.1:18923/';" ^
  "  exit 0;" ^
  "}" ^
  "Write-Host '';" ^
  "Write-Host 'ERROR: The overlay did not start.' -ForegroundColor Red;" ^
  "if (Test-Path '.\spotify-overlay.log') {" ^
  "  Write-Host '';" ^
  "  Write-Host 'Last log messages:' -ForegroundColor Yellow;" ^
  "  Get-Content '.\spotify-overlay.log' -Tail 30;" ^
  "} else {" ^
  "  Write-Host 'No spotify-overlay.log file was created.';" ^
  "}" ^
  "exit 1"

set "check_status=%errorlevel%"
echo.
pause
exit /b %check_status%
