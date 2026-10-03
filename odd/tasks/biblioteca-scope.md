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
- [x] B1b — Index Zotero HTML snapshots too (owner, 2026-10-03). Live count of the
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
- [x] B3 — TopBar search includes bibliography works (new result kind). Route:
  delegated writer. Commit e9404ed.
- [x] B4 — Research chat scope: Corpus / Biblioteca / both, with cited Zotero
  passages. Route: delegated writer (single writer on main). Commits b4e2fe6
  (backend) and 1dc2113 (UI). See the 2026-10-03 B4 progress entry.
- [ ] B5 — Passages in "Obras" and a top-level "Biblioteca" section.
  - [x] B5a — Passages in Writing's "Obras" tab (route: delegated writer, single
    writer on main).
  - [ ] B5b — Top-level "Biblioteca" section (awaits the owner; NOT part of B5a).
- [ ] B6 — Investigación can use the Biblioteca (cross-repo `entropia-agent`).

- [ ] B7 — First sync of a large library must be usable early and show its progress (owner,
  2026-10-03). Measured: profile embeddings were 80 of 82 busy minutes, one request at a
  time. B7a (running, branch perf/bibliography-embeddings): batched embedding requests +
  3-4 bounded concurrent requests with 429/Retry-After backoff; skip already-embedded chunks.
  B7b (after B7a is measured): order work so every work's metadata profile is done first
  (works search by meaning usable early), then passages, recent/opened works first.
  B7c: progress and time estimate in the Zotero tab instead of 'processing in background'.
  Not pursued: several OpenRouter keys/accounts to raise throughput (limits are per account;
  extra accounts would breach their terms).

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

- 2026-10-03, coordinator (reversible, B4): ONE answer with ONE continuous `[n]`
  numbering across both scopes; each source shows a scope label ("Corpus" /
  "Biblioteca"); a bibliography source names the work (authors · year · title),
  its library and its location: "p. N" / "pp. a–b" for PDFs, "párr. a–b" for
  HTML snapshots (decided from the attachment `content_type`; an HTML snapshot
  is a single stored page 1 and never shows "p. 1"). The scope choice is per
  session (conversations persist no settings of their own), not per
  conversation. Merge rule: the two legs are never compared or added by score;
  they are interleaved by rank (corpus 1, biblioteca 1, corpus 2, ...) under
  the existing `top_k` and `context_max_chars`.

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
- 2026-10-03: B1 hang diagnosed and fixed (7266d88). Owner's real run walked
  items (2812) and attachments (737 PDFs) and then sat `running` forever.
  Root cause, found by replaying the publication steps over a copy of the
  frozen dev DB: 43 works have no title, and `profile_input_for_item` read the
  title column as non-NULL, so the chained profile admission inside the success
  publication failed ("Invalid column type Null ... title"). `run_one` then
  returned that error without ending the attempt, leaving the task `running`
  under a live lease with nothing to resume it (0 CPU, no log). Fixes: title
  optional; a publication that cannot commit now fails the attempt with
  `publish_failed` (retry with backoff) instead of stranding it; one log line
  per phase (start, items done, attachments done, published with task counts)
  and per pause/failure goes to the app log through `set_log_sink`. TDD: RED
  for both new tests (untitled work; stranded `running`), two old tests that
  asserted the stranding were updated to the new contract. A scale test (2812
  works, 2065 attachment rows) settles in ~6s. On the copy, profile admission
  now succeeds (2657 tasks). Full cargo green except the 4 known
  `web_capture_sync_two_device`.
- 2026-10-03: B1b done (route: delegated writer, single writer on main).
  Catalog: `attachment_page_from_json` also keeps parented `text/html` and
  `application/xhtml+xml` attachments unless `linkMode` is `linked_url` (a bare
  web link has no file), so the 280 link-only attachments stay out. Extraction:
  new `bibliography/html_text.rs` (`dom_query` 0.27, already compiled for
  tauri-utils/wry, now declared directly so no new crate enters the build;
  `encoding_rs` was already a direct dependency). Block elements (`p`, `h1-6`,
  `li`, `blockquote`, `td/th`, `pre`, `figcaption`, `dd/dt` plus the container
  blocks `div`, `section`, `table`...) each end a paragraph; `<br>` is one
  newline inside it; source line breaks collapse; `head/script/style/noscript/
  nav/header/footer/aside/form/template/svg/iframe` and hidden elements
  (`hidden`, `aria-hidden=true`, inline `display:none`/`visibility:hidden`) are
  dropped; the walk is iterative (no stack risk on deep pages) and output is
  capped at the 4 MB page bound. Charset: BOM, then `<meta charset>`, then
  UTF-8, then Windows-1252. The executor stores one page (page_number 1, method
  `native`, quality `rich` or `empty`) so `chunks.rs`, profiles and embeddings run
  unchanged; boilerplate-only pages yield an `empty` row and no chunks, no error.
  `BIBLIOGRAPHY_EXTRACT_MAX_BYTES` still gates the read. No migration, no schema
  change. Display marker: none needed yet (no UI shows bibliography page
  numbers); B4/B5 must decide "p. N" from the attachment's `content_type`
  (`text/html`/xhtml means "web snapshot, show paragraph range, no page").
  TDD: RED observed (7 `html_text` unit tests against a stub, 6 failed; 3
  extraction tests failed with `extraction_unsupported`); GREEN after the
  implementation; two catalog tests updated to the new contract (HTML now
  cataloged, linked_url/image still skipped).
- 2026-10-03: B3 done (route: delegated writer, single writer on main). The
  TopBar dropdown gained a separate "Biblioteca" group below the corpus rows
  (corpus rows, order and behaviour untouched; the two score scales are never
  compared). `bibliography_search_works` hits now also carry `authors`, `year`,
  `libraryName`, `libraryType`, `libraryNativeId` and `cslJson`
  (`retrieval::read_work_display`, read per hit in the command; no change to
  `search_works`, no new command, no ACL change, no migration). The TopBar calls
  it unscoped (all synced libraries, top 5) concurrently with the corpus
  search, same debounce and request-id guard; corpus results are shown as soon
  as they are ready and the group appears when the bibliography answers. A
  failing bibliography leg logs a warning and adds nothing. No synced library
  means no hits and no group. A row shows title, "authors · year", library
  name and the "Por significado" tag (B2's `writing.zoteroSemanticTag`) for
  hits that came through the vector leg. Choosing a hit (click or Enter, one
  flat arrow-key sequence across both groups) shows the existing
  `WritingZoteroDetails` ficha inside the dropdown, in place of the list, with
  its own back button; no navigation, no tab change, no manuscript needed (the
  Zotero tab only exists with an open document, so routing to Writing would
  have been the surprising path). Escape, typing or leaving the search closes
  it. TDD: RED observed for 2 Rust tests (`read_work_display` stubbed) and 9
  TopBar tests (timeouts waiting for the missing group); GREEN after the
  implementation. Checks: lint, typecheck (Pro and Lite), format:check, vitest
  (230 files) green; cargo fmt/clippy green; cargo test green (2197 passed, 0 failed).
- 2026-10-03: B4 done (route: delegated writer, single writer on main).
  Backend (b4e2fe6): `rag_ask` takes `scope` (`corpus`|`biblioteca`|`both`,
  default corpus: an old caller gets today's answer) and `libraries`
  (`libraryType` + `libraryId`, empty = every synced one). New
  `rag/scope.rs`: `bibliography_leg` runs `search_passages` on its own
  connection (embedding may be a network call in Lite; the shared worker
  connection is not held), applies `rag_min_similarity` to its own cosine
  scale, snips like the corpus, and returns sources; `merge_scopes`
  interleaves the two ranked lists by rank only under `top_k` and
  `context_max_chars` and renumbers 1..n. Corpus-only keeps its exact old path
  (no merge); Biblioteca-only skips corpus embedding and rerank. Stored source
  shape: `RagSource.bibliography` (optional, `skip_serializing_if` none; chunk
  id, item key, library name/type/native id, authors, year, location
  `{kind: pages|paragraphs, from, to}`), so old conversations load and older
  builds ignore the field. Chat conversations ARE part of the sync set
  (`rag_conversations`, `rag_messages`), so a bibliography source can reach a
  device whose local-only catalog lacks the chunk: the reader says so and shows
  the stored snippet. Prompt header per fragment: corpus unchanged
  (`«title» (collection)`), bibliography `«title» (authors · year · p. N)`.
  `RagAnswer.bibliographyNotice` (`no_library_synced`, `no_embeddings`,
  `embedding_unavailable`, `failed`; not persisted). Location: PDF = min/max
  span page; HTML (attachment `content_type` contains "html") = paragraph range
  computed with the chunker's own blank-line split (`chunks::paragraph_range`)
  over the page text. New commands (ACL: `build.rs`, `capabilities`, handler):
  `bibliography_library_status` (synced libraries with work/passage counts and
  whether a generation is active) and `bibliography_passage_context` (the
  expansion of `bibliography_open_passage` without opening anything).
  UI (1dc2113): `RagChatView` has a Corpus / Biblioteca / Ambos `TabList`, a
  `ToolbarMenu` of checkbox libraries (reopens after each toggle; default
  "all" = no filter sent), honest notes (no library synced, no vectors, status
  failed), a scope tag on every source, work line + location for passages, and
  a passage reader (`ConfirmDialog`, page text window around the cited range
  marked like `mark.citation-hit`; "Abrir original" calls
  `bibliography_open_passage`). The scope choice is in the `ragChat` store
  (session). An untouched chat sends the same `rag_ask` payload as before.
  Viewer decision: not the item viewer. The Zotero storage folder is outside
  the Tauri asset-protocol scope (`$DATA/com.entropia.shared/**`,
  `$LOCALDATA/...`), so the in-app PDF/HTML viewer cannot render those files
  without widening it; it was NOT widened. The in-app reader shows the page
  text the catalog already holds (identical offsets to the cited span) and the
  original opens in the OS viewer on request.
  Findings: (1) corpus citations in the chat do not highlight today either:
  `openSource` navigates to the item/asset without `citationRange`, so
  `RagSource.provenance` offsets are unused by the UI. (2) `search_passages` is
  vector-only; there is no lexical passage fallback, so with no active
  generation the Biblioteca leg says so instead of falling back. (3) The
  research handoff ("Profundizar") spreads bibliography sources (empty
  `assetId`) into the Investigación context; that is B6's concern.
  TDD: RED observed for 18 Rust tests (`rag/scope.rs` stubs: scope parsing,
  merge by rank/budget/numbering, labels, library filter, notices, HTML
  paragraph location, min similarity), 3 prompt/answer tests (compile failure
  on the new field), 5 store tests, 14 view tests and the `rag-scope` helper
  tests (module missing). Written alongside rather than strictly RED-first:
  `library_status` (2 tests), `ragAsk` options and the three
  `bibliography-search` wrappers (glue), and the persistence round-trip test
  (it passed on first run because the shape existed by then; it still pins the
  stored JSON). Checks: lint, typecheck (Pro and Lite), format:check, vitest
  (232 files, 3218 passed) green; cargo fmt/clippy green; cargo test green
  (2222 passed, 0 failed). `--features local-ml` was not compiled (it pulls
  the MNN source build); the `local-ml` branches touched are the rerank guard
  and a `bibliography: None` in a test helper.
- 2026-10-03: B5a done (route: delegated writer, single writer on main).
  "Obras" now searches works AND passages for the same query (submit and
  "Buscar desde la selección"), concurrently and independently: a failing
  passage search never hides the works, nor the reverse. Layout decision:
  passages are a separate "Pasajes" section BELOW the works (the works list
  stays byte-for-byte as it was), each work's passages grouped under a small
  work header (title, "authors · year · library"); rows follow the Zotero tab's
  pattern (text left, eye + quote `IconButton`s right, no stray text nodes).
  Nesting under each works hit was rejected: the passage search admits its own
  candidate works, so passages rarely belong to the works listed above and a
  nested layout would hide the ones that do not.
  Backend: new command `bibliography_search_passages` (`build.rs`
  `APP_COMMANDS`, `capabilities/default.json`, handler; no migration). It does
  not reimplement anything: `rag::scope::passage_search` wraps the chat's own
  `bibliography_leg` (same libraries rule, `rag_min_similarity` floor, snippet,
  PDF "p." / HTML "párr." location, same notices) and adds the work's CSL-JSON
  for citing. Passage search stays vector-only, so with no active generation
  the tab says so (`no_embeddings`) instead of an empty list; wording family of
  "Solo búsqueda léxica" (`bibliography.passagesNotice.*`).
  Frontend: the passage reader was extracted from `RagChatView` into
  `components/PassageReaderDialog.svelte` (same dialog, same look; the CSS moved
  with it as `passage-reader__*`); the chat and the tab both use it.
  The location text is the chat's `locationText`; new pure helpers
  `locatorOf` (citation locator: "3", "3-4", type `page`/`paragraph`),
  `passageHeading` and `passagesNoticeKey` live beside it in `rag-scope.ts`.
  Cite: the tab takes `oncite` (WritingResearchPanel passes `oncitezotero`, the
  Zotero tab's own path) and inserts the same citation payload with `locator` +
  `locatorType` from the passage location: the citation model already supports
  them. Gaps: (1) `itemVersion` is `null` (the catalog does not keep Zotero's
  item version; the model accepts null). (2) `WritingView.citeZotero` opens the
  citation editor on the Zotero tab after inserting, as for any Zotero cite, so
  the user lands there with the locator prefilled. (3) `citeWork` ignores a work
  already cited right beside the caret, so a second passage of the same work
  cited back to back does not add its locator (existing behaviour, untouched).
  (4) A passage whose work has no CSL-JSON in the catalog cannot be cited
  (button disabled).
  TDD: RED observed for 2 Rust tests (`passage_search` stub), 5 `rag-scope`/
  wrapper tests (missing exports) and 14 tab tests; GREEN after implementation;
  `RagChatView` tests (63) stayed green through the extraction. Existing tab
  tests were re-pointed from a blanket mock to per-command routing (the tab now
  calls two commands).
