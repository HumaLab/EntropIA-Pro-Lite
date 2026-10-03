# Biblioteca (Zotero) as a search scope across the app

## Objective

Make the Zotero bibliographic layer a selectable search scope everywhere the
user searches: the research chat, Investigación, the Zotero search in Writing,
the TopBar search, Writing's "Obras" tab, and a new top-level "Biblioteca"
section. Requested by the owner on 2026-10-02.

## Finding that sets the order (verified 2026-10-02)

No production code writes `zotero_attachments`: every caller of
`bibliography::repository::upsert_attachment` and every `INSERT INTO
zotero_attachments` sits inside a test module (`retrieval.rs`, `detail.rs`).
The library sync reconciles top-level works only (`processing.rs:190-195`).
So the whole full-text chain (PDF extraction → `bibliographic_page_texts` →
chunks → chunk embeddings → `search_passages`) never receives a PDF; only the
metadata profiles (`search_works`) are searchable today. `zotero_data_dir`
(needed to resolve stored attachments) has no UI.

## Tasks (route: delegated writer per task; TDD strict, Vitest + cargo test)

- [x] B1 — Data layer: sync each work's PDF attachments into
  `zotero_attachments` (local API attachment walk), so extraction/chunks/
  embeddings run. Route: delegated writer. Commit d168ff2. No data-dir setting
  was needed: Zotero's local API reports each attachment's file as
  `links.enclosure.href` (a `file:` URL, also for `imported_url`), stored as
  `native_path`, which the resolver already tries first.
- [ ] B1b — Index Zotero HTML snapshots too (owner, 2026-10-03). Live count of the
  owner's library: 969 stored `text/html` snapshots vs 733 stored PDFs (plus 280
  link-only web attachments with no file). Extraction is PDF-only today. Extract
  readable text from the stored HTML, chunk and embed it like PDFs; passages
  carry a character range instead of a page.
  Design (agreed with the owner): split on block elements (`<p>`, headings,
  `<li>`, `<blockquote>`, `<td>`; `<br>` as a soft break), NOT on source line
  breaks (HTML collapses whitespace). Drop `script`/`style`/`nav`/`header`/
  `footer`/`aside`. Emit one paragraph per block, blank-line separated, and feed
  the existing paragraph chunker (`chunks.rs`, ~800 chars, paragraph-aligned
  overlap) as a single page; citations point at the paragraph(s), no page.
- [x] B2 — Writing's Zotero tab also searches by semantic similarity (reuse
  `search_works`, map library ids). Route: delegated writer. Commit d7e9322.
- [ ] B3 — TopBar search includes bibliography works (new result kind).
- [ ] B4 — Research chat scope: Corpus / Biblioteca / both (works level first,
  passages after B1). Product decisions pending.
- [ ] B5 — Passages in "Obras" and a top-level "Biblioteca" section.
- [ ] B6 — Investigación can use the Biblioteca (cross-repo `entropia-agent`).

## Constraints

- Lite and Pro share code; embeddings follow the user's provider setting (API
  in Lite). Bibliography tables are local-only (not synced).
- Never mix corpus and bibliography scores numerically; keep separate legs
  (see `bibliography::compose`).
- No JS migration without reporting first; dev profile for any `tauri dev`.

## Decisions

- 2026-10-02, owner: bibliography citations in the chat behave exactly like
  corpus citations: inline `[n]`, each source is a passage (snippet, page,
  char range) that opens the document at that fragment, highlighted. This
  makes B1 (PDF attachments + full text) a prerequisite of B4.

## Progress

- 2026-10-02: plan opened; B1 started.
- 2026-10-03: B1 done. Live local API: `GET /items?itemType=attachment`
  (2065 attachments, paged 100) returns `data.parentItem`, `linkMode`,
  `contentType`, `filename`, `md5`, `mtime` and `links.enclosure.href`
  (`file:///C:/Users/.../storage/<key>/<name>`); `imported_url` PDFs have one
  too. One paged walk after the items walk, inside the sync executor; rows a
  complete walk no longer lists are deleted (cascade); an unreadable or
  stopped walk deletes nothing. TDD: RED = compile failure of the new tests
  (missing API), GREEN after implementation; 6 new tests in
  `tests/bibliography_processing.rs` + 1 URL test in `connector.rs` (written
  just after the builder, not before). Checks: lint, typecheck, format:check,
  vitest, cargo fmt/clippy/test green except `web_capture_sync_two_device`
  (4 tests; they fail on the base too: the test Cloud server lacks
  `web-capture-v1`).
- 2026-10-03: B2 done. `bibliography_search_works` gained an optional
  Zotero scope (`zoteroLibraryType` + `zoteroLibraryId`, as the tab names the
  library); `retrieval::resolve_zotero_library_rows` maps it to the internal
  `zotero_libraries.id` rows the filter and the hits use, and the response
  carries `librarySynced` (false = library never synced into the catalog, so
  nothing was searched). No new command, no ACL change, no migration.
  `searchLibrary` in `writing-zotero.ts` now runs the Zotero search and
  `bibliographySearchWorks` together (`allSettled`). Ranking rule (never
  adds or compares the two score scales): text matches first (list, then
  Zotero-only) in their own order, then works only the bibliography found,
  in its ranked order, deduplicated by Zotero item key (the search is
  already scoped to one library). Hits whose key is not in the library read
  from Zotero are dropped (not citable). Entries from the vector leg carry
  `semantic: true` and show a "Por significado" tag. The tab states
  `not_synced`, `lexical_only` (no active generation / embed failure) and
  `failed`; Zotero being unreachable no longer prevents the semantic leg.
  TDD: RED observed for the Rust resolver test (compile failure), the 8
  store tests (8 failed) and 4 of the 5 tab tests; the "stays silent when ok"
  tab test passed at RED (negative assertion). Checks: lint, typecheck,
  format:check, vitest green; cargo fmt/clippy green; cargo test green except the 4 known
  `web_capture_sync_two_device` (stale Cloud binary).
- 2026-10-03: B1 reopened. Owner's first real run (dev profile `navegador`,
  Lite): "Sincronizar biblioteca" failed with `unknown_library: no catalog row
  for user/0`. Finding: no production code ever created `zotero_connections` /
  `zotero_libraries` rows (every `upsert_connection`/`upsert_library` caller is
  a test), so library sync could never start; pre-existing since the Zotero
  merge. Fixed in 3dbf2f2: `apply_bibliography_sync_request` first calls
  `ensure_local_zotero_library` (same transaction), which inserts the single
  `local-zotero` connection (origin local, endpoint 127.0.0.1:23119) and the
  library with `DO NOTHING` semantics. Not `upsert_connection`: it bumps the
  connection revision, the reconciliation fence, which would retire a running
  walk on every click. Identity: personal library is `user/0` (the local-API
  alias the executor reads and `Library::personal()` uses everywhere); groups
  keep their numeric id; name "Mi biblioteca" for user/0, "Zotero group <id>"
  otherwise (no network call inside the request transaction). A namespace that
  already has rows is untouched, so cross-connection duplicates still fail as
  `ambiguous_library`. `recovery.rs` resume test (user/0) still passes. TDD: RED
  observed (`unknown_library` on an empty catalog), GREEN after the fix. Full
  cargo green except the 4 known `web_capture_sync_two_device`. B1 is done again.
