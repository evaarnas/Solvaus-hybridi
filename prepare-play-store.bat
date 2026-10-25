@echo off
setlocal

cd /d "%~dp0"
if errorlevel 1 goto :fail

if exist ".venv\Scripts\python.exe" (
    set "PYTHON=.venv\Scripts\python.exe"
) else (
    where python >nul 2>nul
    if errorlevel 1 (
        echo Python was not found on PATH.
        goto :fail
    )
    set "PYTHON=python"
)

where wasm-pack >nul 2>nul
if errorlevel 1 (
    echo wasm-pack was not found on PATH. Install it before preparing the Android app.
    goto :fail
)

where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust Cargo was not found on PATH.
    goto :fail
)

echo Regenerating the web frontend...
"%PYTHON%" tools\gen-index.py
if errorlevel 1 goto :fail

echo Building the WASM package...
"%PYTHON%" tools\build-wasm.py
if errorlevel 1 goto :fail

echo.
echo Generating the Android Studio project...
pushd "src-tauri"
cargo tauri android init --ci
set "INIT_EXIT=%ERRORLEVEL%"
popd
if not "%INIT_EXIT%"=="0" (
    echo Android project generation failed with exit code %INIT_EXIT%.
    exit /b %INIT_EXIT%
)

echo Applying the tracked Android project settings...
"%PYTHON%" tools\configure-android.py
if errorlevel 1 goto :fail

echo.
echo Android Studio project is ready at src-tauri\gen\android. No AAB was built.
echo Starting the Tauri CLI bridge and opening Android Studio.
echo Keep this window open while building the AAB in Android Studio.
echo.
pushd "src-tauri"
cargo tauri android dev --open
set "TAURI_EXIT=%ERRORLEVEL%"
popd
if not "%TAURI_EXIT%"=="0" (
    echo.
    echo The Tauri CLI exited with code %TAURI_EXIT%.
)
exit /b %TAURI_EXIT%

:fail
echo.
echo Preparation stopped. Fix the prerequisite or build error above and run this file again.
exit /b 1
