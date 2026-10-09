# Reprocess dialog progress

Owner-observed in tauri dev (prueba-sync, 2026-10-09): the preview counter only
advances per attachment, so a 1537-page PDF shows "Leyendo 0 de 1 adjuntos" for
5 min and looks hung; the candidates scan takes ~30 s with no progress and
cannot be cancelled. Branch `fix/reprocess-dialog-progress`.

Out of scope (decided): work-header button size (default size is the header
convention); pdf-extract `println!` noise (dev console only, release has no
console); real paid reprocess (owner runs it).

## Tasks

- [x] T1 — Preview progress within the current attachment. Thread a unit
  callback through the per-page native reading (lopdf pass, PDFium batches,
  whole-document extract) so the progress event carries work done inside the
  current attachment; the dialog shows it as a percentage next to the
  attachment counter. Cancel keeps working.
- [x] T2 — Candidates scan with progress and cancel. The scan reports
  attachments checked / total through an event and stops on a cancel flag; the
  dialog shows the count in the loading phase and Cerrar cancels the scan.
- [ ] T4 — Per-page progress and cancel inside the whole-document pdf-extract
  pass. Observed in T3: Abulafia (1537 pages) sits at 99 % for 124 s because
  the extract counts as one unit and cannot be cancelled. Wrap pdf-extract's
  PlainTextOutput in a delegating OutputDev that reports end_page and stops on
  cancel; units become 3 x pages; extracted text must stay byte-identical.
- [ ] T3 — Drive the dialog in tauri dev over CDP and confirm both behaviours
  with screenshots; never click confirm.

## Evidence

- T1: RED observed (3 frontend failures; 9 Rust compile errors against the old
  API). GREEN: `bibliography_reprocess` 27 passed / 3 ignored (binary run
  directly: the shared target's app exe is locked by the live tauri dev),
  `cargo test --lib` 2437 passed, clippy -D warnings clean, frontend 18 passed,
  typecheck 0 errors, lint and format:check clean.
- T1 commit: 7b88ebef.
- T2: RED observed (Rust unresolved imports; 5 frontend failures). GREEN:
  `bibliography_reprocess` 29 passed / 3 ignored (binary run directly),
  `acl_manifest_guard` 6 passed, clippy -D warnings clean, cargo fmt --check
  clean (also fixes T1 formatting drift in the test file), frontend 23 passed,
  typecheck 0 errors, lint and format:check clean. build.rs APP_COMMANDS gains
  the new command (ACL guard keeps build.rs, generate_handler! and the default
  capability in sync).
- T2 commit: 145824b5.
- T3 (partial, tauri dev over CDP): candidates count "Revisando 555 de 1704
  adjuntos", scan 28 s, close mid-scan closes the dialog; preview percent
  9/18/43 % inside the first attachment, cancel closes; Abulafia work mode
  climbs 6 -> 99 % in 173 s, then holds 99 % until 297 s (pdf-extract) -> T4.
