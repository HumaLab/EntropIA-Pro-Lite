# PDF viewer performance — phase 1

Plan: `odd/plans/plan-pdf.md` (revision 4, approved for phase 1 by the owner on 2026-10-08 after two
Judgment Day rounds). Branch `feat/pdf-image-decoder`, worktree
`G:/EntropIA-Stack/EntropIA-Pro-Lite-worktrees/pdf-image-decoder`.

## Tasks

- [x] T1 — Shared `pdfDocumentOptions(url)` in `packages/ui`: `isImageDecoderSupported: true` only on
  Chromium >= 134 (userAgentData, then UA `Edg/NNN` / `Chrome/NNN`); unit tests.
- [x] T2 — `DocumentViewer.svelte` and `apps/desktop/src/lib/ocr-rich-text.ts` call `getDocument` through
  it; guard test fails on any other `getDocument` call.
- [x] T3 — Biblioteca only: Original tab stays mounted (hidden) in `BibliographyWorkView`; opt-in
  `DocumentViewer` prop suppresses renders while the container is 0×0 and renders once on re-show.
- [x] T4 — Visual safeguard: pixel-compare pages rendered with and without the option on a sample of the
  owner's real PDFs (incl. corpus, custom colour profile JPEG, masked/JPX pages); report differences.
- [ ] T5 — CI green (frontend), merge, owner measures section 6 targets in the app.

## Progress

- 2026-10-08: tasks opened.
- T4 (2026-10-08): 14 real PDFs from the owner's Zotero storage, 39 pages, picked by image feature from a
  scan of the 400 largest PDFs (285 pages with ICC-profile JPEG, 124 CMYK, 226 masked JPEG, 73 JPX). Each
  page rendered at 900 px height in headless Edge (Chromium 154), pdf.js 4.10.38, with and without
  `isImageDecoderSupported: true`, compared per pixel. Worst page: 0.078 % of pixels differ by more than
  16/255 (ICC JPEG, decoder rounding at photo edges); every CMYK, masked and JPX page is identical (pdf.js
  does not route them to `ImageDecoder`). No page above the 0.5 % review threshold, so no visual diffs were
  saved. Speed on the 128 MB PDF: page 6 8.6 s → 0.7 s, page 11 20.3 s → 0.8 s. Corpus (Colecciones) PDFs
  were not in the sample: the option only changes JPEG/BMP decoding, which this sample covers. Harness:
  `agent-scratch/pdfbench-web/compare.html`, `pdfbench-scripts/scan_features.py`, `pick_sample.py`; result
  `agent-scratch/pdfbench/compare-result.json`.
