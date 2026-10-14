@echo off
setlocal
cd /d "%~dp0.."
where node.exe >nul 2>nul
if errorlevel 1 goto :nonode

echo TBuildTool - Tauri/Rust backend
call npm run app:dev
set "RESULT=%ERRORLEVEL%"
if not "%RESULT%"=="0" pause
exit /b %RESULT%

:nonode
echo [ERROR] Node.js is required for the development launcher.
echo Installed Tauri builds do not require Node.js.
pause
exit /b 1
