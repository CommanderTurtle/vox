@echo off
REM Install vox as Windows autostart (via registry Run key)
REM Run as Administrator: right-click → Run as administrator
REM Usage: scripts\install-autostart.bat

setlocal

set "VOX_PATH=%~dp0..\target\release\vox.exe"

if not exist "%VOX_PATH%" (
    echo Building vox release first...
    pushd "%~dp0.."
    cargo build --release
    popd
)

for %%i in ("%VOX_PATH%") do set "VOX_ABS=%%~fi"

echo Installing vox autostart from: %VOX_ABS%

reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" ^
    /v "vox" ^
    /t REG_SZ ^
    /d "%VOX_ABS%" ^
    /f

if %errorlevel% equ 0 (
    echo ✅ Autostart registered for current user.
    echo    (visible in Task Manager → Startup tab)
) else (
    echo ❌ Failed to register autostart. Try running as Administrator.
)

pause
