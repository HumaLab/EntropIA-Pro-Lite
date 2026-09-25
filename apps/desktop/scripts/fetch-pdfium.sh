#!/usr/bin/env bash
# Downloads the Pdfium shared library the macOS and Linux Lite bundles ship,
# pinned by release tag and archive sha256, into src-tauri/resources/pdfium/.
#
# Source: bblanchon/pdfium-binaries, release chromium/7543 — the same release as
# the committed Windows src-tauri/resources/lib/pdfium.dll (byte-identical to
# that release's pdfium-win-x64.tgz bin/pdfium.dll). Bump all platforms together.
#
# Usage: fetch-pdfium.sh <mac-univ|linux-x64>
set -euo pipefail

TAG="chromium/7543"
case "${1:-}" in
  mac-univ)
    ASSET="pdfium-mac-univ.tgz"
    SHA256="5484abb015b38e7b2598f0ac94c6fb889b5bc12f29dd5660693c8e8d8e57bcb1"
    LIB="libpdfium.dylib"
    ;;
  linux-x64)
    ASSET="pdfium-linux-x64.tgz"
    SHA256="9329a3c4b19b3c8d0a93af5440f44be84e4bd879a204e47b1a7a160e96809da4"
    LIB="libpdfium.so"
    ;;
  *)
    echo "usage: $0 <mac-univ|linux-x64>" >&2
    exit 2
    ;;
esac

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$HERE/../src-tauri/resources/pdfium"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

curl -fsSL --retry 3 -o "$WORK/$ASSET" \
  "https://github.com/bblanchon/pdfium-binaries/releases/download/${TAG}/${ASSET}"

if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL="$(sha256sum "$WORK/$ASSET" | awk '{print $1}')"
else
  ACTUAL="$(shasum -a 256 "$WORK/$ASSET" | awk '{print $1}')"
fi
if [ "$ACTUAL" != "$SHA256" ]; then
  echo "::error::$ASSET sha256 mismatch: expected $SHA256, got $ACTUAL" >&2
  exit 1
fi

tar -xzf "$WORK/$ASSET" -C "$WORK" "lib/$LIB" LICENSE
mkdir -p "$DEST"
cp "$WORK/lib/$LIB" "$DEST/$LIB"
cp "$WORK/LICENSE" "$DEST/LICENSE"
echo "pdfium $TAG ($ASSET, sha256 $SHA256) -> $DEST/$LIB"
