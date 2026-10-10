# OCR pages rendered from one loaded document

Observed 2026-10-09 in tauri dev (debug, prueba-sync) during the real
bibliography reprocess: Abulafia 1950 (1537 pages, 420 GLM-OCR pages) advanced
~1 OCR page per minute with one CPU core at 100 % and no open TCP connection.
Per OCR page, `ProductionSelectiveOcr::render_page` (bibliography/processing.rs
~3745) calls `render_pdf_page_to_image` (ocr/pdf.rs ~2164), which reloads the
whole PDF in PDFium, renders the page at 2550 px wide and PNG-encodes it with
the pure-Rust `image` crate (unoptimized in dev builds). The batch is paused
with Abulafia at 25/420; paid pages are checkpointed. Branch
`fix/ocr-render-once`.

## Tasks

- [~] T1 (dropped) — Keep one loaded PDFium document across the selective OCR page loop
  (`maybe_ocr_pages`), so each OCR page only renders and encodes; byte-identical
  page images; cancel/checkpoint behaviour unchanged.
- [x] T2 — Optimize the image encode/resize crates in the dev profile (like
  `vecscan`), so debug builds do not spend minutes per page in PNG encoding.
- [ ] T3 — Resume the paused batch in tauri dev and measure the per-page rate
  until it finishes; check the result (candidates, Abulafia Texto tab).

## Evidence

- Measurement (worker, dev profile, Abulafia, 5 pages per mode):
  per-page load 5028 ms/page without T2, 909 ms with T2; one-load session
  4970 ms without T2, 851 ms with T2. The document reload costs ~60 ms/page.
- T1 dropped: correct (one load proven, byte-identical images) but ~1 % gain,
  while it needs `unsafe` lifetime widening and holds pdfium-render's global
  lock for the whole OCR run (hours), blocking the PDF viewer meanwhile. Patch
  kept outside the repo (agent-scratch/ocr-render-once-T1.patch).
- T2: Cargo.toml dev-profile opt-level 3 for image, png, fdeflate, flate2,
  miniz_oxide, adler2, crc32fast, simd-adler32 (all in Cargo.lock; lock file
  unchanged). Render+encode ~5.5x faster in dev.
- Open question for T3: the app measured ~60 s/page, the isolated render
  ~5 s/page, so most of the in-app time is elsewhere.
