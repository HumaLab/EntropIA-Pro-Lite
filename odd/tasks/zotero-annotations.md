# Zotero annotations in the Biblioteca

Implements `docs/zotero-annotations-plan.md` Revision 8 (blob 7eaf6e58, gitignored local
doc; it is the authoritative design). Branch `feat/zotero-annotations`. Engram mirror:
`odd/zotero-annotations/tasks`.

## Specs

- **S1** Goal, verbatim from the plan: "PDFs of the Zotero library shown in the Biblioteca can be highlighted, underlined and annotated (notes, areas, text) inside EntropIA, with EntropIA's look (reusing the Collections `DocumentViewer` / `AnnotationToolbar` and the "Documento activo" panel style), and every annotation syncs in both directions with Zotero."
- **S2** Data model and state invariants: plan section 3 (tables `zotero_annotation_items`, `zotero_annotation_sync_state`; invariants; local mutations, including in-flight keep and never-sent delete). Excluded from EntropIA device sync (plan section 2).
- **S3** Pull: plan section 4 and the 4.1 merge table, including reconcile-first for uncertain rows.
- **S4** Push: plan section 5 (drainer, create/update/delete, acknowledgement, uncertain outcome and startup recovery, error policy including "parent item not found" disambiguation).
- **S5** Conflict lifecycle: plan section 6 (6.1, 6.2 with `conflict_json` for "zotero" decisions, 6.3 traces).
- **S6** Wire mapping, coordinates and sort index: plan sections 7 and 8.
- **S7** Viewer: plan section 9 (PdfMarkup contract, `markups` / `allowedTools` / `palette` / `textSelection`, TextLayer identity, pointer routing, selection popover via `ActionIcon`, trashed annotations hidden). Collections behavior unchanged.
- **S8** Right panel "Anotaciones": plan section 10 (list by page, sync badges, trash group with Restore, conflict UI).
- **S9** Conventions and verification: plan section 11 (i18n es/en, zero warnings, both variants, every listed Rust and TS test, Collections regression suites unchanged). Credentials stay in Rust (plan section 2).

## Tasks

| ID | Specs | Route | Commit |
|---|---|---|---|
| T1 Data model: migration 0059, `schema.ts`, Rust registry, schema fixtures | S2, S9 | inline | 3f0b49be |
| T2 Rust sync core (pure): row model, invariants, local mutations, 4.1 merge, acknowledgement, uncertain reconciliation, conflict resolution, wire mapping; all plan traces as unit tests | S2-S6 | inline | a7dd7f6d |
| T3 Rust transport + repository + engine: Web API client, pull, drainer, error policy, startup recovery, per-library lock, triggers | S2-S4, S9 | inline | a7dd7f6d, a6b35d5f |
| T4 Tauri commands, ACL registration, frontend `lib` wrapper | S4, S5, S9 | inline | a6b35d5f, 99f658fd |
| T5 packages/ui viewer: PdfMarkup, new props, TextLayer, popover, coordinate and sort-index helpers | S6, S7, S9 | inline | a327617e, e9b68804 |
| T6 Biblioteca integration: viewer wiring, "Anotaciones" panel, conflict UI, i18n | S1, S7, S8, S9 | inline | 1055c5bd, 2e37f22f |
| T7 Full checks (lint, typecheck both variants, vitest, cargo test, clippy) and independent verification | S1-S9 | verify | job-6 all green |
| T9 Finish the plan's remaining items: faithful sort index, text layer in the render request, post-catalog-sync trigger, area/ink previews, live tests, dev-profile note | S3, S6-S9 | inline | 7e46403d, 78a10794, 48a697a5, 4ac3f17e, 07d22077, 72e8aaba |
| T8 Fix the verify findings (in-flight race, atomic rekey, strict replies, trash during conflict, trash visibility, error policy, missing-PDF panel, color edit, key redaction) | S2-S8 | inline | 72d4b5cf, 0d6a0aa3 |

Risk: item 1 (new migration and stored data), item 2 (API key use in a new writer).

## Log

- **L1** (user, 2026-10-27): "metele con todo, me voy a dormir, espero despertarme y que esté todo implementado"
- **L2** Context: the plan went through Judgment Day on v1-v7; the user decided after v7 to implement Revision 8 and cover remaining state-machine edge cases with the plan's traces as tests.
- **L3** Exploration findings: `deps/mod.rs` / `mod_lite.rs` are dependency-manager modules, not command lists; new commands go in `lib.rs` `generate_handler!`, `build.rs` `APP_COMMANDS` and `capabilities/default.json` (guarded by `tests/acl_manifest_guard.rs`). Rust migrations: `src-tauri/src/db/migrations.rs` registry with `include_str!` of the store SQL. Schema fixtures: `export-schema`, `export-schema-canonical`. HTTP test server: `navegador::zotero_copy::port::test_server`.
- **L4** The three parallel `gentle-ai-worker` launches (T1, T2, T5) failed at once with "assistant reported an error" after a few turns. Not retried; every task ran inline in the parent.
- **L5** Gotchas found: the store migration runner splits statements on `;` even inside SQL comments; the Rust test `atomic_kinds_match_the_runner_atomic_condition` hardcodes the Atomic count (now 28); a child `$effect` that reports an element to a parent which stores it in `$state` must `untrack` the callback or it loops.
- **L6** Deferred inside this feature: the `catalog_sync` trigger after `bibliography_sync` (the work-open pull, edit pushes and the panel's sync button cover the flow); area/ink thumbnails in the panel; live Zotero fixtures for the sort index (plan section 8) and the opt-in live tests (section 11).
- **L7** Independent verify (gentle-ai-verify) found: engine applied answers to the stale sent copy (overwrote edits made while a request was out); rekey not atomic; malformed replies read as definitive; remote trash during a field conflict merged as a field; trashed rows with pending edits shown; per-item POST codes, parent-read errors, 503 and backoff growth mishandled; panel gone without the local PDF; no color edit; server text unredacted. All fixed in T8 with tests (engine race tests use a second connection on a file archive).
- **L8** Still open, deliberately: sort index splits text-item widths evenly and is not pinned to an upstream zotero/reader commit (plan section 8 live fixtures); the text layer is built by its own effect keyed on (attachment, page, scale), not inside the page-render request; the post-`bibliography_sync` trigger, area/ink thumbnails and the opt-in live tests (section 11) are not implemented. `db/repairs.rs` migration count bumped to 59.
- **L9** Final checks after T8: store 456/456; ui lint 0, typecheck 0, 913/913; desktop lint 0, Pro 3758 passed/9 skipped, Lite typecheck 0, 3722 passed/45 skipped; cargo test --lib 2770 passed/14 ignored; ACL and async-lock guards green; clippy -D warnings clean.
- **L10** (user): "encargate de todo, hace todo lo que falte para temrinar de implementar los cambios"
- **L11** T9 done. Sort index ported literally from zotero/reader@0b16e731 (`selection.js`, `utilities.js`): the reader picks its "top-most" rect by x2 and measures top from the page height; glyph boxes come from the rendered text layer, falling back to evenly split text items. The rotation branch of `getPositionBoundingRect` is unreachable for sort indexes and was not ported. The text layer is built under the page render's request id; a stale request cancels it. Catalog syncs trigger an annotation pull from the scheduler's commit observer. Area/ink previews render from the viewer's open document (cached per revision, two at a time). Live tests (`ZSB_LIVE_ZOTERO_WRITE=1`, test group 6680944) passed 4/4 and confirmed the client-key POST contract; a first run left 4 orphan fixture books (group libraries reject linked files), deleted with a title+tag filtered cleanup, and the fixture now cleans up on partial failure. The dev profile still writes to the real Zotero account (documented in AGENTS.md).
- **L12** Not verified: no interactive smoke test in the real app (plan section 11 smoke list) — needs a person at the UI under a dev profile; the commit observer trigger has no automated test (needs a Tauri AppHandle).
- **L13** (user): "ese alguien frente a la app sos vos, ya lo haz hecho, peudes tomar control d ela PC y abrir y testear la app, hazlo"
- **L14** Smoke test run in the real app: Lite `tauri dev`, dev profile `zotero-smoke`, driven through WebView2's CDP port with Playwright, every result checked against Zotero's Web API. No full catalog sync (it would queue OCR extraction over the whole library); one real work (Griffin 2003, attachment 2B5N2EKH) was seeded into the profile archive, and the Zotero key reused through a `secret_ref` without touching the keyring. Passed: migration 0059 on a fresh archive; key verification; first pull (168 annotations); text selection, popover and highlight placed on the text; comment push; Zotero-side edit pulled; a real comment conflict shown and resolved; note, area (with preview), text; move with sort index recomputed; recolor; Zotero trash hidden and restored; delete. Three defects found and fixed: the markup toolbar did not take pointer events (81706399); a pull of our own last write turned a later delete into a false conflict (e3d33aea); the markup toolbar pushed the zoom column onto the page (f72279e2). All test annotations deleted (5 keys 404, the PDF has 0 children on the server).

