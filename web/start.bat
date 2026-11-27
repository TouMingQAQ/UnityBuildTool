@echo off
setlocal
cd /d "%~dp0.."
where node.exe >nul 2>nul
if errorlevel 1 goto :nonode

echo TBuildTool - legacy Node user web reference
node server.js
set "RESULT=%ERRORLEVEL%"
if not "%RESULT%"=="0" pause
exit /b %RESULT%

:nonode
echo [ERROR] Node.js is required for the development launcher.
echo Use "npm run app:dev" from the repository root for the native egui admin.
pause
exit /b 1
