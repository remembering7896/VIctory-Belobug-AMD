@echo off
REM ============================================================
REM  VIctory - tuned run (RX 7900 XTX / 32 GB RAM)
REM
REM      scripts\run-tuned.bat [path\to\game_scarlett_master.exe]
REM
REM  Starts the title through scripts\play.ps1 with the settings that
REM  measured best on this machine. Everything here is a switch the
REM  build already knows; the memory patches live in the code (branch
REM  memory-trim-v2), the rest is configuration.
REM
REM  Measured (same scene, 6-10 min runs, p50 frame time):
REM      original                          59.0 ms   17.0 fps
REM      + memory patches                  58.3 ms   17.4 fps
REM      + renderer port patches (below)   46.2 ms   21.9 fps
REM
REM  The port patches are the ones README.md lists as off by default
REM  while they are tested. PORT_CBV removes about 22,000 constant-buffer
REM  fills a frame, PORT_GUARD takes the title threads' guard waits from
REM  29 ms to about 2 ms a frame, PORT_JOURNAL takes the submit thread's
REM  waits from about 6 ms to 0.3 ms.
REM
REM  The 30 Hz clock is kept on purpose: at VICTORY_FRAME_INTERVAL_MS=
REM  16.667 the menus reach 60 fps but the world gets slower (55.4 ms
REM  median instead of 46.2), because twice as many submissions queue on
REM  the one submitting thread.
REM ============================================================
setlocal
set "TREE=%~dp0.."
set "EXE=%~1"
if "%EXE%"=="" set "EXE=F:\x\GTA6.Xbox.v1.0.2.1\game_scarlett_master.exe"
if not exist "%EXE%" (
  echo No such executable: %EXE%
  echo Usage: scripts\run-tuned.bat [path\to\game_scarlett_master.exe]
  pause
  exit /b 1
)

set "MORE=@{VICTORY_TEXTURE_BUDGET_MB='8192'; VICTORY_UPSCALER='6'; VICTORY_PORT_CBV='1'; VICTORY_PORT_GUARD='1'; VICTORY_PORT_JOURNAL='1'; VICTORY_PORT_DESCRIPTORS='1'; VICTORY_PORT_CONSTANTS='1'; VICTORY_TEXTURE_STORE_KEPT_MB='2048'; VICTORY_TEXTURE_STORE_QUEUE_MB='512'; VICTORY_SCRATCH_SPARE_MB='256'; VICTORY_SEND_SPARE_MB='128'; VICTORY_STAGING_IDLE_MS='5000'; VICTORY_WINDOW_IDLE_MS='5000'; VICTORY_READ_KEEP_MB='64'; VICTORY_LIST_IDLE_TRIM_S='60'}"

powershell -NoProfile -ExecutionPolicy Bypass -Command "& '%TREE%\scripts\play.ps1' -Exe '%EXE%' -More %MORE%"
echo.
echo (Window can be closed now.)
pause
