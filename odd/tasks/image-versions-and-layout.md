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
| T0.1 | 0 | Oriented decoder module: `decode_oriented`, `decode_oriented_bytes`, `oriented_bytes`, orientation probe; tests for the eight orientations | delegated (writer trigger: new module + editors) | |
| T0.2 | 0 | The four editors in `image_edit.rs` decode through it; crop test on a 4000×3000 orientation-6 JPEG | delegated, with T0.1 | |
| T0.3 | 0 | Single ingress for asset image bytes (`ocr/mod.rs`, `processing/ocr.rs`, `block_reocr.rs`, `llm/openrouter.rs`) and every decode site listed in the plan | delegated | |
| T0.4 | 0 | Thumbnails: oriented, generator version in the hashed input, orphan cleanup | delegated | |
| T0.5 | 0 | Card and viewer open the same asset (C6) | delegated | |
| T1.1 | 1 | Migration `0064_derived_source_path` and the repo checklist | delegated | |
| T1.2 | 1 | Writers stamp `source_path` and `source_sha256` (individual OCR, queue, per-block, correction) | delegated | |
| T1.3 | 1 | Reader command: sha (lazy, cached by path/size/mtime), EXIF orientation, pixel dimensions | delegated | |
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
