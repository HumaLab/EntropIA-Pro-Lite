#!/usr/bin/env bash
# Builds the llama-server binary backing local GLM-OCR from the same llama.cpp
# revision the app links (llama-cpp-sys-2 0.1.145), into
# src-tauri/resources/tools/llama-server/ (flat, like resources/pdfium/: CI
# checks out one platform per job, so no per-platform subdirectories).
#
# The output is a build artifact, never committed (see .gitignore): ~220 MB of
# CUDA shared libraries cannot live in git. Installers pick it up through the
# `resources/tools/llama-server/*` entries in the tauri.*.conf.json overlays.
#
# Pinned revision: e21cdc11a0461d8b0cbd28cc356d993bf6be7282. It matches the
# vendored sources (CMakeLists.txt and ggml/src/ggml.c are byte-identical to
# the registry copy); the script re-checks that and refuses to build anything
# else. Bump together with the llama-cpp-2 crate.
#
# Usage: fetch-llama-server.sh <linux-x64> [--cpu]
#   Default is a CUDA build (GGML_CUDA=ON, needs CUDA toolkit + ~5 min).
PLATFORM="${1:-}"

CPU_ONLY=0
if [[ "${2:-}" == "--cpu" ]]; then
  CPU_ONLY=1
fi

case "$PLATFORM" in
  linux-x64) ;;
  *)
    echo "usage: $0 <linux-x64> [--cpu]" >&2
    exit 2
    ;;
esac

LLAMA_COMMIT="e21cdc11a0461d8b0cbd28cc356d993bf6be7282"
TARBALL_URL="https://codeload.github.com/ggml-org/llama.cpp/tar.gz/${LLAMA_COMMIT}"

for tool in cmake curl; do
  command -v "$tool" >/dev/null 2>&1 || { echo "::error::$tool is required" >&2; exit 1; }
done
if [[ "$CPU_ONLY" -eq 0 ]]; then
  command -v nvcc >/dev/null 2>&1 || { echo "::error::nvcc is required for the CUDA build (or pass --cpu)" >&2; exit 1; }
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"
DEST="$HERE/../src-tauri/resources/tools/llama-server"

curl -fsSL --retry 3 -o "$WORK/llama.tgz" "$TARBALL_URL"
mkdir -p "$WORK/src"
tar -xzf "$WORK/llama.tgz" -C "$WORK/src" --strip-components=1

# Refuse to build a different revision than the app links.
REGISTRY_LLAMA=(~/.cargo/registry/src/*/llama-cpp-sys-2-0.1.145/llama.cpp)
if [[ -d "${REGISTRY_LLAMA[0]}" ]]; then
  for f in CMakeLists.txt ggml/src/ggml.c; do
    cmp -s "${REGISTRY_LLAMA[0]}/$f" "$WORK/src/$f" || {
      echo "::error::$f differs from the vendored llama.cpp; refusing to build $LLAMA_COMMIT" >&2
      exit 1
    }
  done
  echo "revision check: $LLAMA_COMMIT matches the vendored llama.cpp"
else
  echo "warning: cargo registry copy not found, skipping revision check" >&2
fi

CMAKE_ARGS=(-B "$WORK/build" -DGGML_NATIVE=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF)
if [[ "$CPU_ONLY" -eq 0 ]]; then
  CMAKE_ARGS+=(-DGGML_CUDA=ON)
fi
cmake -S "$WORK/src" "${CMAKE_ARGS[@]}" >"$WORK/cmake.log" 2>&1
cmake --build "$WORK/build" --target llama-server -j "$(nproc)" >"$WORK/build.log" 2>&1

mkdir -p "$DEST"
cp -P "$WORK/build/bin/llama-server" "$WORK"/build/bin/lib*.so* "$DEST"/
rm -f "$DEST"/lib*.so 2>/dev/null || true
# Keep versioned .so files; the unversioned dev symlinks must not ship.
(cd "$DEST" && for f in lib*.so; do [[ -L "$f" ]] && rm -f "$f"; done) || true

echo "llama-server ($PLATFORM, $([[ "$CPU_ONLY" -eq 0 ]] && echo cuda || echo cpu)) -> $DEST"
(cd "$DEST" && sha256sum llama-server lib*.so.* | awk '{print $2": "$1}')
