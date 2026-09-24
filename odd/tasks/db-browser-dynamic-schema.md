# DB browser: dynamic schema discovery

## Objective

The Database → DB query page lists every browsable table and view of the live
archive, discovered from SQLite on every visit, with no hand-kept list.

## Problem

`list_db_browser_tables` (`apps/desktop/src-tauri/src/db/commands.rs`)
intersected the live schema with a hardcoded `DB_BROWSER_CANDIDATE_TABLES`
array of 14 names. The live archive has 49 ordinary tables; 36 were invisible,
and every migration since the array was written had to be mirrored by hand.

## Audit (live `com.entropia.shared/entropia.sqlite`, 37/37 migrations applied)

- 61 `sqlite_master` tables, 0 views. `pragma_table_list`: 49 ordinary
  (incl. `sqlite_schema`/`sqlite_sequence`), 3 virtual (FTS5), 10 shadow (FTS5).
- Shown before: 13 (`vec_assets` included). Hidden on purpose: `app_settings`,
  `_migrations`, `fts_items`.
- BLOB values live in `rag_chunks.embedding` and `vec_assets.embedding`.

## Decisions

- Browsable = `pragma_table_list` rows of type `table` or `view` in `main`.
  Excluded: `sqlite_*` internals, FTS5 virtual tables and their shadow tables
  (derived search indexes), `app_settings` (API keys), names that fail
  `is_safe_identifier`.
- `_migrations` becomes visible: read-only, and it answers "which schema is
  this archive on".
- No cache: listing is re-queried on entry and on refresh; the backend
  allowlist is recomputed on every query, so a new table is accepted as soon as
  it exists.
- BLOB cells render as a size summary; copy and expand keep the Base64 value.

## Tasks

- [x] T1 Backend: discover tables/views from `pragma_table_list` (inline; one file)
  - RED: 2 new tests failed on the hardcoded list; GREEN: `cargo test --lib db::` 27 passed.
- [x] T2 Frontend: BLOB cell summary + refresh re-discovers schema, keeps selection (inline; context already loaded)
  - RED: 6 new tests failed; GREEN: desktop suite 151 files / 1924 tests passed.
- [x] T3 Parity check: live-DB list vs browser list; new-table appears without frontend change
  - `db_browser_matches_a_real_archive` (ignored, env-driven) on a copy of the live
    archive: 62 objects, 14 excluded, 48 listed = 48 expected; all 48 describe,
    sort desc on the last column, filter and page. New-table case covered by
    `db_browser_picks_up_a_table_created_after_startup`.

Gates: `cargo fmt --check`, `cargo clippy --lib --tests -D warnings`, prettier,
eslint, typecheck (Pro and `VITE_LOCAL_ML=0`) all clean. Visual check in the
running app confirmed by the user on 2026-09-22.

- [x] T4 Open on `extractions` by default (user request 2026-09-24): `pickInitialDbBrowserTable` (`lib/db-browser-view.ts`) picks `extractions` when browsable, else the first listed; a refresh still keeps the current table. RED 4; GREEN desktop 162 files / 2117; typecheck Pro+Lite, lint, format:check clean.
- [x] T5 Bug (user report 2026-09-24): under 900px the icon-only search and refresh buttons grew into ~200px squares. Cause: `.db-browser-toolbar__actions :global(.btn) { flex: 1 1 0 }`, from when they carried text, meets the icon-only Button's `aspect-ratio: 1` (made icon-only in `fa679f74`, 2026-08-28). Fix: the stretch rule removed, the row aligned to the end; source test guards it. RED 1; GREEN desktop 2118; typecheck Pro+Lite, lint, format:check clean. No other view has the same combination.
- [x] T6 Simpler filter, automatic reload (user decision 2026-09-24): no search or refresh buttons. The filter applies as you type (debounced; Enter immediate). Rows and schema reload on their own when the app announces a change: batch progress/completion (`batchStore`), document/page import or deletion (document-explorer events), sync applied (`syncStore`); coalesced, keeping page, sort and filter. Writes from outside the app are picked up on re-entering the view, as today.
  - Delegated `f42cff41`: `lib/db-browser-auto-reload.ts` (trailing debounce 600 ms, never overlapping, at most one queued follow-up); filter debounce 300 ms like TopBar, Enter immediate, `rowsRequestId` stale guard; sync reloads only when `last_sync_at` changes; first snapshots skipped. `batchStore` polls every 3 s only while a batch is active, so an idle archive never reloads on its own. RED 8 + 12; GREEN desktop 163 files / 2139; typecheck Pro+Lite, lint, format:check clean.
- [x] T7 Fixed header on scroll (user request 2026-09-24): page header, table toolbar (row count, JSON/CSV, rows per page) and column names stay visible; only the rows scroll. Approach: the table card fills the remaining height and scrolls itself, with sticky column headers inside it. Queued until T6 (same file) lands.
  - Inline: `.db-browser-view` height 100% (was min-height, so it grew with the rows and the page scrolled), `.db-browser-card` flex 1 / min-height 0, `.db-browser-table-wrap` flex 1 / min-height 12rem / overflow auto; the existing sticky `thead th` now sticks. The view is a direct child of `.content` (definite height). A top banner (Store update, missing deps) adds its own height of page scroll. RED 3 (after tightening a min-height false positive); GREEN desktop 163 files / 2142; typecheck Pro+Lite, lint, format:check clean.

## Checks

TDD: strict (session config). Runners: `cargo test --lib db::commands`,
`pnpm --filter @entropia-pro/desktop test -- src/lib/db-browser-view.test.ts src/views/DbBrowserView.test.ts`.

## Progress

- Done: T1–T3 and the visual check. Feature closed.
