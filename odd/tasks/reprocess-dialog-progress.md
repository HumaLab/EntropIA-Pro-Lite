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
- [ ] T2 — Candidates scan with progress and cancel. The scan reports
  attachments checked / total through an event and stops on a cancel flag; the
  dialog shows the count in the loading phase and Cerrar cancels the scan.
- [ ] T3 — Drive the dialog in tauri dev over CDP and confirm both behaviours
  with screenshots; never click confirm.

## Evidence

- T1: RED observed (3 frontend failures; 9 Rust compile errors against the old
  API). GREEN: `bibliography_reprocess` 27 passed / 3 ignored (binary run
  directly: the shared target's app exe is locked by the live tauri dev),
  `cargo test --lib` 2437 passed, clippy -D warnings clean, frontend 18 passed,
  typecheck 0 errors, lint and format:check clean.
