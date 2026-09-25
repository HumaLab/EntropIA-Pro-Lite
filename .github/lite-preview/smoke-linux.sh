#!/usr/bin/env bash
# Smoke test for the EntropIA Lite .deb inside a fresh Ubuntu container
# (CI only, nothing is published).
#
# Installs the .deb through apt so apt has to resolve every runtime dependency,
# launches the installed binary under Xvfb with a D-Bus session, waits, and
# asserts it is still alive with no fatal stderr. Evidence (apt log, ldd,
# stdout/stderr, screenshot) lands in $OUT_DIR.
#
# Usage: smoke-linux.sh <path-to-deb> <out-dir>
set -euo pipefail

DEB="$(readlink -f "$1")"
OUT_DIR="$(readlink -f "$2")"
WAIT_SECONDS="${WAIT_SECONDS:-20}"
mkdir -p "$OUT_DIR"
export DEBIAN_FRONTEND=noninteractive

. /etc/os-release
echo "== $PRETTY_NAME / $(uname -m)" | tee "$OUT_DIR/host.txt"

apt-get update -q
# Smoke harness only: a virtual display, a session bus and a screenshot tool.
apt-get install -y -q --no-install-recommends xvfb xauth dbus dbus-x11 imagemagick procps ca-certificates

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
"$BIN" > "$OUT_DIR/app-stdout.log" 2> "$OUT_DIR/app-stderr.log" &
PID=\$!
sleep "$WAIT_SECONDS"
if kill -0 "\$PID" 2>/dev/null; then echo alive > "$OUT_DIR/liveness.txt"; else
  wait "\$PID"; echo "exited:\$?" > "$OUT_DIR/liveness.txt"; fi
import -window root "$OUT_DIR/screenshot.png" || echo "screenshot failed" >> "$OUT_DIR/liveness.txt"
kill "\$PID" 2>/dev/null || true
sleep 2
kill -9 "\$PID" 2>/dev/null || true
EOF
chmod +x /tmp/run-app.sh

dbus-run-session -- xvfb-run -a -s "-screen 0 1440x900x24" /tmp/run-app.sh

echo "--- liveness ---"; cat "$OUT_DIR/liveness.txt"
echo "--- app stderr ---"; cat "$OUT_DIR/app-stderr.log" || true
echo "--- app stdout ---"; tail -n 50 "$OUT_DIR/app-stdout.log" || true

if ! grep -qx alive "$OUT_DIR/liveness.txt"; then
  echo "::error::app exited within ${WAIT_SECONDS}s"
  exit 1
fi
if grep -Eq 'panicked at|fatal runtime error|Segmentation fault|core dumped|error while loading shared libraries' \
  "$OUT_DIR/app-stderr.log" "$OUT_DIR/app-stdout.log"; then
  echo "::error::fatal output from the app"
  exit 1
fi
echo "Linux smoke OK: alive after ${WAIT_SECONDS}s, no fatal output"
