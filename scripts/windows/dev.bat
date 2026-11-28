@echo off
rem Starts the Vite UI and the Rust/Tauri app for development.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."

where node >nul 2>nul || goto :missing_node
where npm >nul 2>nul || goto :missing_node
where cargo >nul 2>nul || goto :missing_rust

if not exist "web\node_modules\vite\bin\vite.js" (
    echo Installing web dependencies...
    pushd web
    call npm ci --no-audit --no-fund || (popd & goto :error)
    popd
)

echo Starting Vite and game-overlai in this window...
node scripts\windows\dev-runner.mjs
if errorlevel 1 goto :error
exit /b 0

:missing_node
echo Node.js and npm are required. Run scripts\windows\setup.bat first.
goto :error

:missing_rust
echo Rust is required. Run scripts\windows\setup.bat first.
goto :error

:error
echo.
echo Development app failed to start, see the output above.
pause
exit /b 1
