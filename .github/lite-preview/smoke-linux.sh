#!/usr/bin/env bash
# Smoke test for the EntropIA Lite .deb inside a fresh Ubuntu container
# (CI only, nothing is published).
#
# Installs the .deb through apt so apt has to resolve every runtime dependency,
# checks its identity (package, binary and launcher are EntropIA Lite's, and no
# installed path collides with the Pro .deb) and that the bundled Pdfium loads
# and renders a PDF, then launches the installed binary under Xvfb with a D-Bus
# session, asserts the main window is visible within VISIBLE_DEADLINE seconds,
# waits, and asserts it is still alive with no fatal stderr. Evidence (apt log,
# ldd, stdout/stderr, window timing, screenshots) lands in $OUT_DIR.
#
# Usage: smoke-linux.sh <path-to-deb> <out-dir>
set -euo pipefail

DEB="$(readlink -f "$1")"
OUT_DIR="$(readlink -f "$2")"
WAIT_SECONDS="${WAIT_SECONDS:-45}"
# The splash holds for 5 s (splash.rs MIN_VISIBLE); the main window must follow
# right after, well before the 20 s watchdog.
VISIBLE_DEADLINE="${VISIBLE_DEADLINE:-12}"
# Keep watching past the deadline, up to the watchdog, so a miss reports the
# real reveal time instead of just "late".
WATCH_SECONDS="${WATCH_SECONDS:-25}"
# Early frame: taken at this point, it must already show the UI.
EARLY_SECONDS="${EARLY_SECONDS:-7}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$OUT_DIR"
export DEBIAN_FRONTEND=noninteractive

. /etc/os-release
echo "== $PRETTY_NAME / $(uname -m)" | tee "$OUT_DIR/host.txt"

apt-get update -q
# Smoke harness only: a virtual display, a session bus and a screenshot tool.
apt-get install -y -q --no-install-recommends xvfb xauth dbus dbus-x11 imagemagick procps ca-certificates xdotool python3

# The package under test. apt fails here if any Depends: cannot be resolved.
apt-get install -y "$DEB" 2>&1 | tee "$OUT_DIR/apt-install.log" \
  || { echo "::error::apt could not install $DEB"; exit 1; }

PKG="$(dpkg-deb -f "$DEB" Package)"
dpkg-deb -f "$DEB" > "$OUT_DIR/deb-control.txt"
dpkg -L "$PKG" > "$OUT_DIR/deb-files.txt"
BIN="$(grep -E '^/usr/bin/[^/]+$' "$OUT_DIR/deb-files.txt" | head -n 1)"
if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  echo "::error::no executable under /usr/bin in package $PKG"
  exit 1
fi
echo "package=$PKG bin=$BIN" | tee -a "$OUT_DIR/host.txt"

# Identity: Lite must install side by side with Pro. The Pro .deb owns
# /usr/bin/entropia-pro-desktop, /usr/lib/EntropIA Pro/, "EntropIA Pro.desktop",
# the entropia-pro-desktop icons and /usr/share/doc/EntropIA Pro/ (tauri.conf.json
# productName + the cargo binary name, laid out by tauri-bundler's debian.rs).
PRO_PATHS_RE='^(/usr/bin/entropia-pro-desktop|/usr/lib/EntropIA Pro(/.*)?|/usr/share/applications/EntropIA Pro\.desktop|/usr/share/icons/hicolor/[^/]+/apps/entropia-pro-desktop\.png|/usr/share/doc/EntropIA Pro(/.*)?)$'
{
  echo "package: $PKG"
  echo "binary: $BIN"
  echo "--- paths shared with the Pro .deb:"
  grep -E "$PRO_PATHS_RE" "$OUT_DIR/deb-files.txt" || echo "(none)"
} | tee "$OUT_DIR/identity.txt"
[ "$PKG" = "entropia-lite" ] || { echo "::error::package is '$PKG', expected entropia-lite"; exit 1; }
[ "$BIN" = "/usr/bin/entropia-lite" ] || { echo "::error::binary is '$BIN', expected /usr/bin/entropia-lite"; exit 1; }
if grep -qE "$PRO_PATHS_RE" "$OUT_DIR/deb-files.txt"; then
  echo "::error::the Lite .deb installs paths the Pro .deb owns"
  exit 1
fi
DESKTOP="$(grep -E '^/usr/share/applications/[^/]+\.desktop$' "$OUT_DIR/deb-files.txt" | head -n 1)"
cp "$DESKTOP" "$OUT_DIR/launcher.desktop"
cat "$OUT_DIR/launcher.desktop"
grep -qx 'Name=EntropIA Lite' "$DESKTOP" || { echo "::error::launcher Name is not EntropIA Lite"; exit 1; }
grep -qx 'Exec=entropia-lite' "$DESKTOP" || { echo "::error::launcher Exec is not entropia-lite"; exit 1; }
if grep -E '^(Description|Package|Source):' "$OUT_DIR/deb-control.txt" | grep -q 'EntropIA Pro'; then
  echo "::error::package metadata still says EntropIA Pro"
  exit 1
fi
if dpkg-deb -f "$DEB" Description | grep -qE 'EntropIA Pro|run on local models'; then
  echo "::error::package description still describes Pro"
  exit 1
fi

# Pdfium: the app resolves it at <resource dir>/resources/pdfium (src/ocr/pdf.rs),
# i.e. /usr/lib/<productName>/resources/pdfium for a .deb install.
PDFIUM="$(grep -E '^/usr/lib/[^/]+/resources/pdfium/libpdfium\.so$' "$OUT_DIR/deb-files.txt" | head -n 1)"
if [ -z "$PDFIUM" ]; then
  echo "::error::the .deb ships no Pdfium under resources/pdfium"
  exit 1
fi
python3 "$HERE/pdfium-probe.py" "$PDFIUM" 2>&1 | tee "$OUT_DIR/pdfium-probe.txt"
[ "${PIPESTATUS[0]}" -eq 0 ] || { echo "::error::bundled Pdfium failed to render"; exit 1; }

ldd "$BIN" | tee "$OUT_DIR/ldd.txt"
if grep -q 'not found' "$OUT_DIR/ldd.txt"; then
  echo "::error::unresolved shared libraries after apt install"
  exit 1
fi

# Containers do not allow the user namespaces WebKitGTK's bubblewrap sandbox
# needs, and Xvfb has no GPU; both are properties of this harness, not of a
# user's desktop.
export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export LIBGL_ALWAYS_SOFTWARE=1
export NO_AT_BRIDGE=1

cat > /tmp/run-app.sh <<EOF
#!/usr/bin/env bash
START=\$(date +%s)
"$BIN" > "$OUT_DIR/app-stdout.log" 2> "$OUT_DIR/app-stderr.log" &
PID=\$!
# The main window's title is "EntropIA Lite"; the splash's is "EntropIA".
VISIBLE_AFTER=""
while [ "\$(( \$(date +%s) - START ))" -le "$WATCH_SECONDS" ]; do
  for W in \$(xdotool search --onlyvisible --name '^EntropIA Lite\$' 2>/dev/null); do
    WIDTH=\$(xdotool getwindowgeometry --shell "\$W" | sed -n 's/^WIDTH=//p')
    if [ "\$WIDTH" -ge 800 ]; then VISIBLE_AFTER="\$(( \$(date +%s) - START ))"; fi
  done
  [ -n "\$VISIBLE_AFTER" ] && break
  sleep 0.5
done
echo "main window visible after: \${VISIBLE_AFTER:-never within $WATCH_SECONDS}s" > "$OUT_DIR/window-visible.txt"
xdotool search --onlyvisible --name 'EntropIA' >> "$OUT_DIR/window-visible.txt" 2>&1 || true
REMAIN=\$(( $EARLY_SECONDS - (\$(date +%s) - START) )); [ "\$REMAIN" -gt 0 ] && sleep "\$REMAIN"
import -window root "$OUT_DIR/screenshot-early.png" || true
REMAIN=\$(( $WAIT_SECONDS - (\$(date +%s) - START) )); [ "\$REMAIN" -gt 0 ] && sleep "\$REMAIN"
if kill -0 "\$PID" 2>/dev/null; then echo alive > "$OUT_DIR/liveness.txt"; else
  wait "\$PID"; echo "exited:\$?" > "$OUT_DIR/liveness.txt"; fi
import -window root "$OUT_DIR/screenshot.png" || echo "screenshot failed" >> "$OUT_DIR/liveness.txt"
kill "\$PID" 2>/dev/null || true
sleep 2
kill -9 "\$PID" 2>/dev/null || true
EOF
chmod +x /tmp/run-app.sh

dbus-run-session -- xvfb-run -a -s "-screen 0 1440x900x24" /tmp/run-app.sh

echo "--- window ---"; cat "$OUT_DIR/window-visible.txt"
echo "--- liveness ---"; cat "$OUT_DIR/liveness.txt"
echo "--- app stderr ---"; cat "$OUT_DIR/app-stderr.log" || true
echo "--- app stdout ---"; tail -n 50 "$OUT_DIR/app-stdout.log" || true

VISIBLE_AFTER="$(sed -n 's/^main window visible after: \([0-9]\+\)s$/\1/p' "$OUT_DIR/window-visible.txt")"
if [ -z "$VISIBLE_AFTER" ] || [ "$VISIBLE_AFTER" -gt "$VISIBLE_DEADLINE" ]; then
  echo "::error::main window not visible within ${VISIBLE_DEADLINE}s ($(head -n 1 "$OUT_DIR/window-visible.txt"))"
  exit 1
fi
if grep -q 'never signalled readiness' "$OUT_DIR/app-stderr.log"; then
  echo "::error::the splash watchdog revealed the main window (frontend readiness never arrived)"
  exit 1
fi
if ! grep -qx alive "$OUT_DIR/liveness.txt"; then
  echo "::error::app exited within ${WAIT_SECONDS}s"
  exit 1
fi
if grep -Eq 'panicked at|fatal runtime error|Segmentation fault|core dumped|error while loading shared libraries' \
  "$OUT_DIR/app-stderr.log" "$OUT_DIR/app-stdout.log"; then
  echo "::error::fatal output from the app"
  exit 1
fi
echo "Linux smoke OK: $(head -n 1 "$OUT_DIR/window-visible.txt"), alive after ${WAIT_SECONDS}s, Pdfium renders, no fatal output"
