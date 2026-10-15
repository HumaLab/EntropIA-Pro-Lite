# Resource audit fixtures

These are the two text stubs that used to live at
`src-tauri/resources/lib/linux-x86_64/` and were bundled by
`tauri.linux.conf.json`. They caused E-02: `dlopen` fails with "file too
short" on the 49-byte `libpdfium.so`, and `libonnxruntime.so` is 54 bytes.

They are kept only as audit provenance. They are NOT native libraries, must
never enter a bundle, and no build step reads them. The real Linux Pdfium is
downloaded by `apps/desktop/scripts/fetch-pdfium.sh linux-x64` into
`src-tauri/resources/pdfium/` (gitignored) and bundled from there by the
explicit release overlays `tauri.pro.linux.conf.json` and
`tauri.lite.linux.conf.json`. The auto-merged `tauri.linux.conf.json` does not
list it, so `cargo build` on Linux works without the download.

The Pdfium resolver in `src/ocr/pdf.rs` rejects any candidate that is not a
native shared object (ELF/PE/Mach-O/fat magic, at least 1 MiB), so even an
accidental copy of these stubs can no longer be resolved and bound.
