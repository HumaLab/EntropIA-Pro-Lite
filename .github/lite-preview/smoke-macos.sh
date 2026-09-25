#!/usr/bin/env bash
# Smoke test for the EntropIA Lite macOS .dmg (CI only, nothing is published).
#
# Mounts the dmg, copies the .app into a throwaway Applications dir, strips
# quarantine, launches it, waits, and asserts the process is still alive and
# left no crash report. Evidence (stdout/stderr, unified log excerpt, lipo,
# otool, screenshot) lands in $OUT_DIR.
#
# Usage: smoke-macos.sh <path-to-dmg> <out-dir>
set -euo pipefail

DMG="$1"
OUT_DIR="$2"
WAIT_SECONDS="${WAIT_SECONDS:-20}"
mkdir -p "$OUT_DIR"

APPS_DIR="$(mktemp -d)/Applications"
MOUNT_POINT="$(mktemp -d)/lite-dmg"
mkdir -p "$APPS_DIR" "$MOUNT_POINT"

echo "== arch: $(uname -m) / $(sw_vers -productName) $(sw_vers -productVersion)" | tee "$OUT_DIR/host.txt"

hdiutil attach -nobrowse -readonly -mountpoint "$MOUNT_POINT" "$DMG"
APP_SRC="$(find "$MOUNT_POINT" -maxdepth 1 -name '*.app' | head -n 1)"
if [ -z "$APP_SRC" ]; then
  echo "::error::no .app inside $DMG"
  ls -la "$MOUNT_POINT"
  exit 1
fi
cp -R "$APP_SRC" "$APPS_DIR/"
hdiutil detach "$MOUNT_POINT"

APP="$APPS_DIR/$(basename "$APP_SRC")"
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

EXE_NAME="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP/Contents/Info.plist")"
EXE="$APP/Contents/MacOS/$EXE_NAME"
echo "app=$APP exe=$EXE" | tee -a "$OUT_DIR/host.txt"

lipo -archs "$EXE" | tee "$OUT_DIR/lipo-archs.txt"
otool -L "$EXE" | tee "$OUT_DIR/otool-L.txt"
# Anything linked outside /usr/lib, /System and the bundle itself would be
# missing on a clean machine.
NON_SYSTEM="$(otool -L "$EXE" | tail -n +2 | awk '{print $1}' \
  | grep -vE '^(/usr/lib/|/System/|@rpath/|@executable_path/|@loader_path/)' || true)"
if [ -n "$NON_SYSTEM" ]; then
  echo "::error::non-system dylibs linked: $NON_SYSTEM"
  exit 1
fi
codesign -dv "$APP" > "$OUT_DIR/codesign.txt" 2>&1 || true

START_TS="$(date '+%Y-%m-%d %H:%M:%S')"
open --stdout "$OUT_DIR/app-stdout.log" --stderr "$OUT_DIR/app-stderr.log" "$APP"

PID=""
for _ in $(seq 1 30); do
  PID="$(pgrep -f "$EXE" | head -n 1 || true)"
  [ -n "$PID" ] && break
  sleep 1
done
if [ -z "$PID" ]; then
  echo "::error::app process never appeared"
  exit 1
fi
echo "pid=$PID"

sleep "$WAIT_SECONDS"

ALIVE=true
kill -0 "$PID" 2>/dev/null || ALIVE=false

screencapture -x "$OUT_DIR/screenshot.png" || echo "::warning::screencapture failed"

log show --start "$START_TS" --style compact \
  --predicate "process == \"$EXE_NAME\" OR eventMessage CONTAINS[c] \"entropia\"" \
  > "$OUT_DIR/unified-log.txt" 2>&1 || true

CRASHES="$(find "$HOME/Library/Logs/DiagnosticReports" /Library/Logs/DiagnosticReports \
  -newermt "$START_TS" \( -iname "*${EXE_NAME}*" -o -iname '*entropia*' \) 2>/dev/null || true)"
if [ -n "$CRASHES" ]; then
  mkdir -p "$OUT_DIR/crash-reports"
  echo "$CRASHES" | while read -r f; do cp "$f" "$OUT_DIR/crash-reports/" || true; done
fi

osascript -e "tell application id \"com.entropia.lite\" to quit" 2>/dev/null || true
sleep 3
kill "$PID" 2>/dev/null || true

echo "--- app stderr ---"; cat "$OUT_DIR/app-stderr.log" || true
echo "--- app stdout ---"; tail -n 50 "$OUT_DIR/app-stdout.log" || true

if [ "$ALIVE" != true ]; then
  echo "::error::app exited within ${WAIT_SECONDS}s"
  exit 1
fi
if [ -n "$CRASHES" ]; then
  echo "::error::crash report(s) written: $CRASHES"
  exit 1
fi
if grep -Eq 'panicked at|fatal runtime error' "$OUT_DIR/app-stderr.log" "$OUT_DIR/app-stdout.log"; then
  echo "::error::panic in app output"
  exit 1
fi
echo "macOS smoke OK: alive after ${WAIT_SECONDS}s, no crash report"
