# main CI repair after the audit stack merge

Merging the audit stack (#24–#42) left `main` red at 42c60e8d:

- `Lint & Typecheck`: `pnpm format:check` flags 7 files.
- `Windows Rust Feature Contract`: `tests/bibliography_processing.rs:6413`
  (`a_pdfium_recovered_page_beside_an_unreadable_one_keeps_the_baseline_ocr_candidates`)
  fails because pdf-extract 0.12 (D-08, #37) now reads text inside Form
  XObjects, which 0.7 did not. The test guards the OCR candidacy basis
  (`native_blank`), so the change may alter which pages go to OCR.

Branch: `fix/main-ci-after-audit-merge` (from `origin/main` 42c60e8d).

## Tasks

- [x] 1. Run prettier on the 7 flagged files.
- [x] 2. Measure the impact: does pdf-extract 0.12 change the OCR candidate list for the recovered-page fixture and for real Form XObject pages?
- [x] 3. Fix the test or the candidacy code, per the decision from task 2.
- [ ] 4. Open the PR to `main`, get CI green (including Windows), merge.
- [ ] 5. Rebase-free update of #44 onto the repaired `main`, CI green, merge.

## Evidence

- Task 1: `ee213882` — `pnpm format:check` passes locally.
- Task 2: not a production regression. `ocr_candidate_pages` (`processing.rs:3962`)
  makes an unreadable page a candidate only under a blank basis; pdf-extract 0.12
  genuinely reads the fixture's Form XObject text, so the basis turns non-blank and
  candidates go `[1, 2]` -> `[1]` (derived from code). Only the fixture's blindness
  premise broke. Decision (user): rebuild the fixture so pdf-extract and lopdf stay
  blind; fall back to rescoping the test to `[1]` if no construction works.
- Task 3: `5e32a832` — path A. The Form XObject now shows its text with the `'`
  operator: lopdf does not recurse into `Do`, pdf-extract 0.12 recurses but only
  handles `Tj`/`TJ`, PDFium reads it. Candidates stay `[1, 2]`.
  `cargo test --locked --test bibliography_processing`: 167 passed, PDFium ran (not skipped).
- Task 4 (in progress): PR #47 opened. CI `Lint & Typecheck` and `Build` pass. `Test`
  failed: `collection-load-cost.test.ts` imports `node:sqlite`, which vitest 4 (#40) cannot
  bundle in the happy-dom client environment on Node 22. Local Node 24 hid it. Fix:
  `@vitest-environment node` pragma, verified on Node 22.
