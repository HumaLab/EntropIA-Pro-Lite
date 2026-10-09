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
    (`bibliography/processing.rs:3190`; JD5-A-001, JD5-B-002), with one render-only fallback: where
    nothing is bundled, an ALREADY-HYDRATED managed runtime copy of the library may be used
    (`ensure_pdfium_path_with_hydrated_runtime`, JD6-B-001) — resolved from disk as it already is,
    never by bootstrapping.

  Bundled candidates must cover the real installer layouts (JD5-B-005): Lite `resources/pdfium/…`,
  Pro Windows `resources/lib/pdfium.dll`, Pro Linux `resources/lib/linux-x86_64/libpdfium.so`, and dev
  `target/debug/resources/lib`. Where nothing is bundled (Pro macOS), log it and fall back to lopdf.
  The runtime-free resolver probes only the host's own OS/arch layouts (JD6-A-005) — a
  foreign-architecture library must never shadow a host-compatible one — while the shared
  `bundled_pdfium_candidate_paths` used by `init_pdfium_path` (corpus/Pro) keeps its pre-part-A
  candidate list and order.

  The no-bootstrap guarantee covers **PDFium resolution only**. The Pro local Paddle OCR path
  resolves its models through `resolve_paddle_model_dir` → `managed_runtime_root_for_ocr`, which does
  call `RuntimeManager::ensure_ready_or_bootstrap`; that is pre-existing behavior and outside part A
  (JD6-A-002).
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
- [x] A5 — Verify locally, CI green including Pro, judgment on the code, merge.

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
- 2026-10-08: Judgment Day 6 regressions fixed test-first. Per-page choice now prefers PDFium only
  where its read is at least as complete as lopdf's in alphanumeric content (ties to PDFium for its
  spacing) or lopdf's is glued/garbled — zero-size runs and other dropped PDFium runs no longer lose
  text (JD6-A-001); `native_blank` is computed again on pdf-extract + the lopdf rows, so a recovered
  page beside an unreadable one keeps the baseline OCR candidates (JD6-A-003, JD6-B-004); the
  bomb-safe bound is decided per page from the lopdf decompressed-content check before any PDFium
  read, which also caps its strings per batch (JD6-A-004); the bundled resolver probes only the
  host's OS/arch layouts while the corpus candidate list keeps its pre-part-A shape (JD6-A-005);
  the page render falls back to an already-hydrated managed runtime copy without bootstrapping
  (JD6-B-001); an empty page union no longer flips a rich pdf-extract text to `empty` (JD6-B-002);
  the PDFium-gated tests fail on a missing library under `ENTROPIA_REQUIRE_PDFIUM=1` (set in the
  Windows CI test leg) and the batch-lifecycle test proves PDFium really ran (JD6-A-006,
  JD6-B-003); the Pdfium instance counters are thread-scoped so parallel tests cannot move each
  other's assertions (JD6-A-007); the off-page text policy is pinned (JD6-B-005).
- 2026-10-08: A5 done — local verification green (2402 unit, bibliography_processing 158 twice in parallel, clippy, fmt, navegador check, ENTROPIA_REQUIRE_PDFIUM=1); CI run 37846732953 green incl. Pro; merged to main.
