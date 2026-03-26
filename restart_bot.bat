@echo off
setlocal enabledelayedexpansion
cd /d "%~dp0"
title TG_BOT_MANAGER
color 0A

REM ========================================
REM   TG Bot Manager (English Version)
REM ========================================

REM Check command line arguments for quick execution
if not "%~1"=="" (
    set "choice=%~1"
    goto process_choice
)

:menu
cls
echo.
echo ========================================
echo   TG Bot Manager
echo ========================================
echo.
echo [1] Quick Restart (Kill old and start)
echo [2] Safe Restart (Check env and start)
echo [3] Startup Check (Only check)
echo [4] Diagnose System
echo [5] Manual Start (Direct start)
echo [6] Kill All Python (Force clean)
echo [0] Exit
echo.
set /p choice="Select option [0-6]: "

:process_choice
if "%choice%"=="1" goto quick_restart
if "%choice%"=="2" goto safe_restart
if "%choice%"=="3" goto startup_check
if "%choice%"=="4" goto diagnose
if "%choice%"=="5" goto manual_start
if "%choice%"=="6" goto kill_all
if "%choice%"=="0" exit /b 0
goto menu

REM ========================================
REM Quick Restart
REM ========================================
:quick_restart
cls
echo.
echo ========================================
echo   Quick Restart Mode
echo ========================================
echo.
call :kill_processes
call :wait_cleanup
call :start_bot
goto end_restart

REM ========================================
REM Safe Restart
REM ========================================
:safe_restart
cls
echo.
echo ========================================
echo   Safe Restart Mode
echo ========================================
echo.
call :startup_checks
if errorlevel 1 (
    echo.
    echo [ERROR] Startup checks failed!
    pause
    goto menu
)
call :kill_processes
call :wait_cleanup
call :start_bot
goto end_restart

REM ========================================
REM Startup Check
REM ========================================
:startup_check
cls
echo.
echo ========================================
echo   Startup Check
echo ========================================
echo.
call :startup_checks
if errorlevel 1 (
    echo.
    echo [ERROR] Checks failed!
    pause
    goto menu
)
echo.
echo [SUCCESS] All checks passed!
pause
goto menu

REM ========================================
REM Diagnose
REM ========================================
:diagnose
cls
echo.
echo ========================================
echo   System Diagnosis
echo ========================================
echo.
python diagnose.py
pause
goto menu

REM ========================================
REM Manual Start
REM ========================================
:manual_start
cls
echo.
echo ========================================
echo   Manual Start
echo ========================================
echo.
call :startup_checks
if errorlevel 1 (
    echo.
    echo [ERROR] Checks failed!
    pause
    goto menu
)
echo [*] Starting bot...
python main.py
goto menu

REM ========================================
REM Kill All Python
REM ========================================
:kill_all
cls
echo.
echo ========================================
echo   Kill All Python Processes
echo ========================================
echo.
echo [WARNING] This will kill ALL python.exe processes!
set /p confirm="Continue? (y/n): "
if /i not "%confirm%"=="y" goto menu
call :kill_all_python
pause
goto menu

REM ========================================
REM Subroutine - Startup Checks
REM ========================================
:startup_checks
echo [*] Checking Python...
python --version >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Python not found!
    echo Please install Python 3.8+ from python.org
    exit /b 1
)
echo [OK] Python found

echo [*] Running startup checks...
python startup_check.py
if errorlevel 1 (
    exit /b 1
)
exit /b 0

REM ========================================
REM Subroutine - Kill Processes
REM ========================================
:kill_processes
echo [*] Killing old processes...

echo [*] Killing all python.exe (quick cleanup)...
taskkill /F /IM python.exe /T >nul 2>&1

REM Kill port 8080
echo [*] Checking port 8080...
for /f "tokens=5" %%a in ('netstat -aon 2^>nul ^| findstr ":8080"') do (
    if not "%%a"=="0" (
        echo [+] Killing process on port 8080: PID=%%a
        taskkill /F /PID %%a >nul 2>&1
    )
)

exit /b 0

REM ========================================
REM Subroutine - Kill All Python
REM ========================================
:kill_all_python
echo [*] Killing all python.exe processes...
taskkill /F /IM python.exe /T >nul 2>&1
echo [OK] Done
exit /b 0

REM ========================================
REM Subroutine - Wait Cleanup
REM ========================================
:wait_cleanup
echo [*] Waiting for cleanup...
timeout /t 3 /nobreak >nul
exit /b 0

REM ========================================
REM Subroutine - Start Bot
REM ========================================
:start_bot
echo [*] Checking Python environment...
set PY_CMD=python
if exist ".venv\Scripts\python.exe" (
    set PY_CMD=".venv\Scripts\python.exe"
    echo [OK] Using virtual environment
) else (
    echo [OK] Using system Python
)

echo [*] Starting bot...
start "TG_BOT_CORE" /MIN %PY_CMD% main.py
echo [OK] Bot started!

echo [*] Waiting for initialization...
timeout /t 5 /nobreak >nul

echo.
echo ========================================
echo   Bot restart complete!
echo   Management UI: http://127.0.0.1:8080
echo ========================================
echo.
exit /b 0

REM ========================================
REM End Restart
REM ========================================
:end_restart
REM If launched with argument (non-interactive), skip pause and exit
if not "%~1"=="" exit /b 0
pause
goto menu
