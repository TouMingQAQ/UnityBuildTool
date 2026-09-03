@echo off
setlocal
cd /d "%~dp0"

where node.exe >nul 2>nul
if errorlevel 1 goto :nonode

echo =============================================
echo   TBuildTool · 批量打包工具 (Web)
echo   URL: http://127.0.0.1:8787
echo   Close this window to stop the server.
echo =============================================
node server.js
echo.
echo Server exited.
pause
exit /b 0

:nonode
echo [ERROR] Node.js not found in PATH.
echo Please install Node.js 18+ and add "node.exe" to PATH.
pause
exit /b 1
