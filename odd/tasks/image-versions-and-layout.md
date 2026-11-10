# Image versions and layout: derived data must match the image shown

Source plan (local, not tracked: `docs/` is gitignored): `docs/plan-versiones-imagen-y-layout.md`, closed on 2026-11-10 after three Judgment Day reviews. Base: `main` at `9c6f531d` (v1.5.0).

## Objective

The layout boxes and the OCR text of an image always correspond to the image version on screen, and editing a camera photo cuts the area the person selected.

## Problem

- Rust decodes images without applying the EXIF orientation; the WebView applies it. Crops land on another area, and layouts of tagged photos are in the sideways coordinate system.
- `layouts` and `extractions` do not record which file they were computed from, so nothing detects a layout that belongs to another version.
- The card and the viewer can pick different assets of the same item.

## Scope

In: PR 0 (EXIF orientation, same asset), PR 1 (migration 0064, writers, reader command), PR 2 (version guard, text warning, split view), PR 4 (versions panel, maintenance repair).

Out: PR 3 (sync; marked "to reevaluate" in the plan, not approved). Versioned annotations (D12 withdrawn). PDF crop behaviour (D11).

## Constraints

- PR 0, 1 and 2 ship in the same release.
- Never run `tauri dev` on the real archive with migration 0064; use `ENTROPIA_DEV_PROFILE`.
- Never stage `apps/desktop/src-tauri/Cargo.lock` (local patch) or `.gitignore` (someone else's edit).
- Original image files are never modified (D9).
- `source_sha256` is the hash of the raw bytes read from disk, before orientation.

## Delivery

Forecast: well over 400 authored lines. Strategy: one branch per plan PR, stacked in order (`fix/exif-orientation` → PR 1 → PR 2 → PR 4). The slicing comes from the plan itself. Push, PR creation and merge are the user's decisions.

## Tasks

| ID | PR | Task | Route | Commit |
| --- | --- | --- | --- | --- |
| T0.1 | 0 | Oriented decoder module: `decode_oriented`, `decode_oriented_bytes`, `oriented_bytes`, orientation probe; tests for the eight orientations | delegated (writer trigger: new module + editors) | 1ad252bf |
| T0.2 | 0 | The four editors in `image_edit.rs` decode through it; crop test on a 4000×3000 orientation-6 JPEG | delegated, with T0.1 | 1ad252bf |
| T0.3 | 0 | Single ingress for asset image bytes (`ocr/mod.rs`, `processing/ocr.rs`, `block_reocr.rs`, `llm/openrouter.rs`) and every decode site listed in the plan | delegated | | 1d320e1d + next |
| T0.4 | 0 | Thumbnails: oriented, generator version in the hashed input, orphan cleanup | delegated | | 89ab71a1 |
| T0.5 | 0 | Card and viewer open the same asset (C6) | delegated | | next |
| T1.1 | 1 | Migration `0064_derived_source_path` and the repo checklist | delegated | | 49c38cab |
| T1.2 | 1 | Writers stamp `source_path` and `source_sha256` (individual OCR, queue, per-block, correction) | delegated | | next |
| T1.3 | 1 | Reader command: sha (lazy, cached by path/size/mtime), EXIF orientation, pixel dimensions | delegated | | next |
| T2.1 | 2 | Version guard in one place; viewer draws no boxes and shows the notice with a re-run button | delegated | |
| T2.2 | 2 | Per-block re-OCR rejects, export omits, a layout from another version | delegated | |
| T2.3 | 2 | Text warning (D2) and confirmation before replacing hand-edited text (D10) | delegated | |
| T2.4 | 2 | `ItemView` listens to `asset:image-updated` for its own asset | delegated | |
| T4.1 | 4 | `list_image_versions` command and the versions panel | delegated | |
| T4.2 | 4 | Maintenance "review image versions": list, confirm, repair; fills source columns on old rows | delegated | |

## Acceptance criteria

The test lists under each PR in the source plan (RED first). Per task: `cargo test` for the touched Rust modules, `pnpm lint`, typecheck Pro and Lite (`VITE_LOCAL_ML=0`), `pnpm test`, `rustfmt`.

## Open checks (from the plan, section 7)

- Version guard scope for PDF page assets (`parent_asset_id`): settle in T2.1.
- What the WebView does with a PNG carrying EXIF orientation: needs the user's eyes on the running app.

## Log

- L1 (user, 2026-11-10): «implementar G:\EntropIA-Stack\EntropIA-Pro-Lite\docs\plan-versiones-imagen-y-layout.md».
- L2: branch `fix/exif-orientation` created from `main` at `9c6f531d`. Receipt-driven development: on (global).
- L3 (T0.1, T0.2): `image_orient.rs` with `decode_oriented`, `decode_oriented_bytes`, `exif_orientation`, `exif_orientation_bytes`, `oriented_bytes`; fixtures in `image_orient::test_support`. RED observed (5 of 6 decoder tests, 2 editor tests), GREEN: `cargo test --lib image_orient` 6 passed (parent re-ran), `image_edit` 20 passed, fmt and clippy clean. Review assessment: medium, `under_budget` (580 lines, mostly tests). Note: the path variant now guesses the format from content, so a mislabelled file decodes.
- L4 (T0.3, partial): ingress helper `image_orient::upright_asset_bytes(asset_type, raw)` wired at `processing/ocr.rs` (`read_asset_bytes`), `ocr/mod.rs` `process_job` (Pro, not compiled locally) and `llm/openrouter.rs` `from_validated_file`; `detect_image_dimensions` decodes upright. RED observed on 4 tests, GREEN: image_orient 8, ocr 302, openrouter 45, processing 226, fmt and clippy clean. PENDING: `ocr/block_reocr.rs:518` `render_page` must switch `image::load_from_memory` to `decode_oriented_bytes`, with its crop test (the file is under `ocr/`, the surface named the wrong path); then drop the `cfg_attr(allow(dead_code))` on `decode_oriented_bytes`. For T1.2: in Pro the queue reads an image twice (`processing/ocr.rs:226` and `ocr/mod.rs:1415`), so the writer hash must pick one read. `layout_onnx.rs` `detect` has no caller.
- L5 (T0.3 rest, T0.4): `ocr/block_reocr.rs` `render_page` decodes upright; thumbnails upright, `IMAGE_THUMBNAIL_GENERATOR_VERSION = 2` folded into the hashed text, name shape unchanged, superseded thumbnails of the asset removed by exact 64-hex match. RED observed on 4 of 5 tests. GREEN: image_orient 8, block_reocr 13, ocr::commands 13 (parent re-ran), sync::apply 44, fmt and clippy clean. Notes: (a) pre-existing `delete_image_thumbnail_files` matches by prefix only, so deleting asset `abc` also removes thumbnails of `abc-2` (not fixed, outside this task); (b) one cached thumbnail per asset now, so the T4.1 versions panel needs its own key or no cache.
- L6 (T0.5): one primary-asset rule (image, then PDF, then other; `sort_index`, `created_at`, `id`) in the card SQL (`item.repo.ts`), `AssetRepo.primaryOf` and `openingAssetIndex` (`asset-visibility.ts`); `ItemView` opens on it, and `CollectionView.refreshItemAssetMeta` uses it too (parent inline edit). Page strip order unchanged. RED observed; SQL tie-break mutation-checked. GREEN: store 469, desktop full 4014, typecheck Pro and Lite, lint, prettier. User-visible: a multi-asset item may open on a page other than 1. Not touched: `DocumentExplorer.svelte:239-249` still uses the first leaf.
- L7 (T1.1): branch `feat/derived-source-path` stacked on `fix/exif-orientation`. Migration `0064_derived_source_path` (four nullable columns), Drizzle schema, `AssetLayout` type, runner atomic list, Rust registry (totals 64, atomic 33), fixtures regenerated. RED observed on the TS runner, layout repo and Rust migration tests; the two sync tests had no RED. GREEN: store 475, desktop 4014, `cargo test --lib db` 128 (parent re-ran `db::migrations`, 15), `--lib sync` 658, fmt, clippy, lint, prettier. Sync needs no code change (`PRAGMA table_xinfo`). Known environmental failure, confirmed by the parent on `main`: `cargo test --lib automation` fails 69 of 169 with a hyper chunk-decoding error on loopback; unrelated to this work. Latent: `migrate_extractions_method_check` (`db/repairs.rs:218-232`) rebuilds `extractions` with six columns and `SELECT *`; only reachable on a legacy archive shape.
- L8 (T1.2, T1.3): `OcrComputeOutput.source` carries the stamp (path + sha of the raw bytes read) through the checkpoint to `publish_ocr_output`, the single publish path (individual OCR is admitted to the same queue). `save_extraction_row` and `save_layout_rows` set both columns in INSERT and `DO UPDATE SET` (None writes NULL). Per-block re-OCR stamps layout and extraction from the file it read. Text-only edits and OCR correction keep the stamp. PDFs are stamped too (path + sha of the whole file). Pro double read removed: `process_job` takes the bytes (`OcrJob.asset_path` gone) — NOT compiled locally. Reader: `derived_source::file_facts(path, with_sha)` and command `asset_file_facts(asset_id, with_sha)`, TS wrapper `lib/asset-file-facts.ts`; sha lazy, cached by path/size/mtime. RED observed on the queue and per-block tests; reader tests written with the code (cache mutation-checked). GREEN: `cargo test --tests --no-fail-fast` all targets green except the 69 known `automation::*`; parent re-ran bibliography_processing, derived_source, processing::ocr and the TS wrapper test; fmt, clippy, typecheck Pro/Lite, lint clean. Regression found by the parent and fixed: `tests/bibliography_processing.rs` builds `extractions`/`layouts` by hand without the 0064 columns, and the scheduler turns the resulting SQL error into a retry (`Waiting`). Same hand-written tables in `tests/bibliography_reprocess.rs:98-99` and `tests/processing_recovery.rs:71` (passing today). MUST ship with T2.2: a per-block re-OCR on an old unstamped layout now stamps it, which would make a stale layout look current unless the guard rejects it first. Plan correction: the read/write race does not exist in individual OCR either (same queue, `source_changed` gate).
