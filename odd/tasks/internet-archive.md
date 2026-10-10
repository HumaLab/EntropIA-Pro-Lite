# Internet Archive connector

## Objective

Search Internet Archive items and collections, preview their PDFs and import
them into new or existing Corpus collections with provenance, through the
existing importer, in both Lite and Pro.

Approved plan (local-only, `docs/` is gitignored): `docs/internet-archive/plan.md`;
Phase 0 evidence: `docs/internet-archive/auditoria-fase-0.md`. The plan went
through three Judgment Day passes (the last one approved) on 2026-10-10.

## Decisions

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

- [ ] 1. Inventory the importer, store schema, collections, job/queue mechanisms
      and Tauri permissions, including the `generate_handler!` / `APP_COMMANDS` /
      capability triple (T0.1).
- [ ] 2. Document extension points and Lite/Pro differences (T0.2).
- [ ] 3. Smoke-test Internet Archive endpoints: Advanced Search, Scraping API,
      Metadata API (`size` as string, `access-restricted-item`, `private`), and
      download redirect hosts (T0.3).
- [ ] 4. Define the data mapping, multi-PDF policy, duplicates and permissions;
      fix the provenance key, confirm job tables stay out of `SYNCED_TABLES`, and
      decide whether job state needs a migration (T0.4, T0.6).
- [ ] 5. Write the security checklist and test plan (T0.5).
- [ ] 6. Write `docs/internet-archive/auditoria-fase-0.md` with real paths,
      reused functions, required migrations and risks; get the user's go-ahead
      for Phase 1.

## Evidence
