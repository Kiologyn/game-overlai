@echo off
rem Builds the installer and a portable executable into release\<version>\.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."

where node >nul 2>nul || goto :missing_node
where npm >nul 2>nul || goto :missing_node
where cargo >nul 2>nul || goto :missing_rust

for /f "usebackq delims=" %%v in (`node -p "require('./web/package.json').version"`) do set "VERSION=%%v"
if "%VERSION%"=="" goto :missing_version

if not exist "web\node_modules\.bin\tauri.cmd" (
    echo Installing web dependencies...
    pushd web
    call npm ci --no-audit --no-fund || (popd & goto :error)
    popd
)

echo Building the web interface...
pushd web
call npm run build || (popd & goto :error)
popd

echo Building game-overlai and its installer...
rem The Tauri CLI builds the release exe and the NSIS installer (it downloads NSIS on first use).
pushd src-tauri
call "..\web\node_modules\.bin\tauri.cmd" build || (popd & goto :error)
popd

set "OUT=release\%VERSION%"
if not exist "%OUT%" mkdir "%OUT%" || goto :error
copy /y "src-tauri\target\release\game-overlai.exe" "%OUT%\game-overlai-%VERSION%-portable.exe" >nul || goto :error
copy /y "src-tauri\target\release\bundle\nsis\game-overlai_%VERSION%_x64-setup.exe" "%OUT%\game-overlai-%VERSION%-setup.exe" >nul || goto :error

echo.
echo Done:
echo   %OUT%\game-overlai-%VERSION%-setup.exe     installer, no admin rights needed
echo   %OUT%\game-overlai-%VERSION%-portable.exe  portable
call :pause_if_double_clicked
exit /b 0

:missing_node
echo Node.js and npm are required. Install Node.js LTS, then run this file again.
goto :error

:missing_rust
echo Rust is required. Install Rust with rustup and the MSVC C++ build tools, then run this file again.
goto :error

:missing_version
echo Could not read the version from web\package.json.
goto :error

:error
echo.
echo Build failed, see the output above.
pause
exit /b 1

:pause_if_double_clicked
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "%~nx0" >nul && pause
exit /b 0
