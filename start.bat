@echo off
title Zen Browser - First Launch
cd /d "%~dp0"

:: Check if pywebview is installed
python -c "import webview" 2>nul
if errorlevel 1 (
    echo Installing dependencies...
    pip install -r requirements.txt
    echo.
)

python main.py
pause
