# Internet Archive importer: layout, file-type filter, bulk selection

Objective: make the Internet Archive import dialog practical (more results visible, file-type filter, bulk selection) without changing the import pipeline's guarantees.
Branch: tracker `feat/ia-importer-ux` (draft, no-merge) + children `feat/ia-importer-ux-<n>-<slice>` · Delivery: `ask-on-risk`, chain: `feature-branch-chain` (user, 2026-11-12) · Runner: Vitest (`pnpm --filter @entropia-pro/desktop test -- <file>`), `cargo test -p entropia-internet-archive`.
Mockup: https://claude.ai/artifact/5XyJZgwi4BRyo9nr39nnAr (artboard "Propuesta").

## Specs

- **S1 — Wide dialog, dense list.** The IA dialog uses a wide layout (only this dialog; other `ConfirmDialog` users keep 440 px). Results render as one dense row each (title, creator/year, identifier) and the list uses the available height instead of `max-height: 240px`. Item files show beside the results (master–detail) when the width allows.
- **S2 — More results per page.** Default page size 50 instead of 10; selectable 25/50/100. Paging must never offer a page past the 10 000 deep-paging limit.
- **S3 — File-type filter.** User words: "permita elegir solo entre tres posibilidades, pdf, pdf+texto, imágenes (jpg, png, etc.), o sea, como filtro, si quiere bajar todo puede bajar los tres". Multi-select of `PDF`, `PDF + texto`, `Imágenes`; all three = everything. Applies to search, collection listing and the item's file list.
  - `PDF + texto` = only IA files with format `Text PDF`. User decision: "Solo el Text PDF" (the separate DjVuTXT is not downloaded).
  - `Imágenes` = JPEG/PNG/GIF/TIFF originals; searching must include `mediatype:image`.
  - Zero kinds selected disables search with a hint.
- **S4 — Bulk selection.** User words: "quiero que se puedan seleccionar y deseleccionar todos los documentos y que no sea obligatorio ir uno a uno". Header checkbox (checked / mixed / empty) and "Seleccionar todos" / "Deseleccionar todos" for the visible page, selecting every matching file of each checked item without opening it; restricted items are never selected. "Seleccionar los N resultados" across pages (bounded, with a volume notice).
- **S5 — App look.** User words: "mantener la estética de la app, no usar html crudo como el que muestra la barra deslizante blanca y gris rectangular sin bordes redondeados". Use `@entropia/ui` components (`Checkbox`, etc.) and the app scrollbar tokens; no unstyled native controls.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1, S2, S5 | inline | done | 7f950fb3 on `feat/ia-importer-ux-1-layout` (+585/−265, over budget: one cohesive dialog rewrite) |
| T2 | S3 (search + listing filter, crate + Tauri + TS) | inline | done | f37d7e12 on `feat/ia-importer-ux-2-kinds` (+483/−21); RED 7 failing → GREEN 4219 desktop tests |
| T3 | S3 (image files: list, download, verify, import) | inline | done | 21875827 on `feat/ia-importer-ux-3-images` (+672/−78); RED compile + 5 Tauri failures → GREEN 162 crate + 85 Tauri; Risk item 1 → independent verifier |
| T4 | S4 (page-level bulk select, file resolution in batch) | inline | done | on `feat/ia-importer-ux-4-bulk` (+275/−1); RED 3+1 → GREEN 4222 desktop + 963 ui |
| T5 | S4 (select all results across pages) | inline | done | on `feat/ia-importer-ux-5-all-results`; RED 2 → GREEN 4225 desktop; cap 500 records |

## Log

- L1 (2026-11-12, user): "quiero que hagas un test y pruebas para elaborar un plan de optimización en lo funcional y visual (hoy es poco practico y se ben pocos documentos por página) me refiero al importador vía api de Internet Archive a Colecciones de EntropIA"
- L2 (2026-11-12, user): "ante de arrancar quiero que también permita elegir solo entre tres posibilidades, pdf, pdf+texto, imágenes (jpg, png, etc.), o sea, como filtro, si quiere bajar todo puede bajar los tres."
- L3 (2026-11-12, user): "Solo el Text PDF"
- L4 (2026-11-12, user): "el boceto me gusta, solo dos cosas, una estetica general, mantener la estetica de la app no usar html crudo como el que muestra la barra deslizante blanca y gris rectangular sin bordes redondeados y queiro que se peuden seleccionar y desseleccioant dodos los docuemtnos y que no se aobligaotrio ir uno a uno"
- L5 (2026-11-12, user): "Sí, arrancá" — implement Phase 1 + type filter + bulk selection.
- L6 evidence: live API probe — `rows` 10/50/100 cost 1.86/1.44/1.30 s; `format:(PDF)` keeps 141 of 283 hits; `Text PDF` 140, `Image Container PDF` 19; `mediatype:image` 195 for the same title query.
- L7 (2026-11-12) User chose chain strategy: "Feature-branch chain". T1 branch renamed `feat/ia-importer-ux-1-layout`; tracker `feat/ia-importer-ux` reset to main (local only, nothing pushed). T2 committed f37d7e12. GIF stays out of the image Lucene group: the app does not import GIF (file-import.ts), pending note to user.
- L8 (2026-11-12) T3: IA publishes `__ia_thumb.jpg` as `source: original` with format "Item Tile"; images now require a declared format of exactly JPEG/PNG/TIFF (name extension only when no format). Kind derives from file_name+format, so job.json gains no field. Staging `<uuid>.{pdf,jpg,png,tif}`; older builds would reject a job holding image staging names (downgrade only).
- L9 (2026-11-12) T4/T5: row checkbox per record, page checkbox (checked/mixed/empty; shared Checkbox gained optional `indeterminate`), lookups 4 at a time; "Seleccionar los N resultados" appears once the page is fully selected, capped at 500 records with a notice. The page checkbox replaces separate "Seleccionar todos"/"Deseleccionar todos" buttons.
- L10 (2026-11-12) Independent verifier on T3: blocker (PDF display names stripped image suffixes, scan.tif -> scan.pdf) and advisory (extra image root made a complete PDF Partial) fixed in 4692f4b5 with tests; T4/T5 rebased (now 1d44133a, d7168da7). Native review T3 (lenses risk+reliability, user granted): approved, acknowledged, authority burned. Advisory follow-ups, not done: (1) is_complete image fallback ignores the expected kind, so a 1-page PDF item that lost its PDF root but holds a user image looks Complete; (2) BigTIFF (magic 43) is listed but always fails verification.
