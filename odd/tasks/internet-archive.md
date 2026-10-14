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

- [x] 12. Collection browsing (T2.1): page through a collection with Advanced
      Search, `collection:<id>`, stable `sort[]=identifier asc`, reusing the
      search builder and limits.
- [x] 13. Job and manifest model (plan §4.2, T2.4): job and file states,
      validated transitions, idempotent terminal states, and the restart
      normalization rules (`running`→`ready`, `downloading`/`verifying`→`queued`,
      `importing`→`verified`, `cancelling`→`cancelled`). Pure, no I/O.
- [x] 14. Job store (T2.2, T2.5, D3): `<staging>/<job_id>/job.json` with atomic
      writes (temp file, fsync, rename), a schema version, listing and
      reloading jobs after a restart, and corrupt files quarantined instead of
      crashing.
- [x] 15. Size estimate and duplicate classification (T2.3, T2.4): totals with
      unknown sizes counted apart; per selected file `AlreadyInTarget`,
      `InOtherCollection`, `NewVersion` (same remote key, different fingerprint:
      md5, else sha1, else size+mtime) or `New`, against provenance records
      supplied through a trait.

## Phase 3 tasks (download and Corpus association)

Decided 2026-10-10 with the user: Tauri's Linux system libraries are now
installed (via `pkexec`), so `src-tauri` builds and tests here in Lite. Always
`cargo ... --locked` so `Cargo.lock` keeps the `entropia-agent` git source line.

- [x] 16. T3.0 bounded PDF split (blocking prerequisite): a new streaming split
      in `src-tauri/src/ocr/pdf.rs` that writes each page as soon as it is
      produced, with `SplitLimits { max_output_bytes, max_duration }`, removing
      partial output and returning no pages when a limit is hit; the command
      `split_pdf_pages` uses it with defaults (min of 2 GB and free space minus a
      reserve; time limit checked between pages); its signature and the TS
      importer contract are unchanged. The existing in-memory
      `split_pdf_to_single_page_bytes` stays as is for `processing/ocr.rs`.

## Phase 3 remaining crate tasks (download and verify; no Tauri commands)

T3.0 is done in `src-tauri`. The download and verification stay in
`crates/internet-archive` so they test here. The app wires the data dir and
commands in Phase 4.

- [x] 17. Streaming download (T3.2, §7 points 1, 3, 5, 6, 9): GET
      `/download/{identifier}/{file}` through the existing fetch policy, write to
      a caller-supplied staging path ending in `.pdf`, refuse to overwrite an
      existing file, stream with a hard cap (default 500 MB, absolute max 2 GB,
      never disabled), abort with marker `archivo demasiado grande`, delete the
      partial on any failure, no `.part` kept. Bounded retries for transient
      failures; never retry 401/403/404. MVP resume is a full re-download by the
      caller.
- [x] 18. Verification (T3.3, §7 point 7): `%PDF` header, remote md5 or sha1 when
      published, always local SHA-256, page count via `lopdf` (accepted parse
      risk, input already capped), reject over the page cap with
      `demasiadas páginas`, unreadable PDF with `verificación fallida`. Result
      carries `sha256` and `page_count` for later confirmation.
- [x] 19. Display-name sanitizer (§7 point 1): last path segment, same rules as
      `navegador-copy.ts` `safeStem`, always `.pdf`, fallback to the identifier.

## App integration tasks (src-tauri, Lite, built and tested on Linux)

Decided 2026-10-10 with the user: the `ia_*` commands move back from Phase 4
because `src-tauri` now builds here. Windows keeps only the final build and the
Pro variant. Command logic lives in plain functions that take their
dependencies, so it is unit-tested without a running app; the `#[tauri::command]`
wrappers stay thin. Every command goes into `generate_handler!`, `APP_COMMANDS`
and `capabilities/default.json` at once. `Cargo.lock` may only gain the
`internet-archive` entry; the `entropia-agent` source line must stay.

- [x] 20. Wire the crate: path dependency, `src/internet_archive/` module with
      shared state (production client, `JobStore` at
      `<data_dir>/staging/internet-archive`), read-only commands `ia_search`,
      `ia_list_collection`, `ia_get_item`, `ia_resolve_identifier`.
- [x] 21. (21a crate: `ItemSnapshot` kept in the job, `build_provenance` /
      `parse_provenance`; 21b app: lookup and command.) Decisions: Rust builds
      the compact provenance record and the frontend passes it unchanged as
      `extraMetadata`; a job always targets an existing collection (the
      frontend creates a new one first with the existing store service).
      SQLite `ProvenanceLookup` over `items.metadata` key
      `__entropia_internet_archive`, compact provenance record (≤ 64 KB, T3.8),
      and `ia_prepare_import` (creates a `Draft` job and returns its summary).
- [x] 22a. Download timeout defect (found while planning 22): downloads share
      the 30 s whole-request timeout of the JSON calls, so any file that takes
      longer than 30 s to transfer fails. Downloads must use the connect timeout
      plus an inactivity (read) timeout, per plan §7 point 4; JSON calls keep
      the total timeout.
- [ ] 22. (Decisions: files download one at a time in the MVP, a deviation
      from the plan's default of 2, to keep the runner simple and the load on
      Archive low; a cancel interrupts the current download; 429 waits for
      `Retry-After`, capped at 60 s, up to 3 times; a job stays `running` while it
      has `verified` files waiting for the frontend import; `ia_list_jobs` added so
      the UI can offer "Retomar importación".) [x] Job runner: `ia_start_import`, `ia_cancel_import`,
      `ia_get_import_status`, `ia_list_verified_files`; one job at a time;
      download, verify, record `sha256` and `page_count`; `recover_all` at
      startup.
- [ ] 23. (Decisions: Rust owns the dedup re-check and the completeness rule.
      `ia_begin_file_import` answers `import` with the payload,
      `confirm_existing` for a complete same-version item already in the
      target, or `discard_partial` for an incomplete one the frontend must
      remove with `discardFailedImport` before asking again;
      `ia_fail_file_import` marks a failed frontend import `failed` and frees
      staging; `ia_list_reconcile` / `ia_finish_reconcile` close the
      cancelled-while-importing case. Complete = one parent `pdf` asset without
      parent plus exactly N child assets with page numbers 1..N, all files on
      disk.) [x] `ia_confirm_file_imported`: validate against SQLite that the item
      exists, sits in the target collection, carries matching provenance, and is
      complete (parent asset plus pages 1..N with files on disk); otherwise
      reject and keep the file `verified` with its staging.

## Phase 4 tasks (frontend)

Precedents: `lib/navegador-sources.ts` (thin `invoke` wrappers, error codes),
`lib/navegador-copy.ts` (orchestration over the importer),
`views/NavegadorCopyDialog.svelte` (destination picker),
`views/ImportSourcesDialog.svelte` (entry point). Tests run with
`VITE_LOCAL_ML=0` and `1`. Visual check needs the user, under
`ENTROPIA_DEV_PROFILE`.

- [x] 24. TS client `lib/internet-archive.ts`: typed wrappers for the 15
      `ia_*` commands, `parseIaError` for `<code>: <message>`, and a helper
      for the `ia://job-updated` event.
- [x] 25. Import orchestrator `lib/internet-archive-import.ts`: for each
      verified file, begin, then import through
      `importClassifiedPathsIntoCollection` (or confirm an existing item, or
      discard a partial one with `discardFailedImport`, exported unchanged),
      then confirm or report failure; cancel checked between files; reconcile
      leftovers from a cancelled job.
- [x] 26. (Split: 26a the dialog component, its states and texts; 26b entry
      points and startup reconcile. MVP imports only: no post-processing option,
      plan §9 default; limits use their defaults, no settings UI yet.)
      `views/InternetArchiveDialog.svelte`: search, collection and link
      tabs, item files with availability, destination (existing or new
      collection), preparation summary (duplicates, sizes, synced volume),
      progress with cancel, results; entry points in `ImportSourcesDialog` and
      `CollectionView`; texts in `i18n.ts` (es and en).
- [x] 28. Review follow-ups (advisory R3-001..003 on the frontend slice):
      reconcile entries become durable and derived on demand from `job.json`
      (a `cancelled` file that still has a `staging_name`), so startup timing
      cannot lose them; cancel paths clear `staging_name` when they delete
      staging; the startup reconcile retries on failure; the dialog ignores
      stale `iaGetItem` and status responses.
- [ ] 27. Variant checks: typecheck, lint and desktop tests with
      `VITE_LOCAL_ML=0` and `1`; then the user's visual check. Status 2026-10-11: still OPEN. The corrected Lite MSI PASS preserved the IA library (4 collections and 14 documents, existing dummy-pdf_2 readable), but task 27 still needs Pro with Rust Pdfium integration, additional IA cases, and the child-asset DB size follow-up (DB reads 0 vs 1373 bytes on disk).

## Evidence

- 2026-10-10, task 27, real Lite GUI acceptance: delegated to
  `gentle-ai-verify` using the authorized project-only Codex override.
  Started with `VITE_LOCAL_ML=0`, `ENTROPIA_DEV_PROFILE=ia-gui-4b85e95c39e7`
  and `pnpm exec tauri dev --config src-tauri/tauri.lite.conf.json --no-watch
  -- --locked`. Startup proved the exact isolated profile and `sync=disabled`
  before interaction. Live title search returned 329 results; linked public
  `dummy-pdf_2` and imported its 7,478-byte, one-page PDF into a new isolated
  collection: 1 imported, 0 errors. Opened and visibly rendered the page
  (parent spot-checked the screenshot). Read-only SQLite/disk checks confirmed
  provenance, parent PDF, exact page-1 child and matching size/SHA-256.
  Repeating into the same collection returned 0 imported, 1 already present,
  0 errors; item/asset IDs and hashes stayed unchanged. SQLite `quick_check`
  returned `ok`. Owned app/server processes stopped. Evidence and screenshots:
  `/tmp/ia-gui-4b85e95c39e7/`; isolated data preserved under
  `~/.local/share/com.entropia.shared/dev-profiles/ia-gui-4b85e95c39e7`.
  Warnings: search results can grow the dialog beyond the viewport, clipping
  header/footer; native Pdfium resource loading logged an invalid/short library,
  although visible page rendering succeeded. No behavior fixes made.
  Pro/Windows, collection-tab browsing, multi-page/multi-file imports,
  cancellation/recovery and restricted/encrypted real-item rejection remain
  unverified; task 27 is not closed. No source/lockfile/generated tracked changes
  from the run; only the authorized verifier configuration and this evidence.

- 2026-10-11, task 27 Windows Lite actual (second VM d9e70790-1faa-488b-a23a-a622d99cb202, Lite 1.0.20 NSIS, no Pro/MSI/builds/commits): real UI PASS. Enlace dummy-pdf_2 into fresh IA Windows Test: 1/0/0; preparation 1 file 7.3 KB vol 14.6 KB; opened page shows Dummy PDF download. Repeat same target: 0/1/0; preparation 0 files Duplicados Ya importado vol 0 B; same collection 62af3181-da91-4a8d-a746-84db0ff4990d and item e929e121-aa95-4700-b028-358580b680ac with unchanged hashes. Public fixture https://archive.org/details/dummy-pdf_2, 7478 bytes 1 page, parent 29550d778741b8ba2ed38d4e21b7bbc97d84925eec24422a55f8a4d5b1eb9a43, child page1 1373 bytes 756a1f4cd5dcf449a897b40688f46edb48fd14998f6ba9f73f8871305ed7a87c. Jobs 4e1a2a97-e5c6-42a4-9237-0dce2f519ce3 and ceeb30ad-d652-468f-bee1-521fcb8a2321 completed under %APPDATA% com.entropia.shared/staging/internet-archive. Native DLL C:\Users\EntropiaTest\AppData\Local\EntropIA Lite\resources\lib\pdfium.dll 5787136 SHA256 6B963C2BE9CACBAA0C0C7F4BF6D20D2FD16729EBDAA9989978B0F7B119C1C1CB x64 (MZ 77/90, PE 120, machine 100/134); FPDF API NOTVERIFIED and app Pdfium integration NOT tested (viewer is pdf.js). SQLite NOTVERIFIED (no CLI). Screenshots second VM shots/w4-ia-*, w4-lib/hash/mz/machine/parent/child/job*. Task 27 still not closed.

- 2026-10-11, independent verification of the Windows Lite run (no new imports): IA images confirmed 1/0/0 then 0/1/0 with unchanged collection/item/jobs and parent 7478 / child 1373 hashes matching fixtures, viewer Dummy PDF. Native direct-API render on the installed DLL verified PASS (FPDF_Init/LoadDocument/LoadPage nonzero, PageCount 1, 300x425 render with 227 nonwhite pixels; Bitmap/Close/Destroy ok) via audited /NativeProbe.cs on the read-only probe ISO; this proves installed-DLL API, not Rust app integration/pdf.js. MSI/Pro/Rust integration/SQLite/other cases remain open.

- 2026-10-11, Windows Lite SQLite readonly PASS (second VM d9e70790-1faa-488b-a23a-a622d99cb202, guest winsqlite3.dll P/Invoke open_v2 0x1, DB only %APPDATA% com.entropia.shared/entropia.sqlite): version 3.51.1, db_readonly(main)=1, quick_check 1 row `ok`, foreign_key_check 0 rows, item e929e121 count 1 with __entropia_internet_archive provenance substring present (dummy-pdf_2, IA Windows Test 62af3181; substring only, not full JSON parse), 2 assets with parent 7478 / child page-1 on-disk 1373 SHA full-match (child DB size column reads 0; on-disk authoritative; no claim all asset metadata equals file), close ok. Rust thumbnail NOT proven: app PID 6012 has no loaded pdfium.dll module in the observed snapshot (lazy init never fired this session); thumbnail cache then-unknown, no fresh import, no cache deletion. Evidence shots/w4-sqlite-20261011/ in the VM dir (outside repo). Task 27 still not closed.

- 2026-10-11, Windows Lite thumbnail cache readonly actual (same VM, no DB write): `%LOCALAPPDATA%\com.entropia.shared\thumbnails\daedde35-dabb-459a-9a46-9e003bde473f.png` 7425 B 10/10/2026 6:03 PM; controlled temp `$env:TEMP\NativeRustPdfium.pdf` 7478 B copied from existing public IA bytes (wildcard, no network, no asset reuse, no prior-record change). Win + EntropIA Lite hit the real single-instance guard (`ya esta abierta`), dismissed with Enter, no second process. Fresh collection/import/thumbnail/module association NOT attempted within the bounded UI budget; Rust app integration remains NOTVERIFIED (cached-only). No commits.

- 2026-10-10, task 17: `download.rs` streams to `<dest>.partial`, renames only
  after success, deletes the partial on every failure, refuses to overwrite an
  existing dest, clamps the cap to 500 MB..2 GB, and retries only network/5xx
  (max 2, injectable zero delay). URL is built with path segments. Shared
  `fetch_final_response` keeps `get_json` unchanged. RED: 11 tests against
  `todo!()`. GREEN: 115. The independent verifier failed twice to start; the
  parent ran `fmt`, `clippy -D warnings`, and `cargo test` (115 passed).
- 2026-10-10, task 18: `verify.rs` checks `%PDF`, declared length, remote md5/sha1
  (case-insensitive; empty means absent), always computes SHA-256, counts pages
  with `lopdf`, and deletes the staged file on every rejection. Markers:
  `verificación fallida`, `demasiadas páginas`. Page cap clamps to 1..=2000 with
  default 500. A PDF that opens with zero pages is also rejected (parent added
  that check and its test after review). RED: 5 failing tests against a stub.
  GREEN: 128. SHA-1 is hand-rolled and checked against FIPS 180-4 vectors because
  `sha2` does not include SHA-1.
- 2026-10-10, task 19: `display_file_name` takes the last path segment, strips
  controls and `<>:"/\\|?*`, prefixes reserved Windows device names, caps the
  length, and always ends in `.pdf`. Empty names fall back to the identifier,
  then `PDF`. Parent wrote it inline (one module). Tests went red on the first
  expectations, then green: 134 passed. `fmt` and `clippy -D warnings` clean.
- 2026-10-10, task 20: `src-tauri/src/internet_archive/` with `IaState`
  (production client, `JobStore` at `<data_dir>/staging/internet-archive`,
  created lazily) and commands `ia_resolve_identifier`, `ia_search`,
  `ia_list_collection`, `ia_get_item` in all three lists. camelCase DTOs,
  availability as string tags, `IaError` mapped to `<code>: <message>` with no
  paths. RED: 7 of 9 tests failed against stubs (and caught `texts_only`
  defaulting to false through a derived `Default`). GREEN: 9. Independent
  verifier: fmt, clippy `-D warnings`, `internet_archive` 9, `acl_manifest_guard`
  6, `ocr::pdf` 92; `Cargo.lock` only gained the `internet-archive` entry and the
  `entropia-agent` source line is intact.
- 2026-10-10, task 21a: crate `provenance.rs`: `ItemSnapshot` stored in the job
  (`#[serde(default)]`, old `job.json` still loads), `build_provenance` (requires
  `sha256_local` and `page_count`; URLs built with path segments; capped at
  64 KB by truncating `rights`, `title`, then list fields with `…[truncado]`,
  identity fields never touched; errors instead of returning an oversized value),
  `parse_provenance` and `ProvenanceRecord::from_row`. RED: 4 tests on `todo!()`.
  GREEN: 143. Parent re-ran fmt, clippy and tests, plus `cargo check --locked
  --tests` for the app.
- 2026-10-10, task 21b: `provenance_db.rs` (`SqliteProvenance` loads rows by
  provenance identifier in 500-id chunks, `json_valid` guard so one malformed
  row cannot fail the load; `collection_exists`; one lock, released before any
  network call) and `ia_prepare_import` (1..=100 selections, 1..=1000 files,
  identifiers only, collection must exist, unavailable files rejected with their
  tag, `Draft` job persisted, nothing importable creates no job). The writer
  asked whether to enable the crate's `test-support` feature on the app
  dependency; the parent rejected it (it would ship the loopback policy) and
  instead made the pure `parse_item_value` public so app tests parse the real
  fixtures without network. RED: provenance 6 and commands 7 on stubs. GREEN:
  `internet_archive` 29, crate 143, `acl_manifest_guard` 6, `ocr::pdf` 92; fmt and
  clippy clean in both; `Cargo.lock` unchanged. Note: the provenance query scans
  `items` (no index on the JSON path); acceptable for the MVP, revisit if slow.
- 2026-10-10, task 22a: `Timeouts` (connect 10 s, JSON total 30 s, download idle
  60 s, download ceiling 4 h) and `FetchKind`; downloads use `read_timeout`
  instead of the total JSON timeout; `IaClient::with_timeouts` for tests. RED: a
  download trickling past a 200 ms JSON total failed. GREEN: 146 (3 new: slow
  download succeeds, stalled download fails with no partial left, JSON keeps its
  total). Parent re-ran fmt, clippy and the crate tests twice, plus the app
  `internet_archive` tests (29). The defect slipped through the reviewed task 17.
- 2026-10-10, task 22: `runner.rs` (`Downloader` trait, live impl, `run_job`,
  `decide_start`) and commands `ia_start_import`, `ia_cancel_import`,
  `ia_get_import_status`, `ia_list_jobs`, `ia_list_verified_files` in all three
  lists; startup `recover_all` keeps `needs_reconcile` in state. Crate change:
  `partial_path` made public. RED: stubs failed. GREEN: 41. Parent review found
  three concurrency defects the single-job tests could not show: the runner kept
  a job copy across awaits (lost updates once task 23 writes the same job),
  `ia_start_import` checked and registered under two separate locks, and a
  cancel arriving as a run ended was dropped. Fixed test-first: one `jobs`
  writer lock with load-modify-save for every write (runner, start, cancel,
  recovery), lock order `active` then `jobs`, end-of-run cancel applied before
  clearing the registry. RED: a concurrent `Importing` change was reverted by
  the runner. GREEN: 43 (twice). Parent ran fmt, clippy, crate 146,
  `acl_manifest_guard` 6, `ocr::pdf` 92; `Cargo.lock` unchanged.
- 2026-10-10, task 23: `import.rs` with `existing_import` / `is_complete`
  (parent `pdf` asset without parent, children exactly 1..N, all files on
  disk; another sha256 is another version) and commands
  `ia_begin_file_import`, `ia_confirm_file_imported`, `ia_fail_file_import`,
  `ia_list_reconcile`, `ia_finish_reconcile` in all three lists; data dir kept in
  `IaState`. SQLite, `jobs` and `reconcile` locks are never nested. RED: 23 of 25
  tests on stubs. GREEN: 68. Parent review found a false negative: confirmation
  required the lowest-id complete item, so a correct item failed when an older
  complete copy existed. Fixed test-first by checking the confirmed item's own
  completeness (RED observed, then GREEN). Parent ran fmt, clippy, `internet_archive`
  69 twice, `acl_manifest_guard` 6, `ocr::pdf` 92; `Cargo.lock` unchanged.
- 2026-10-10, task 24: `lib/internet-archive.ts`: 15 typed wrappers (camelCase
  args), DTO and tag unions, closed `IA_ERROR_CODES` plus the `http_<status>`
  family, `parseIaError`, `onIaJobUpdated`. RED: missing module. GREEN: 12 tests
  in both variants; typecheck 0 errors in both; eslint clean. `pnpm install
  --frozen-lockfile` was needed first. Parent also changed one Rust message so a
  poisoned SQLite lock in `ia_prepare_import` reports the `io` code like every
  other command (app `internet_archive` 69, clippy clean).
- 2026-10-10, task 25: `lib/internet-archive-import.ts` (`importVerifiedFiles`,
  `resolveReconcileLeftovers`, `IaImportError`) and `discardFailedImport`
  exported unchanged. A rejected confirmation also discards the item it created;
  a second `discard_partial` is recorded `partial_remains` without calling fail
  (Rust only fails `importing` files). Known edge: an `io` (not validation)
  confirm error leaves the file `importing` until the next start, where recovery
  returns it to `verified`; no data is lost. RED: missing module. GREEN: 18 new
  tests; with the client and importer suites, 58 pass in both variants; typecheck
  0 errors in both; eslint clean.
- 2026-10-10, task 26a: `views/InternetArchiveDialog.svelte` (discover with
  three tabs, item files with availability, destination, summary with duplicate
  choices, sizes and synced volume, running with cancel, done) and ~130
  `internetArchive.*` keys in es and en. The writer found a design gap from task
  21b: a prepared job already contains duplicate files and no command drops one,
  so "skip" could not be honored. Parent decision: keep re-preparing without the
  skipped files and add `ia_discard_draft` (removes a job only while `draft`),
  used after a re-prepare and when the dialog closes before starting. Running the
  FULL desktop suite caught two regressions the targeted tests missed: a native
  checkbox (only `Checkbox` may use one) and a font family instead of a token;
  both fixed. RED: missing component, then the two token tests. GREEN: app
  `internet_archive` 74, `acl_manifest_guard` 6, `Cargo.lock` unchanged; desktop
  250 files green in both variants (3546 and 3525 tests); typecheck 0 errors in
  both; eslint clean.
- 2026-10-10, task 26b (commit 8a1ae94a): "Desde Internet Archive" in
  `ImportSourcesDialog` (opened from `HomeView`) and in `CollectionView`
  (preselected collection); resume notice for started jobs on dialog open;
  `runStartupReconcile` from `App.svelte` after the store is ready (once, never
  throws). RED: 5 view tests plus the missing reconcile module. GREEN. Parent
  review found that resuming a job left `ready` (or `running` without a runner)
  after a restart never restarted its downloads; fixed test-first by calling
  `iaStartImport` when no runner is active (RED: 2 tests, then GREEN). Desktop
  251 files green in both variants (3559 and 3538 tests); typecheck 0 errors in
  both; eslint clean.
- 2026-10-10, review of `c9ecf499..1ad6901d` approved with three advisory
  findings. Parent analysis: R3-003 hid a real data-loss path (reconcile entries
  were computed once at startup in a background task and kept in memory, so a
  frontend call that ran first got an empty list and the entries were lost
  forever). Task 28: entries are now derived on demand from `job.json`
  (`cancelled` file still holding `staging_name`), cancel paths clear
  `staging_name` when they delete staging, `IaState.reconcile` is gone, the
  startup reconcile retries after a failure and shares one in-flight call, and
  the dialog drops stale item and status responses. RED observed for each part.
  GREEN: app `internet_archive` 78, `acl_manifest_guard` 6, clippy clean,
  `Cargo.lock` unchanged; desktop 251 files green in both variants (3563 and
  3542); typecheck 0 errors; eslint clean.
- 2026-10-10, review of `224dbdbf..c9ecf499` (TS client and orchestrator) from a
  temporary worktree at `c9ecf499` (user-authorized): `correction_required`,
  BLOCKER R3-001, a rejected `discardFailedImport` escaped the per-file loop and
  aborted the whole import. Fixed in the worktree (`af5c2cbb`, 107 of 120
  planned diff lines; RED 3 tests, GREEN 21) and cherry-picked to the branch as
  `0defcfd5` (desktop 251 files green in both variants, typecheck 0 errors,
  eslint clean). The review's targeted validation timed out in the Pi relay
  after ~15 min; it needs `GENTLE_PI_REVIEW_RELAY_PI_TIMEOUT_MS` raised before the
  host starts. The lineage stays open and the worktree is kept until then.

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
- 2026-10-10, task 12: `IaClient::list_collection` shares a private
  `advanced_search` builder with `search` (whose request stays without `sort[]`)
  and adds one `sort[]=identifier asc`. RED: 5 compile errors
  (`list_collection` missing). GREEN: 52 passed. Independent verifier: fmt,
  clippy, 52/52 twice, `Cargo.toml` unchanged, no prior test lost. Live smoke
  (parent): `opensource` page 1 returned 5 of 1225104 items; all other live checks
  still pass.
- 2026-10-10, task 13: pure `job.rs` (no fs/net/clock): `Job`, `JobFile`,
  `ImportTarget`, `JobState`, `FileState`, validated transitions, `settle`,
  `request_cancel`, `finish_cancel_if_idle`, `normalize_after_restart`; serde
  snake_case for `job.json`. RED: 11 tests against `todo!()` stubs. GREEN: 63.
  Parent review found a gap the plan did not cover: a `Cancelling` job restarted
  with an `Importing` file marked it `Cancelled` and silently kept its staging
  (possible partial Corpus item, leaked file). Fixed test-first:
  `RestartEffects.needs_reconcile` reports it; the caller keeps a complete item or
  discards a partial one, then deletes staging (Phase 3 must honor this).
  Independent verifier: fmt, clippy, 63/63 twice, module pure.
- 2026-10-10, task 14: `store.rs` (`JobStore`): ids and staging names only
  as UUID v4 / `<uuid>.pdf` (traversal impossible by construction); atomic save
  (tmp, `sync_all`, rename, dir sync); `recover_all` skips symlinks and invalid
  names, drops leftover tmp, quarantines corrupt `job.json` keeping PDFs, leaves
  newer schema versions untouched, normalizes and saves, deletes released and
  orphan staging but never staging pending reconciliation. Added `uuid` and
  dev-dependency `tempfile`. RED: 16 tests against `todo!()` stubs. GREEN: 79.
  Independent verifier: fmt, clippy, 79/79 twice, tests confined to temp dirs.
- 2026-10-10, task 15: pure `prepare.rs`: `RemoteFingerprint` (md5 > sha1 >
  size+mtime, hex lowercased), `same_version` (strongest kind both sides have;
  `None` when incomparable), `ProvenanceLookup` trait for the app's SQLite
  lookup, `classify` (AlreadyInTarget > NewVersion > InOtherCollection > New;
  incomparable counts as a match, conservatively; sorted records for
  determinism), `summarize` and the synced-volume estimate (2x known bytes of
  files that would be imported; other-collection bytes apart). RED: 2 failing
  tests against stubs. GREEN: 104. Independent verifier: fmt, clippy, 104/104
  twice, module pure, only a test-only `allow(clippy::too_many_arguments)`.
- 2026-10-10, Phase 2 close: collection browsing, job model, job store and
  preparation logic done at crate level. App wiring (SQLite `ProvenanceLookup`,
  data dir, commands) moves with Phases 3-4. Phase 2 acceptance (inventory a
  paginated collection, close and reopen without losing the selection, no
  implicit download) holds at crate level: `job.json` survives restart through
  `recover_all`; nothing downloads yet.
- 2026-10-10, Phase 3 setup: Tauri system libraries installed with `pkexec`
  (the user typed the password in the system dialog). Lite baseline for
  `src-tauri` with `--locked`: `cargo check --tests` OK, `ocr::pdf` 88 passed,
  `acl_manifest_guard` 6 passed, `Cargo.lock` unchanged.
- 2026-10-10, task 16 (T3.0): `split_pdf_to_files` + `SplitLimits` in
  `ocr/pdf.rs` (one page in memory at a time, size accounted before writing,
  time checked between pages, cleanup of only this call's files, marker
  `demasiado grande para dividir`); shared `serialize_single_page` keeps
  `split_pdf_to_single_page_bytes` identical for `processing/ocr.rs`.
  `split_pdf_pages` keeps its signature and uses min(2 GiB, free - 512 MiB) and
  10 min. RED: 8 compile errors. GREEN: `ocr::pdf` 92 passed. Independent
  verifier: fmt, clippy `-D warnings`, `ocr::pdf` 92 twice, `processing::ocr` 4,
  `acl_manifest_guard` 6, no prior test changed, `Cargo.lock` unchanged. Note: if
  free space cannot be read the budget falls back to the 2 GiB cap.

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

## W4 reconciliation 2026-10-11 (history preserved; supersedes overstrong phrasing only)

- Prior `close ok` means prompt-returned; close/finalize codes ignored, not verified handle closure. SQLite readonly PASS stands with WARNs: child DB size 0 vs disk 1373 (disk authoritative), provenance substring only not JSON, no full metadata/global-dedup claim. Rust native integration NOTVERIFIED; 7425 B PNG filename-only; direct-DLL PASS historical. Temp copy only, no new collection/import; single-instance guard dismissed no fix; MSI awaits explicit EXE uninstall (none done); Pro needs matching build + fresh CI (not run). Task 27 / W4 in progress, ASSESS unassessable.

## W4 MSI replacement mirror 2026-10-11 (STOP BEFORE UNINSTALL; task 27 still open)

Second VM d9e70790 running. NSIS Lite 1.0.20 PID 6012; CloseMainWindow True but process persists, WAL 4:16->4:23 during copy, so no uninstall/MSI per hardgate. Guest-only backup $env:TEMP\EMSI20261011: sqlite 1339392, shm 32768, wal 4120032. Registry entry + hooks.nsh ($INSTDIR only) verified; installer ISO dbf2470/MSI 8948e11 preserved. IA/demo counts not re-proven; MSI/ProductCode/install/viewer pending. See windows-validation-sandbox.md actual. No commits.

## W4 MSI mirror 2026-10-11 (ONE X done; process ALIVE; STOP; task 27 open)

ONE visible-main X only (31680,1493, single click, no repeat/kill/tray/Rust/backend). Pre-X main 4/14 no overlay; post-X >60s PID 6012 alive (444 handles, 123.81s CPU), main still present. No NEW backup, no uninstall, no MSI run. MSI NOT tested. See windows-validation-sandbox.md. No commits.

## W4 MSI clarification 2026-10-11 (independent verifier; no overclaim)

User authorized EXE uninstall + MSI preserving data, but uninstall NOT invoked, MSI NOT executed: MSI NOT TESTED, not failed. ONE X: 'no exit observed' (PID 6012 alive), not 'ignores X'; click/handle delivery NOT proven, cause unknown. Guest %TEMP%\EMSI20261011 live-WAL copy: CONSISTENCY UNVERIFIED, not proven corrupt; size-equal not snapshot proof. No forced exit/reset/second close; first guest/host archive untouched. NSIS presumed current from stored query only. Stop gate respected. ASSESS unassessable untracked doc; independent verifier completed, no review closure claim. Next PROPOSED only: guest-only ONLINE backup API snapshot + checks, then explicit force-exit consent if needed. Task 27 open.

## W4 consistent-backup mirror 2026-10-11 (READY; STOP AFTER BACKUP; task 27 open)
Second VM d9e70790, Lite PID 6012 ALIVE (444 handles 127.98s CPU), no force. Guest-only online backup via winsqlite3 3.51.1: src readonly 1, backup DONE calls 3 pagecount 327 finish 0 closes 0, DEST readonly 1 quick_check 1 ok FK 0 collections 4 items 14. IA target 62af3181-da91-4a8d-a746-84db0ff4990d / e929e121-aa95-4700-b028-358580b680ac title dummy-pdf_2 metadata 1040 prov key+hint True json_valid 1 identifier dummy-pdf_2. Assets parent daedde35 7478 + child 3c0c4e00 page1 DB size 0 WARN (disk authoritative); on-disk parent 7478 29550d77... + child 1373 756a1f4c... valid; backup copy 17 files 0 mismatches, backup IA hashes match. Dest snapshot C:\Users\EntropiaTest\AppData\Local\Temp\EMSI-Verified-20261011-051221\entropia.sqlite SHA 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6 size 1339392 + backup-report.txt/manifest.json public only. Old EMSI20261011 preserved UNVERIFIED. Helper ISO w4-backup-20261011.iso 9a731835... B.cs 00304646... preserved under VM artifacts. See windows-validation-sandbox.md actual. No uninstall/MSI. No commits.

## W4 readonly-verifier mirror 2026-10-11 (media READY; NOT executed; task 27 open)
NEW VM/w4-verify-src/V.cs 93 lines SHA df225dae3d14b390276433a7d178f1512ab2c1332cb3b66c22bf39a70942ee09 + ISO w4-verify-20261011.iso 71680 SHA 32b619e9810089c51c72e9e9da493439bb438d9f480dee3143b4347e1d3f9cdc vol ENTROPIA_VFY (pycdlib venv). Manifest layout audited from B.cs: files[] {rel,len,sha}; V parses via JavaScriptSerializer reflection, reads ONLY TEMP/EMSI-Verified-20261011-051221 manifest + 17 files + DB (pins DB 1339392/03996561...), rejects rooted/escape/links/source reads, PASS only 17/17 + DB + disposed. CDROM swapped readonly to verify ISO (UUID d9e70790 running, sysdisk unchanged, priors preserved). Entry for verifier ONLY: Add-Type -Path 'D:\V.cs';[V]::Run() (~112 -enc). NOT executed by worker; Lite RUNNING; gate pending. See windows-validation-sandbox.md. No commits.

## W4 V2 verifier mirror 2026-10-11 (RED fix READY NOT executed; task 27 open)
RED V.cs:54 ArrayList cast -> InvalidCastException (JS returns object[]). NEW VM/w4-verify2-src/V2.cs 95 lines 721c50e7... class V2, SINGLE ICollection+null guard, Count 17/PASS-only-all kept. NEW ISO w4-verify2-20261011.iso 69632 a125f0f9... vol ENTROPIA_VF2 Joliet V2.cs verified. CDROM swapped ro after UUID d9e70790, olds preserved. Entry: Add-Type -Path 'D:\V2.cs';[V2]::Run() (~116 -enc). NOT executed; no force/uninstall/MSI. Gate PENDING. See windows-validation-sandbox.md. No commits.

## W4 force/NSIS/MSI mirror 2026-10-11 (NSIS PASS, MSI FAILED, data PRESERVED; task 27 open)
Gate independent V2 GREEN: 17/17 MATCH + db 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6 1339392 PASS. Force PID 6012 (unique, Lite path) via authorized Stop-Process -Id 6012 -Force; absent confirmed NoProcessFound. Source S.cs PASS (readonly 1, quick 1 ok, FK 0, IA 62af3181/e929e121 dummy-pdf_2, parent 7478 + child 1373 valid, child DB 0 WARN). Closed manifest lock 0 / sqlite 1339392 / shm 32768 / wal 4120032. NSIS full UI (data checkbox unchecked): Complete successfully; registry + binary gone; manifest IDENTICAL. MSI 1.0.20 (SHA 8948e11b...) full UI + verbose log TEMP\MSI-Install-20261011.log 328172 B: UAC Yes, then 1722-type package error, wizard ended prematurely, system not modified. Manifest after failed MSI IDENTICAL. Backup EMSI-Verified + old EMSI20261011 intact, no restore. ProductCode/UpgradeCode not COM-read (no guess). No MSI launch/verification, no Pro, no commits. See windows-validation-sandbox.md actual.

## W4 MSI-log reader mirror 2026-10-11 (READY NOT executed; MSI FAILED stands; task 27 open)
NEW VM/w4-msilog-src/L.cs 86 lines SHA f72b1f338ccbc05a6aeafc8b48a2f99e1f225d926fe6170faa1fb7f7a16fa006 + ISO w4-msilog-20261011.iso 69632 SHA 9e04a2bb641b8164447d0a121a04365418ded371494fdd9faa94a577826b9ea9 vol ENTROPIA_MSI (pycdlib existing venv, Joliet L.cs verified). CDROM swapped readonly after UUID d9e70790 running; sysdisk unchanged; priors preserved. L reads ONLY MSI log (BOM autodetect, FIRST 1722/Return-3/CustomAction-actual +-4 dedup <=80, size+SHA) + Uninstall limited fields + bin exists + backup EMSI-Verified 03996561.../1339392 exist/hash + APPDATA sqlite/wal/shm lens/hashes; no writes/SQL/Process/kill/env/creds; no live-vs-snapshot same-hash claim (post-fail manifest SIZE-only identical). MSI FAILED (1722-type, prematurely, not modified), NO success claim; backup protected, no restore. Screen w4-msi-log-screen: NO StickyKeys; no worker typing/policy change. Entry verifier ONLY: Add-Type -Path 'D:\L.cs';[L]::Run(). NOT executed. Gate PENDING. See windows-validation-sandbox.md. No commits.

## W4 MSI-log diagnosis mirror 2026-10-11 (FAILED bootstrapper; task 27 open)
Independent L executed once (0.03s spacing): MSI log 328172B 1704 lines SHA 798e9abcb9f0e6e4138360bd3935a6e5dc8fd565594da3382fd3dc926ed8eb77; FIRST 1047 ForceInvokeWebView2Bootstrapper -2147219416=0x80040828, 1722 lines 1048/1051, InstallFinalize Return 3 line 1056; fragment wxs 45-55 NOT REMOVE; prior WebView2 155.0.4283.45; EdgeUpdater reason NOT read; winget 192036 correlation not conclusive. Backup EMSI-Verified 03996561...a6b6 quick_ok FK0 4/14 17-file match; force ONLY after gate; NSIS removed unchecked; MSI NOT installed. Source current DB 530cf722... WAL 315c548c... SHM 18509a9d...; hash diff NOT corruption proof; rows not requeried. Backup protected, no restore. No fix/tests/commit/push/build without next permission. See windows-validation-sandbox.md actual. No commits. Historical pre-fix failure; superseded by the corrected MSI PASS below.

## W4 corrected-MSI mirror 2026-10-11 — current PASS (historical FAILED above preserved; task 27 still OPEN)

This mirror is the current result. All older MSI FAILED mirrors above are now historical pre-fix records and are preserved as history. Task 27 stays OPEN for Pro with Rust Pdfium integration, additional IA cases, and the child-asset DB size follow-up.

The corrected MSI (SHA256 335c8dc7d6f510e69e9562f8e653500d9f16d055a77d837e1e116abaefdbae21, 30707712 bytes, source 3b) was installed once with the full user interface. The human authorized one normal install and accepted only the expected unknown-publisher UAC prompt from CD/DVD source. The package is actually unsigned. Install completed at the default current-user path C:\Users\EntropiaTest\AppData\Local\EntropIA Lite\, with exactly one 64-bit HKLM Windows Installer registration (GUID {1AD2370F-66DB-429F-AE0C-A8EE4DC1D4DE}, version 1.0.20, publisher entropia). The installed program is 50671616 bytes, 64-bit, version 1.0.20. Launch was unchecked until preservation was checked, then started once as PID 10792 with child WebView PID 9884 under it, 5 related processes, version 155.0.4283.45. The app was left open on the selected PDF with no forced closure.

The fresh log %TEMP%\msifixed-20261011.log shows the bootstrapper action return code 1, InstallFinalize return code 1, Logger MainEngineThread server return code 0 and client return code 0 (successful), installation completion status 0, and no Return value 3. These are return codes, not counts. No raw bootstrapper child exit-code 0 proof was established. Integrity checks before install, after install before launch, and after startup while alive each reported PREFLIGHT_PASS plus True, read-only main 1, quick_check one row ok, zero foreign-key violations, 4 collections and 14 items, target e929e121 present once, and 17 of 17 files matched with zero bad. The post-install V2 check matched 17 of 17 against backup database 1339392 bytes SHA 039965617fef73adaeb8974050e5b7dbba64ceee571690279f4c20bfdbbba6b6. The old coherent backup stayed immutable with no restore. WAL and SHM were both present through a read-only link. The child asset still shows DB size 0 against 1373 bytes on disk.

The library kept 4 collections and 14 documents (Actas 2, Casas 6, La Capital 5, IA Windows Test 1). The existing item e929e121-aa95-4700-b028-358580b680ac with dummy-pdf_2 showed the real text Dummy PDF download. That display uses PdfJS and does not prove native Rust Pdfium. The old thumbnail cache stays uncorrelated and NOT VERIFIED, and Pro with local ML stays NOT VERIFIED. Protected screenshots and the read-only V4 diagnostic ISO are recorded in windows-msi-webview2.md and windows-validation-sandbox.md. Parent observed documentation work-unit commit `e4dc60a1981f738c6d6ca025917752ed11ce7f7e` on unchanged source 3b (doc commits local UNPUSHED); guest app still OPEN as PID 10792. Task 27 stays OPEN. No commits by this worker.

## W4 native-thumbnail trial mirror 2026-10-11 (bounded security BLOCK; task 27 OPEN)

Passive mirror of windows-validation-sandbox.md actual; past baseline/MSI receipts above preserved unchanged. All main tasks OPEN.

New fixture `.git/entropia-native-fixture-20261011/P.pdf` 608 B SHA 7f9ffea8451c82e61833b4244fc901e7e0582bf2a6cccd74b4454477630017b8 (one-page `Native Rust Pdfium Proof20261011`, pdfinfo/pdftotext valid). ISO 65536 B SHA b7b6a2c184ee1b2a770c535efee96eee5ffd90286ff38d06fc88c3f90c5f731c, exact 2 files P.pdf/SHA256SUMS.txt; verifier NOGO 0664 repaired to parent-owned 444 ISO, freshly PASS mode 444/digest before guest mount; current readonly DVD this ISO, v4 preserved.

BEFORE P.Run PASS 4/14/17 files readonly 1 quick ok FK 0 final-close 0 True; app 10792 alive; cache only daedde35-...png 7425 B SHA E3990E3B... 400x566 loaded-pdfium 0 at 2026-10-11T15:53:14.4450222-03:00. New collection `Native Rust Pdfium Test20261011` + D:/P.pdf normal UI import (forward-slash picker reject = NO import; backslash picked ONE). Visible one-page PDF + rotated card thumbnail (unresolved, NOT renderer PASS). UI 5 collections (2/6/5/1 retained) total 15 arithmetic, NOT fresh SQL. No source-17 post hash PASS, no backup post PASS recorded. App/new data remain; no cleanup/kill/relaunch.

BLOCKED on unnecessary `powershell -enc` module query: reported `Threats found`, then captured powershell.exe AccessDenied/ResourceUnavailable/NativeCommandFailed. PNG read + scout = ERROR ONLY, NOT threat toast; detection/cause/quarantine UNKNOWN. Encoding not reused. Guest inputs STOPPED (no retry/allow/exclusion/toggle/restore/dismiss/override). Human approval needed for readonly ProtectionHistory inspection before more diagnostics. NOT VERIFIED: post PID/DLL/new cache PNG/asset ID/SQL/17 hashes/backup; native integration NOT PASS. Old P.Run hardcodes 4/14, do NOT rerun after 5/15. MSI DONE unchanged. Pro 615 = workflow commit/push/build artifacts only, NO releases/tags/PR; NO CI change/build yet. Evidence R shots nativeunit-{preflight-result,cachebaseline-result,fixturehash-result,created,import-start,queue11s,module-command,security-stop}.png; incident memory 616. Bounded incident, NOT product-bug cause. No future guest action.

## W4/task 27 reconciliation 2026-10-11 (CURRENT: native Pdfium subcheck PASS; task 27 still OPEN)
Passive mirror of `odd/tasks/windows-native-pdfium.md` CURRENT R4 PASS; all history above preserved. R4 unit 630 ONE Run PASS `R4_CORRELATION_PASS` + True (R4.cs 68685 `58e158bc...`, ISO 122880 `a7dc4b9a...`, 444 DVD readonly; V1-V3 preserved, R3 DON'T rerun). Target `83342a03...` -> `40b166ce...` P -> `990dfc88...` Native Rust Pdfium Test 20261011; source 608 B `7f9ffea8...`; NEW PNG 11501 B `B9C87E23...` 300x400 UTC 2026-10-11T18:58:57.1294797Z; old `daedde35...` retained 2/2. LIVE 5/15 (1,1,2,5,6), backup PRE=POST 1339392 `03996561...`, 17/17 bad 0; DB-15 + 2 staging job.json extras = 17. ES: ONE unsubmitted line Escape-cleared, plain commands, no encoding, NEW alert not observed. Console clean; app 10792 last alive prior check. Native app-integration subcheck now PASS (NOT pdf.js/direct-DLL; DLL-0-after not failure). Task 27 stays OPEN: Pro, additional IA cases, child DB size 0. No all-Windows-DONE claim. No commit (Native Doc Git grant required; 615 NOT used).
