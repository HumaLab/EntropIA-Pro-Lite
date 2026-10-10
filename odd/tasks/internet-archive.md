# Internet Archive connector

## Objective

Search Internet Archive items and collections, preview their PDFs and import
them into new or existing Corpus collections with provenance, through the
existing importer, in both Lite and Pro.

Approved plan (local-only, `docs/` is gitignored): `docs/internet-archive/plan.md`;
Phase 0 evidence: `docs/internet-archive/auditoria-fase-0.md`. The plan went
through three Judgment Day passes (the last one approved) on 2026-10-10.

## Decisions

- 2026-10-10, Phase 0: job state lives in
  `<data_dir>/staging/internet-archive/<job>/job.json` (atomic writes); the
  connector adds no migration. Provenance key `__entropia_internet_archive`.
  Encrypted (`LCP`/`ACS`, `.lcpdf`) and `private` files are never offered. MVP
  collection browsing uses Advanced Search; the Scraping API is deferred.

- Rust owns network, staging and job state; the frontend imports each `verified`
  file with `importClassifiedPathsIntoCollection` (plan §4). No Rust port of the
  importer.
- Provenance is a compact record (≤ 64 KB) in `items.metadata`; job tables are
  local-only, never in `SYNCED_TABLES` (plan §5).
- MVP resume restarts an interrupted file from byte zero; no `Range`, no pause.
- Unbounded `lopdf` parsing of untrusted PDFs is an accepted MVP risk, bounded
  by the 500 MB per-file download cap; isolation is T6.6 (plan §7 point 3).

## Constraints

- Branch `feat/internet-archive`.
- Any `tauri dev` run with a new migration uses `ENTROPIA_DEV_PROFILE`; abort on
  `profile=shared` (plan §5, AGENTS.md).
- Phase 0 writes no product code and no database changes (plan §2 "Bloqueo").
- T0.3 smoke tests only issue small public GETs; no large downloads.

## Phase 0 tasks

- [x] 1. Inventory the importer, store schema, collections, job/queue mechanisms
      and Tauri permissions, including the `generate_handler!` / `APP_COMMANDS` /
      capability triple (T0.1).
- [x] 2. Document extension points and Lite/Pro differences (T0.2).
- [x] 3. Smoke-test Internet Archive endpoints: Advanced Search, Scraping API,
      Metadata API (`size` as string, `access-restricted-item`, `private`), and
      download redirect hosts (T0.3).
- [x] 4. Define the data mapping, multi-PDF policy, duplicates and permissions;
      fix the provenance key, confirm job tables stay out of `SYNCED_TABLES`, and
      decide whether job state needs a migration (T0.4, T0.6).
- [x] 5. Write the security checklist and test plan (T0.5).
- [x] 6. Write `docs/internet-archive/auditoria-fase-0.md` with real paths,
      reused functions, required migrations and risks; get the user's go-ahead
      for Phase 1.

## Evidence

- 2026-10-10, tasks 1-2: two read-only scouts mapped the importer
  (`collection-import.ts`, `file-import.ts`), store (`schema.ts`, `item.repo.ts`,
  `runner.ts`, latest migration `0058`), the durable processing queue, the Tauri
  command triple guarded by `tests/acl_manifest_guard.rs`, `reqwest` usage (no
  shared client, no redirect policy, no DNS resolver), `navegador/url_policy.rs`,
  data-dir/dev-profile resolution, `ImportSourcesDialog.svelte` (no source list),
  `ActionIcon`, `LOCAL_ML`, settings, i18n and test conventions (no HTTP mock
  crate; hand-rolled `test_server` in `navegador/zotero_copy/port.rs`). Details in
  the Phase 0 audit.
- 2026-10-10, task 3: live GETs against archive.org (small, public):
  - Advanced Search OK; `access-restricted-item` is requestable in `fl[]` (string
    `"true"`).
  - Metadata API: `size`/`mtime` are strings, some files have no `size`;
    `private: "true"` on lending PDFs; lending items also list `LCP Encrypted PDF`
    (`.lcpdf`) and `ACS Encrypted PDF` files that a format-first PDF rule would
    wrongly accept. Plan gap.
  - Download: `archive.org/download/...` 302 to `dn711501.ca.archive.org` (not only
    `ia*.us.archive.org`), `Accept-Ranges: bytes`, 206 on range, `%PDF-1.4`
    header. A `private` file redirects to `ia801501.us.archive.org` and ends in 403.
  - Scraping API returned identical items and totals for different queries
    (`collection:americana` and `mediatype:movies AND title:(chaplin)` both
    `total 4391810`, same first ids). Not reliable as observed. Plan gap.
- 2026-10-10, tasks 4-5: proposed decisions D1-D6 (provenance key
  `__entropia_internet_archive`; exclude encrypted PDFs; job state in
  `<data_dir>/staging/internet-archive/<job>/job.json` with no migration; collection
  browsing via Advanced Search, Scraping API deferred; `*.archive.org` redirect rule
  kept), security checklist and test plan written to the Phase 0 audit. Real API
  responses saved as fixtures under `docs/internet-archive/fixtures/` (local).
  Pending the user's approval (task 6).
- 2026-10-10, task 6: the user delegated D3; chose `job.json` (no migration, no
  `schema_tag` change, out of sync by construction). Phase 0 corrections applied to
  `docs/internet-archive/plan.md` (encrypted PDFs excluded, Advanced Search for
  collection browsing, `dn*.ca.archive.org` hosts, `job.json`, restricted marker
  from `fl[]`). Phase 0 closed; Phase 1 needs the user's go-ahead.
