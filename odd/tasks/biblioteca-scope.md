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
- [ ] B2 — Writing's Zotero tab also searches by semantic similarity (reuse
  `search_works`, map library ids).
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
