# Native text — part A: extract correctly going forward

Plan: `odd/plans/plan-texto-nativo.md` (revision 3, three Judgment Day rounds). The owner approved splitting
it on 2026-10-08:
- **Part A (this file):** extract correctly from now on. It does not change when anything goes to OCR.
- **Part B (later, own plan and judgment):** repair what is already stored (explicit reprocess, cost
  preview, retry rules, new detector).

Branch `feat/pdfium-page-text`, worktree `G:/EntropIA-Stack/EntropIA-Pro-Lite-worktrees/pdfium-page-text`.

## Scope rules

- No change to OCR candidacy, `extraction_is_settled`, `unresolved_empty`, the garbled detector or any
  re-demand rule.
- No change to `attachment_extraction_fingerprint`. Changing it would orphan in-flight paid OCR checkpoints
  at upgrade (JD5-A-002, JD5-B-003).
- No SQL migration.

## Tasks

- [x] A1 — PDFium resolution without the ML runtime. Add `ensure_pdfium_path_without_runtime`, which
  resolves the bundled library only and never calls `RuntimeManager::ensure_ready_or_bootstrap`. Use it:
  - in app setup;
  - before the bibliography page reader;
  - in place of `init_pdfium_path` in `ProductionSelectiveOcr::render_page`
    (`bibliography/processing.rs:3190`; JD5-A-001, JD5-B-002).

  Bundled candidates must cover the real installer layouts (JD5-B-005): Lite `resources/pdfium/…`,
  Pro Windows `resources/lib/pdfium.dll`, Pro Linux `resources/lib/linux-x86_64/libpdfium.so`, and dev
  `target/debug/resources/lib`. Where nothing is bundled (Pro macOS), log it and fall back to lopdf.
- [x] A2 — PDFium per-page reader. `read_native_page_texts` uses PDFium (`page.text().all()`), with lopdf
  as the per-page fallback.
  - Remove the soft-hyphen markers `\u{2}` and `\u{FFFE}` (and a following line break) so the word is
    joined.
  - Bound the text size with the existing per-page limit.
  - Read in batches of at most 20 pages per `Pdfium` instance, released between batches, and never alive
    while `maybe_ocr_pages` runs.
- [x] A3 — Document text is the union of the published pages (PDF only), replacing the
  `pdf-extract`/`richer_native_text` choice for stored bibliography text.
- [x] A4 — Delete page rows beyond the new `page_count` on publish. Count the deletion as a page move, so
  the profile is re-demanded and stale passages are dropped.
- [ ] A5 — Verify locally, CI green including Pro, judgment on the code, merge.

## Progress

- 2026-10-08: tasks opened.
- 2026-10-08: A1 done — `ensure_pdfium_path_without_runtime`(_dir) + superset bundled candidates
  (Lite/Pro Windows/Pro Linux/macOS/dev), wired into app setup, the extract reader and
  `ProductionSelectiveOcr::render_page`; scan + fake-layout tests green.
- 2026-10-08: A2 done — PDFium per-page reader in ≤20-page batches (instance per batch, released between
  batches; alive-counter test proves none is live when the OCR pass renders), lopdf per-page fallback for
  absent/empty/garbled/over-limit reads, soft-hyphen marker cleanup and the shared per-page size cap
  unit-tested.
- 2026-10-08: A3 done — stored PDF document text is now the published pages joined in order (`\n\n`);
  the OCR-candidacy input (`native_blank`) intentionally keeps its pre-A3 basis, per the scope rules.
- 2026-10-08: A4 done — `delete_page_texts_beyond` on publish; deletions count as page moves and
  re-demand the profile (3-page → 2-page replacement test green).
