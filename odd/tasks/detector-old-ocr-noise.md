# Bibliography detector v2: old OCR noise and punctuation soup

After the real reprocess on prueba-sync (2026-10-10), Abulafia 1950 kept 911
native pages, most of them old-OCR garbage the v1 detector misses. Cause: rule 2
judges a page only with 40+ tokens of 4+ letters and counts only `.`, `~`, `·`
between letters; the garbage is punctuation soup with few whole words.
Branch `feat/detector-old-ocr-noise`.

Prototype (read-only on the prueba-sync archive, 18 984 native pages,
`agent-scratch/detector/proto3.py`):

| Rule | Abulafia flagged (of 911) | Other works flagged (of 18 073) |
|---|---|---|
| v1 | 42 | 56 |
| soup >= 15 % or noisy >= 15 % | 469 | 83 |
| soup >= 15 % or noisy >= 12 % | 543 | 99 |
| **soup >= 15 % or noisy >= 10 %** | **582** | **121 (52 works)** |
| soup >= 15 % or noisy >= 8 % | 617 | 162 |
| soup >= 15 % or noisy >= 6 % | 635 | 271 |

Manual review of other-work samples: mostly true positives (shifted glyph
fonts, spaceless text layers, unreadable bodies); false positives are code,
XML and some tables.

## Tasks

- [x] T1 — Detector v2 in Rust: punctuation-soup rule, broader old-OCR-noise
  rule with identifier exclusions and a lower judging gate, version bump to 2,
  unit tests from real samples (true and false positives).
- [ ] T2 — Measure v2 on a copy of the prueba-sync archive with the existing
  ignored measurement test; update odd/reports/native-text-detector-measurement.md.
- [ ] T3 — Preview in tauri dev: Abulafia is a candidate again; record the
  new cost. No paid confirm without the owner.
- [ ] T4 — Owner rule (2026-10-10): scanned paper books whose PDF carries a
  bad old-OCR text layer go to GLM-OCR automatically in the regular
  extraction, without a per-charge approval. Make the regular bibliography
  extraction send detector-flagged pages to GLM-OCR on its own (when a GLM-OCR
  key is configured), with tests; born-digital pages stay native. If T1's
  report shows it already does, record the evidence and only add the missing
  test.

## Evidence

- T1: RED observed (6 new detector tests failing against v1; 2446 existing
  tests green, so no v1 guard changed). GREEN: cargo test --lib 2452 passed,
  bibliography_reprocess 31 passed (receipt fixtures now follow
  BIBLIOGRAPHY_DETECTOR_VERSION), bibliography_processing 167 passed, clippy
  -D warnings and fmt clean.
- Regular extraction already sends detector-flagged pages to GLM-OCR
  automatically (worker report, read-only): extract_pdf_document ->
  maybe_ocr_pages(Automatic) processing.rs:3285/3300-3307 -> ocr_candidate_pages
  :3487 -> rich pages flagged by is_garbled_bibliography_text :3959-3968. No
  cost confirmation on this path (only reprocess has one). Gates: GLM-OCR key
  missing -> unit blocked, nothing charged (selective_ocr.rs:318-320); the
  paddle-ocr build OCRs locally. So detector v2 alone makes scanned books with
  bad old-OCR layers go to GLM automatically on extraction (owner rule, T4).
