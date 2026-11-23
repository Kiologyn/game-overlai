@echo off
rem Installs the Windows build prerequisites and web dependencies where possible.
rem Keep this file ASCII-only: cmd misreads non-ASCII text in batch files.
setlocal
cd /d "%~dp0..\.."
set "RESTART="

where winget >nul 2>nul || (
    echo winget is unavailable. Install App Installer from the Microsoft Store, then run this again.
    goto :error
)

node -e "process.exit(+process.versions.node.split('.')[0] >= 22 ? 0 : 1)" >nul 2>nul
if errorlevel 1 (
    echo Installing Node.js LTS...
    winget install -e --id OpenJS.NodeJS.LTS --accept-package-agreements --accept-source-agreements || goto :error
    set "RESTART=1"
) else (
    echo Node.js: found
)

where cargo >nul 2>nul
if errorlevel 1 if not exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
    echo Installing Rust with rustup...
    winget install -e --id Rustlang.Rustup --accept-package-agreements --accept-source-agreements || goto :error
    set "RESTART=1"
) else (
    echo Rust: found
)

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VCTOOLS="
if exist "%VSWHERE%" (
    for /f "usebackq delims=" %%p in (`"%VSWHERE%" -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VCTOOLS=%%p"
)
if defined VCTOOLS (
    echo Microsoft C++ Build Tools: found
) else (
    echo Installing Microsoft C++ Build Tools. This may take a while...
    winget install -e --id Microsoft.VisualStudio.2022.BuildTools --accept-package-agreements --accept-source-agreements --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" || goto :error
)

reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>nul || reg query "HKCU\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv >nul 2>nul
if errorlevel 1 (
    echo Installing WebView2 Runtime...
    winget install -e --id Microsoft.EdgeWebView2Runtime --accept-package-agreements --accept-source-agreements || goto :error
) else (
    echo WebView2: found
)

if defined RESTART (
    echo.
    echo A tool was just installed. Open a new terminal and run setup.bat again to install web dependencies.
    goto :done
)

echo Installing web dependencies...
pushd web
call npm ci --no-audit --no-fund || (popd & goto :error)
popd

echo.
echo Ready. Run scripts\windows\dev.bat to start development or scripts\windows\build.bat to build a portable EXE.
where claude >nul 2>nul || echo Claude Code is not on PATH; the app needs Claude Code signed in to answer questions.

:done
call :pause_if_double_clicked
exit /b 0

:error
echo.
echo Setup failed, see the output above.
pause
exit /b 1

:pause_if_double_clicked
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "%~nx0" >nul && pause
exit /b 0
