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

## Phase 1 tasks (Rust client, no local writes, no downloads)

Standalone crate `crates/internet-archive` (precedent `crates/vecscan`), so it
builds and tests without Tauri's Linux system libraries; only the thin command
layer (task 11) lives in `src-tauri`. Decided 2026-10-10 with the user: rustup
1.90.0 installed in the user's home (`~/.cargo`, no PATH change). TDD per task
(observed RED, then GREEN). Fixtures: real responses from Phase 0 copied to
`crates/internet-archive/tests/fixtures/`. The crate re-implements the public-IP
predicates of `navegador/url_policy.rs` (it cannot depend on `src-tauri`);
unify later.

- [x] 7. HTTP client with a strict fetch policy (T1.1, T1.7): `https` and port
      443 only, host `archive.org` or `*.archive.org` (suffix boundary), manual
      redirects (max 5) validated hop by hop, public IPs only via the
      `url_policy.rs` predicates (exposed `pub(crate)`), resolved IP pinned for the
      connection, response body caps (16 MB metadata, 4 MB search) enforced while
      streaming. The policy is injectable so tests can target the local
      `test_server`; production code cannot build the relaxed policy.
- [x] 8. Advanced Search (T1.2): clause-based query builder with Lucene
      escaping, validated dates, empty or invalid filters rejected, `rows`
      clamped; request with `fl[]` including `access-restricted-item`; tolerant
      parsing of `numFound`/`docs`.
- [x] 9. Metadata API and PDF selection (T1.3, T1.4): tolerant DTOs (`size`/
      `mtime` as string, number or missing), PDF candidate rule D2 (exclude
      `Encrypted` formats, `.lcpdf`, `private: "true"`), cap of 1000 PDFs per item
      with a truncation flag, normalized output.
- [x] 10. Input resolution (T1.5): accept a bare identifier, an
      `archive.org/details/<id>` URL or a collection URL; reject anything else; no
      arbitrary endpoints.
- [x] 11. ~~Tauri commands `ia_search` and `ia_get_item`~~ moved to Phase 4
      (decided with the user 2026-10-10): `src-tauri` cannot compile on this Linux
      box (no Tauri system libraries) and its `Cargo.lock` must not be regenerated
      here (AGENTS.md `entropia-agent` git source line). All `ia_*` commands get
      registered together in Phase 4 and verified once on Windows (Lite, Pro,
      `acl_manifest_guard.rs`, `Cargo.lock`). Replaced by a manual live smoke test,
      `crates/internet-archive/examples/live_smoke.rs`.

## Phase 2 tasks (collections, manifests, job state; crate level)

Still in `crates/internet-archive`. The app-side pieces (SQLite provenance
lookup, data-dir wiring, commands) arrive in Phases 3-4 through a trait the
crate defines. MVP browsing is page by page with explicit selection, so no full
collection inventory is persisted (plan §1, D5).

- [ ] 12. Collection browsing (T2.1): page through a collection with Advanced
      Search, `collection:<id>`, stable `sort[]=identifier asc`, reusing the
      search builder and limits.
- [ ] 13. Job and manifest model (plan §4.2, T2.4): job and file states,
      validated transitions, idempotent terminal states, and the restart
      normalization rules (`running`→`ready`, `downloading`/`verifying`→`queued`,
      `importing`→`verified`, `cancelling`→`cancelled`). Pure, no I/O.
- [ ] 14. Job store (T2.2, T2.5, D3): `<staging>/<job_id>/job.json` with atomic
      writes (temp file, fsync, rename), a schema version, listing and
      reloading jobs after a restart, and corrupt files quarantined instead of
      crashing.
- [ ] 15. Size estimate and duplicate classification (T2.3, T2.4): totals with
      unknown sizes counted apart; per selected file `AlreadyInTarget`,
      `InOtherCollection`, `NewVersion` (same remote key, different fingerprint:
      md5, else sha1, else size+mtime) or `New`, against provenance records
      supplied through a trait.

## Evidence

- 2026-10-10, task 7: `crates/internet-archive` created (`FetchPolicy`,
  `IaClient::get_json`, `IaError`). RED: `cargo test` failed to compile (49
  errors, tests written first). GREEN: 13 passed. Independent verifier:
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (13/13, twice, identical), `cargo check --features test-support`, all exit 0;
  `target/` and `Cargo.lock` ignored. Parent review of host rule, redirect loop,
  address pinning and streaming cap: no issues. Follow-up: split `lib.rs` (756
  lines) into modules in task 8.
- 2026-10-10, task 8: `lib.rs` split into `policy`, `client`, `error`,
  `search`, `test_support` (13 prior tests unchanged and green), then Advanced
  Search added test-first. RED: 16 compile errors (missing `SearchQuery`,
  `Endpoints`, `InvalidQuery`, `search`). GREEN: 30 passed. Independent verifier:
  fmt, clippy `-D warnings`, `cargo test` 30/30 twice, no real network I/O in
  tests, changes confined to the crate. Parent review of `to_lucene`/`quote`: every
  user value quoted after escaping `\` and `"`; unfiltered queries refused.
- 2026-10-10, task 9: `IaClient::get_item` + `item.rs` (`ItemDetails`,
  `PdfFile`, `Availability`), real Phase 0 fixtures copied byte-identical.
  RED: 10 item tests failed against a stub parser. GREEN: 42 passed. Rule order:
  `Encrypted` > `Private` > `ItemRestricted` > `UnsafeName` > `Available`; the
  lending fixture yields three non-available PDFs (two `Encrypted`, one
  `Private`). Independent verifier after a forced re-lint: fmt, clippy, tests
  42/42 twice, `test-support` check, no prior test lost.
- 2026-10-10, task 10: `resolve_identifier` (pure; bare id, `/details/<id>`,
  `/download/<id>` on `archive.org`/`www.archive.org` only; never returns a URL)
  and `ItemDetails::mediatype` / `is_collection()` (collections share the item URL
  shape, so the mediatype decides). RED: accept tests failed against a stub,
  item tests failed to compile. GREEN: 47 passed. Independent verifier: fmt,
  clippy, 47/47 twice, `Cargo.toml` unchanged, no prior test lost.
- 2026-10-10, task 11 / Phase 1 close: `cargo run --example live_smoke`
  against the real archive.org with `FetchPolicy::production()` passed (search
  283 found; open item 1 PDF available; lending item 3 PDFs, none available, one
  or more `Encrypted`; details URL resolved; unknown id `NotFound`). fmt, clippy
  `-D warnings` and `cargo test` 47/47 green after the crate description update.
  Phase 1 acceptance (search and metadata correct, no local writes, no
  downloads, clear errors) met at crate level.

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
