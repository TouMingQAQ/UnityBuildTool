@echo off
setlocal
cd /d "%~dp0"

where node.exe >nul 2>nul
if errorlevel 1 goto :nonode

rem 读取 config.json 里的 serverPort（默认 8787）
set "PORT=8787"
for /f %%P in ('node -e "try{console.log(JSON.parse(require('fs').readFileSync('config.json','utf8')).serverPort||8787)}catch(e){console.log(8787)}"') do set "PORT=%%P"

echo =============================================
echo   TBuildTool · 批量打包工具 (Web)
echo   URL: http://127.0.0.1:%PORT%
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
