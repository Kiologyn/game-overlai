@echo off
rem Builds a portable Windows executable into release\<version>\.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."

where node >nul 2>nul || goto :missing_node
where npm >nul 2>nul || goto :missing_node
where cargo >nul 2>nul || goto :missing_rust

for /f "usebackq delims=" %%v in (`node -p "require('./src-tauri/tauri.conf.json').version"`) do set "VERSION=%%v"
if "%VERSION%"=="" goto :missing_version

if not exist "web\node_modules\vite\bin\vite.js" (
    echo Installing web dependencies...
    pushd web
    call npm ci --no-audit --no-fund || (popd & goto :error)
    popd
)

echo Building the web interface...
pushd web
call npm run build || (popd & goto :error)
popd

echo Building game-overlai for Windows...
pushd src-tauri
call cargo build --release || (popd & goto :error)
popd

set "OUT=release\%VERSION%"
if not exist "%OUT%" mkdir "%OUT%" || goto :error
copy /y "src-tauri\target\release\game-overlai.exe" "%OUT%\game-overlai-%VERSION%.exe" >nul || goto :error

echo.
echo Done:
echo   %OUT%\game-overlai-%VERSION%.exe
call :pause_if_double_clicked
exit /b 0

:missing_node
echo Node.js and npm are required. Install Node.js LTS, then run this file again.
goto :error

:missing_rust
echo Rust is required. Install Rust with rustup and the MSVC C++ build tools, then run this file again.
goto :error

:missing_version
echo Could not read the version from src-tauri\tauri.conf.json.
goto :error

:error
echo.
echo Build failed, see the output above.
pause
exit /b 1

:pause_if_double_clicked
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "%~nx0" >nul && pause
exit /b 0
