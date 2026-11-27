@echo off
REM ============================================================
REM  VIctory - make the host pipelines ahead of a run
REM
REM      scripts\warm-tuned.bat              warm the pipelines earlier runs drew with
REM      scripts\warm-tuned.bat record       play once recording them, then warm
REM      scripts\warm-tuned.bat all          also retry the ones already made
REM
REM  Why: a pipeline the driver has not seen costs 2.7 ms at the median but
REM  533 ms at the p99 to make, and that lands as a single long frame in the
REM  game. Warming 3,780 recorded pipelines took 26 s and 82 s of compiler
REM  work here, and the run afterwards made none ("0 being made"), where it
REM  used to build up to 176 of them while playing.
REM
REM  The driver's own cache is what warm fills, so this has to be repeated
REM  after a driver update, a new build of the runtime, or a first visit to
REM  a part of the title no run has recorded yet - which is what "record"
REM  is for. A recording run is a normal run; it writes
REM  known-pipelines.bin and known-shaders.bin beside host-pipelines.bin in
REM  the cache folder (~250 MB).
REM
REM  Uses the same switches as scripts\run-tuned.bat, so the pipelines that
REM  go into the cache are the ones the tuned settings will ask for.
REM ============================================================
setlocal
set "TREE=%~dp0.."
set "MODE=%~1"
set "EXE=%~2"
if "%EXE%"=="" set "EXE=F:\x\GTA6.Xbox.v1.0.2.1\game_scarlett_master.exe"
set "VICTORY=%~dp0..\target\diag\victory.exe"

if not exist "%VICTORY%" (
  echo No %VICTORY% - build it first ^(README.md^).
  pause
  exit /b 1
)
pushd "%TREE%" || (echo cannot enter %TREE% & pause & exit /b 1)

if /i "%MODE%"=="record" (
  if not exist "%EXE%" (
    echo No such executable: %EXE%
    echo Usage: scripts\warm-tuned.bat record [path\to\game_scarlett_master.exe]
    popd & pause & exit /b 1
  )
  echo === 1/2  play once with VICTORY_PIPELINE_BOOK=1 so the pipelines drawn with are written down ===
  set "MORE=@{VICTORY_TEXTURE_BUDGET_MB='8192'; VICTORY_UPSCALER='6'; VICTORY_PORT_CBV='1'; VICTORY_PORT_GUARD='1'; VICTORY_PORT_JOURNAL='1'; VICTORY_PORT_DESCRIPTORS='1'; VICTORY_PORT_CONSTANTS='1'; VICTORY_TEXTURE_STORE_KEPT_MB='2048'; VICTORY_TEXTURE_STORE_QUEUE_MB='512'; VICTORY_SCRATCH_SPARE_MB='256'; VICTORY_SEND_SPARE_MB='128'; VICTORY_STAGING_IDLE_MS='5000'; VICTORY_WINDOW_IDLE_MS='5000'; VICTORY_READ_KEEP_MB='64'; VICTORY_LIST_IDLE_TRIM_S='60'; VICTORY_PIPELINE_BOOK='1'}"
  powershell -NoProfile -ExecutionPolicy Bypass -Command "& '%TREE%\scripts\play.ps1' -Exe '%EXE%' -More %MORE%"
  echo.
  echo === 2/2  make them ahead of the next run ===
)

if /i "%MODE%"=="all" (
  "%VICTORY%" warm --again
) else (
  "%VICTORY%" warm --used
)
set "WARM_EXIT=%ERRORLEVEL%"

echo.
if "%WARM_EXIT%"=="0" (
  echo Done. The next run makes no pipeline the driver has not already seen here.
) else (
  echo warm exited with %WARM_EXIT% - see the lines above.
)
echo Pipelines that crash the compiler are listed in the cache folder's pipelines\warm-bad.txt
echo and skipped until the driver or the translator changes.
popd
echo.
echo (Window can be closed now.)
pause
