# Audit remediation (plan.md)

Source: `plan.md` (remediation plan for `auditoria.md`). One branch/PR per task or small group of
related tasks; security never mixed with refactors. Work starts at Phase 0, then Phase 1.

## Phase 0 — Linux CI baseline (branch `ci/audit-phase-0`)

- [x] P0.1 (Q-05) — Lite `cargo test` passes on Linux. The `navegador/download.rs` half was already
  fixed in `5905a2dd`. Baseline (Lite, Linux): only
  `attachment_page_keeps_pdfs_..._decodes_the_enclosure` failed (RED: `C:/Libros/externo.pdf` is not
  absolute on Linux, so `native_path` was `None`). Production is right; the fixture now uses a
  platform-native absolute path, plus a non-Windows test that a drive-letter path is not local.
  GREEN: `bibliography_processing` 168/168; every other binary was already green (lib 2444 passed);
  clippy `-D warnings` and fmt clean. Commit `985ef89f`.
- [x] P0.2 (C-01) — `rust-lite-linux` CI job on `ubuntu-22.04`: fmt, clippy `-D warnings`, test,
  no features, gated by `detect-rust-changes`. Commit `cd357702`. Pending: observe the job green on
  the PR (needs a push).
- [x] P0.3 (E-02 verification) — Pro `.deb` on a clean Ubuntu 22.04 VM without the runtime: does
  Pdfium load? Manual, needs a VM; cannot run from this session. Static check (2026-10-10):
  CONFIRMED. `resources/lib/linux-x86_64/libpdfium.so` is 49 bytes of ASCII text (also
  `libonnxruntime.so` 54 B and the runtime-pack copies 51/56 B); `tauri.linux.conf.json` bundles
  `resources/lib/linux-x86_64/**/*`; `release.yml` runs `fetch-pdfium.sh` only for Lite (the Pro
  Linux job ships the fixture, comment at `release.yml:92`); `ocr/pdf.rs:368-376` picks the first
  candidate that merely `exists()`, binding fails, the system fallback is absent on a clean Ubuntu,
  so text degrades to lopdf and page render/thumbnails/OCR fail. Fix belongs to 6.1: fetch the real
  Pdfium for Pro Linux, move the fixtures out of the bundle globs, reject non-ELF/tiny files in the
  resolver. The VM run remains the runtime proof.
  Runtime check (2026-10-10, clean `ubuntu:22.04` container instead of a VM): the published
  `EntropIA.Pro_1.0.19_amd64.deb` installs and ships only the text fixtures (`dpkg-deb -c`: 49 B
  `libpdfium.so`, no `resources/pdfium/`); `dlopen` of the bundled file fails with "file too short"
  and the system has no `libpdfium.so`. The Linux dev build logs the same failure:
  `[pdf] Failed to load pdfium from resolved path (.../resources/lib/linux-x86_64/libpdfium.so)`.
  E-02 exists; the fix is 6.1. Not run: the GUI of the installed .deb (container has no display).

## Phase 1 — Critical security (small changes)

- [x] P1.1 (S-03) — renderer cannot overwrite the runtime bootstrap trust source. Branch
  `fix/s03-runtime-bootstrap-trust`, commit `fdaf827b`. RED (fix neutralized, Pro): 4 new tests failed
  (set/delete refused, built-in source wins, built-in key never replaced). GREEN: Pro
  `settings::` + `runtime::manager::` 70/70, Lite `settings::` 22/22; clippy Lite clean.
  Note: the plan's step "ACL test" is n/a here (the guard is in the command body, covered by unit
  tests). `db_execute*` can still write `app_settings` until S-02 (Phase 3); release precedence is
  what neutralizes that path meanwhile.
- [x] P1.3 (S-04) — `prosemirror-view` >= 1.42.3. Branch `fix/s04-prosemirror-view`, commit `58dcfa53`.
  Root `pnpm.overrides` → 1.42.6; `pnpm dedupe` leaves one `prosemirror-model` (1.25.12). RED:
  `pnpm audit --prod` listed prosemirror-view (high); GREEN: gone (high 9 → 8). Frozen install ok;
  UI 847/847 (incl. writing-image-paste); desktop Pro 3488 passed / Lite 3467 passed (247 files
  each); typechecks clean. Manual (Linux, same session as S-01): HTML pasted from a web page and
  Word-style HTML keep headings, bold/italic, links and lists; a pasted `<img onerror=…>` probe
  left no image and ran no script.
- [x] P1.2 (S-01) — narrow the `fs` plugin scope. Branch `fix/s01-fs-scope`, commits `e751aaf8`
  (frontend never deletes files outside the archive) and `43cdd6c3` (runtime scope). Inventory: every
  frontend plugin-fs path is a dialog pick, a drop, or an archive subdirectory; no frontend flow reads
  Zotero paths or `source_directory` through plugin-fs (the backend does). Design: setup grants
  `assets`, `writing-images`, `writing-crops`, `temp`, `sample-staging` and cache `audio-previews`
  via `fs_scope::grant_frontend_fs_scope` (path_utils-derived, covers dev profiles) and forbids
  `entropia.sqlite{,-wal,-shm,-journal}`; capability keeps only `deny` globs `**/*.sqlite*`, drops
  the four `*-read-recursive` sets, adds `fs:allow-stat`; dialog/drop paths are added to the scope
  by tauri-plugin-dialog 2.8.1 / tauri-plugin-fs 2.6.0 at runtime. RED: `app_acl` home read and
  shared-root DB path answered "No such file" (in scope) and archive dirs were forbidden; manifest
  guard failed on the old capability; 3 file-import tests failed. GREEN: `app_acl` 15/15 and
  `acl_manifest_guard` 7/7 (Lite and Pro), Lite lib 2447 passed, desktop JS Pro 3493 / Lite 3472
  passed (247 files), lint 0 errors, Lite typecheck 0 errors, clippy Lite clean, Pro clippy only the
  9 known pre-existing errors. Writer subagent failed twice before any tool call (model error); the
  work was done inline as the reported fallback.
  - Manual check on Linux (2026-10-10, Lite debug build of `fix/s01b-zotero-data-dir-grant`, dev
    profile `audit-s01`, Xvfb + xdotool, native GTK dialogs): OK with no scope errors in the log —
    sample collection seeded at first start; import via dialog (PDF); drag-drop of a PDF, a PNG and
    a WAV; audio preview (`audio-previews/` written and played); writing image via picker, clipboard
    paste and drop (`writing-images/`); DOCX export with both images and collection JSON export via
    save dialog; delete item; delete collection. External path: an asset row repointed to a PDF
    outside the archive (with a `.pages` sibling) was deleted from the UI; DB rows and the in-archive
    item folder went away, the external PDF and its `.pages` stayed. Dropping a folder answers
    "Formato no soportado": pre-existing (no frontend code reads directories), not a regression.
    Not tested: dictation (no audio device, needs an API key), RAG chat export, Windows.
- [x] P1.2b (S-01 plan step 5) — backend-granted Zotero data dir. Branch
  `fix/s01b-zotero-data-dir-grant`, commit `df91b3bb`. `ZOTERO_DATA_DIR_SETTING_KEY` is now
  `backend_grant.zotero_data_dir`; `settings_set`/`settings_delete` refuse the `backend_grant.`
  prefix and the legacy `zotero_data_dir`; new command `zotero_data_dir_grant` (no path argument:
  native picker in Rust, canonicalized, requires `storage/` or `zotero.sqlite`), registered in
  build.rs, the capability and `generate_handler!`; a legacy row is never read and is logged at
  startup. RED: 4 of 5 new tests failed against a stub; GREEN: Lite `cargo test` 2722 passed / 0
  failed, Pro `settings::`+`zotero_data_dir` 40/40, ACL suites green in both variants, clippy Lite
  clean, Pro clippy no new errors. No UI calls the command yet (none bound the old key either).
  Plan step 6 (`import_copy_into_archive`) stays unneeded: no frontend fs read of Zotero paths or
  `source_directory` exists.

## Review follow-ups (branch `fix/audit-review-followups`)

RDD approved #26, #27, #28 and #29; these act on their non-blocking advisories.

- [x] F1 (#28 R3-003) — `isOutsideDataDir` fails closed before `primeDataDir()`: an unproven path
  is treated as external, so `deleteAssetFile` and the `.pages` cleanup guards never act on it.
  Commit `df06541a`. RED: 2 new expectations failed (remove called / `false` before priming);
  GREEN: file-import, CollectionView, ItemView and layout tests 545/545. The `deleteAssetFile`
  suite and WorkPane's asset-delete suite now prime the data directory (they relied on the
  fail-open window).
- [x] F2 (#27) — bound the `prosemirror-view` override to `^1.42.3` so a future major is not
  pulled in silently. Commit `461af098`. Frozen install ok; still one version (1.42.6);
  `pnpm audit --prod` lists no prosemirror-view.
- Checks: desktop Pro 3494 passed / 7 skipped (247 files, x3); typechecks clean in both variants;
  eslint clean. Lite: 3472 passed, but `route-loader.test.ts` (Biblioteca routes) times out at
  5 s under the full parallel suite in 3 of 4 runs; it passes alone with and without these
  changes — pre-existing flake, follow-up.
- No action: #28 R3-001 (the archive root is never granted; `forbid_file` covers dialog/drop
  grants), #28 R3-002 (cache thumbnail/`.pages` may orphan when the asset delete throws;
  best-effort cleanup), #27 manual paste check (user).

## Later phases

- [x] Phase 2 — vulnerable dependencies (D-01, D-02, D-08, D-03, D-04). Baseline `pnpm audit --prod`:
  8 high / 11 moderate / 1 low. One stacked branch per item, on `fix/audit-review-followups`.
  - [x] 2.1 (D-01) — replace `html-docx-js` with `docx` in the OCR export (removes `lodash.merge`,
    `jszip` 2.7). Branch `fix/d01-ocr-docx-export`, commit `10b157a8`. New `ocr-docx.ts` reads the
    sanitized OCR HTML into the manuscript `Node` model and builds it with `toDocx`; the `<script>`
    loader and `types/ocr-export-libraries.d.ts` are gone; `export-docx.ts` now honours
    `colspan`/`rowspan` and optional page margins (OCR keeps 720 twip; page size is docx's A4 like
    the manuscript export, html-docx-js wrote Letter). RED: 13 failed (no OOXML structure, no
    gridSpan/vMerge, pgMar 1440); GREEN: focused 96/96; desktop Pro 3507 / Lite 3486 passed,
    UI 847/847, lint, both typechecks and both `vite build`s clean. Audit: high 8 → 6,
    moderate 11 → 10; `docx` brings `jszip` 3.10.2 (patched).
  - [x] 2.2.1 (D-02) — `svelte` >= 5.55.7, `devalue` >= 5.9.3. Branch `fix/d02-svelte-devalue`,
    commit `0b416f08`: svelte 5.55.3 → 5.56.10, devalue 5.7.1 → 5.9.4 (no override needed).
    5.55.7-5.55.9 leave the TS `?` on optional parameters in the compiled output (fixed in
    5.56.x, sveltejs/svelte#18448), which broke 6 UI test files with `Expected ',', got '?'`;
    the ranges (including the UI peer) are `^5.56.10` so installs cannot land there. Checks:
    frozen install ok; store 374, UI 847, desktop Pro 3507 / Lite 3486 passed; svelte-check 0
    errors (UI, desktop Pro and Lite); lint clean. Audit: high 6 → 3, moderate 10 → 3, low
    1 → 0; no svelte/devalue advisory left. The verifier subagent failed twice before any tool
    call, so these checks ran inline.
  - [x] 2.2.2 (D-02) — `markdown-it` >= 14.3.1, `linkify-it` >= 5.0.2 (override). Branch
    `fix/d02-markdown-it`: desktop dep and root override `^14.3.1` (caret keeps the breaking
    15.x out); resolves markdown-it 14.3.2 and linkify-it 5.0.2 (required by markdown-it 14.3,
    no own override). 14.2/14.3 changes are parse/security fixes; our only direct use
    (`ocr-rich-text.ts`) runs with `linkify: false`. Checks: frozen install ok; desktop Pro
    3507 / Lite 3486, UI 847 passed; both typechecks 0 errors; lint clean. Audit: high 3 → 1
    (drizzle-orm), moderate 3 → 1 (@tiptap/core).
  - [x] 2.2.3 (D-02) — `drizzle-orm` 0.40 → >= 0.45.2. Branch `fix/d02-drizzle-orm`: `^0.45.2` in
    `packages/store` and `apps/desktop` (prebundle copy), resolves 0.45.4; no drizzle-kit in
    the repo. Advisory GHSA-gpj5-g38j-94v9 (identifier escaping; our identifiers are static).
    Release notes 0.41-0.45 checked: nothing touches sqlite-proxy + classic CRUD +
    `.returning()`; 0.44's `DrizzleQueryError` wrapper does not affect
    `isAssetOrderSnapshotConflict` (raw-client errors). Checks: frozen install; store 374/374
    (real `node:sqlite` through the proxy), store tsc clean; desktop Pro 3507 / Lite 3486;
    typechecks 0 errors; Lite `vite build` ok; lint clean. Audit: high 1 → 0; only
    `@tiptap/core` (moderate) remains.
  - [x] 2.2.4 (D-02) — Tiptap 2 → 3: **accepted risk** for this cycle (user decision,
    2026-10-10). Advisory GHSA-cp6q-959q-f8rh (moderate, `@tiptap/core` >=2.0.0-alpha.0
    <3.30.4): `mergeAttributes()` assigns an own `__proto__` key with bracket assignment, so
    the merged object gets an attacker-controlled prototype that ProseMirror's `renderSpec()`
    then copies into DOM attributes (`for...in`). Exposure check: our four `mergeAttributes`
    calls (`WritingEditor/extensions.ts:120,191,267,348`) merge literal objects with
    `HTMLAttributes`; Tiptap builds `HTMLAttributes` keyed by schema attribute names, every
    custom attribute `renderHTML` (`paragraph-format.ts`, `font-size.ts`, `highlight.ts`,
    `text-color.ts`) returns a literal object from parsed values, and ProseMirror drops
    attributes the schema does not define. No path found for a document-supplied `__proto__`
    own key; residual risk is Tiptap's own internals and third-party extensions. Revisit with
    the Tiptap 3 migration (major: custom footnote, image and citation extensions in
    `packages/ui`). It is the only advisory left in `pnpm audit --prod`, so the plan's
    `--audit-level high` gate passes.
  - [x] 2.3 (D-08) — Rust advisories (`cargo audit`). Branch `fix/d08-rust-advisories`. Baseline
    (cargo-audit 0.22.2): 6 vulnerabilities, 12 warnings. Now 0 vulnerabilities from
    `apps/desktop/src-tauri`, 6 warnings (5 crates), all from third-party crates.
    - `fee1634f`: lockfile rustls 0.23.45, crossbeam-epoch 0.9.21, quinn-proto 0.11.19, plist
      1.10.1 (Tauri's plist moves to quick-xml 0.42).
    - `e1415527`: pdf-extract 0.7 → 0.12, so lopdf 0.34 (RUSTSEC-2026-0187, stack overflow
      that aborts the process) leaves the tree; no subprocess isolation needed. RED: a page
      with an array nested 100 000 deep aborts the test binary (SIGABRT) on 0.7; GREEN on
      0.12. 0.12 now reads the type-4 tint and inline-image fixtures, so the panic-containment
      tests use a generated missing-colour-space PDF. RDD `review-af34c0532f9aa22b` approved
      (both commits).
    - `2580a114`: lockfile imageproc 0.25.1 (3 bounds-check warnings), anyhow 1.0.104, rand
      0.8.8, aes 0.9.3 (0.9.0 yanked). New `.cargo/audit.toml` ignores RUSTSEC-2026-0194/0195
      (quick-xml 0.38.4 via hayagriva → citationberg 0.7.0, latest releases; user decision,
      exposure: custom .csl ≤ 2 MB on a blocking thread) and documents, without ignoring, the
      5 remaining crates: glib + proc-macro-error (Tauri GTK3), core2 (image AVIF), paste
      (hayagriva, rav1e), ttf-parser (lopdf 0.42, imageproc). cargo-audit reads the file from
      the working directory only (C-03 must run it from `apps/desktop/src-tauri`).
    - Checks: fmt; Lite clippy `-D warnings` and `cargo test` 2724 passed / 0 failed / 34
      ignored; Pro clippy no new findings (9 known in `deps/uv.rs`, `llm/download.rs`), Pro
      `ocr::` 216 and `image_edit` 18 passed.
  - [x] 2.4 (D-03) — unused/duplicated deps. Branch `chore/d03-deps-cleanup`.
    - `9448104d`: drop `@tauri-apps/plugin-sql` (no TS import, no Rust plugin, no capability).
    - `50f7a9b2`: the 17 `@tiptap/*` packages and `leaflet` use a pnpm catalog
      (`pnpm-workspace.yaml`), referenced as `catalog:` by `packages/ui` and `apps/desktop`.
      The plan's other option (desktop stops declaring them) was rejected: desktop redeclares
      them so Vite prebundles one copy from its own root; without that a second
      `prosemirror-state` loads (`vite.config.ts` comment). `AGENTS.md` gains a Dependencies
      section: the catalog rule, that redeclaration rule, and why `lib/markdown.ts` (untrusted
      LLM output, HTML escaped first) and `markdown-it` (OCR only, sanitized by
      `sanitizeOcrHtml` at all three call sites) coexist.
    - Checks: lock keeps every resolved version (only plugin-sql removed); one version per
      `@tiptap/*` and leaflet 1.9.4 only; `pnpm build` ok; store 374, UI 847, desktop Pro 3507
      passed; Lite 3485 + 1 failed: the known `route-loader.test.ts` 5 s timeout (passes alone
      with and without the change; fixed by PR #31, which is not in this stack); typecheck 0
      errors; lint clean.
  - [x] 2.5 (D-04) — test tooling (vitest, happy-dom, vite). Branch `chore/d04-test-tooling`,
    commit `ea592d4c`. Full `pnpm audit` (dev included) had 4 critical: happy-dom < 20, vitest
    < 3.2.6, tinypool <= 2.1.1 (x2). vitest 3.2.7 still uses tinypool 1.x, so vitest 4.1.11 (no
    tinypool; also clears vitest/@vitest/mocker < 4.1.11) + @vitest/ui, @vitest/coverage-v8
    4.1.11, happy-dom 20.14.6, vite 6.4.4. Configs were already vitest-4 shaped.
    - Surfaced and fixed in tests/config only: @types/node declared in store and UI (vitest 3
      had leaked it through its type references) and `"node"` in the UI tsconfig types;
      `vi.fn<() => void>()` in `transcription.test.ts` (vitest 4 mock typing); a no-op
      `window.prompt` in the UI test setup (gone in happy-dom 20); three desktop tests that
      asserted after waiting on the wrong signal (CollectionView, DependenciasTab, ItemView),
      exposed by happy-dom 20 timing. A bisect (vitest 4 + happy-dom 17) proved those three
      came from happy-dom, not vitest. No production code changed.
    - Checks: frozen install; audit 0 critical (`--prod --audit-level high` exit 0); store
      374, UI 847, desktop Pro 3507 passed; Lite 3485 + the known `route-loader.test.ts`
      timeout (passes alone; fixed by PR #31, not in this stack); typecheck 0 errors (UI,
      desktop Pro and Lite, store); lint and `pnpm build` clean; coverage-v8 smoke ok.
    - Not done: unifying jsdom vs happy-dom (22 files force jsdom) — left as is. Remaining
      dev-only advisories (20 high): brace-expansion and js-yaml via the eslint chain,
      postcss/nanoid via vite, ws via jsdom, source-map-js via coverage; none ships. Follow-up
      for Phase 7 (C-03, supply chain).
- [x] Phase 3 — SQL IPC and asset protocol hardening (S-02, S-05, A-04, A-05, S-06)
  - [x] 3.1 (S-02) — SQLite authorizer on the UI connection (S-02a/b/c) + agent-run dev-profile
    check (S-02d). Branch `fix/s02-sql-authorizer`, PR #41.
  - [x] 3.2 (A-04) — Rust-side rollback in `db_execute_batch` + cascades on
    `db_execute_transaction`. Branch `fix/a-04-rollback-cascades`, commit `f6d5f709`.
    `execute_batch_on` rolls back on the locked connection when a failed statement leaves
    `!is_autocommit()` (backend SQL, no authorizer; the original error is returned). The
    runner's catch-ROLLBACK is unaffected: it already swallows the no-transaction error
    (`ROLLBACK;` with no open txn fails in SQLite, verified). The three cascades
    (item/asset/collection) run one `executeTransaction` with bound params — same order,
    same errors, no interpolation; item's `vec_assets` delete joins the txn when the table
    exists. Checks: Lite `cargo test` 2781 / 0 / 35; store 381; desktop Pro 3510 / Lite 3489;
    typecheck and lint clean. Left out (reported, not in plan scope): runner.ts:3173 keeps its
    catch-ROLLBACK (harmless), `deleteIfEmpty` in collection.repo.ts still interpolates one
    id in a single-statement batch. RDD `review-3cdc11f5789e7062` approved; PR #42.
  - [x] 3.3 (S-05) — asset protocol scope. Branch `fix/s05-asset-protocol-scope`, commit
    `1cbe7a10`. Inventory corrected the plan: thumbnails live in the CACHE dir
    (`ocr/commands.rs:497`), and the Navegador viewer opens the saved PDF from the archive's
    `web-captures/` (`pdf_capture_file`), never from `navegador/downloads` (that quarantine path
    never reaches the frontend); audio previews and dictation scratch go through the fs plugin.
    Served: `$DATA/…/{assets,writing-images,writing-crops,web-captures}/**` and
    `$LOCALDATA/…/thumbnails/**`; deny `**/*.sqlite*` and `**/web-captures/**/*.html`. One
    list in `asset_scope.rs`, granted from the resolved dirs at setup with `forbid_file` for the
    four database files (`forbid_file` cannot take globs; the config deny covers the rest).
    RDD `review-9b45b87bd84d5052` approved; PR #44.
    Tests: TS config equality + Rust-list alignment (4); mock-app post-setup scope (2). Checks:
    Lite `cargo test` 2783 / 0 / 35; desktop Pro 3514 / Lite 3493; typecheck, lint clean.
    Manual acceptance (agent-run, profile `s05-scope`, Lite + `navegador` feature, Xvfb):
    collection thumbnails, image and PDF asset in the item viewer, and a Navegador-saved PDF
    in its viewer all render; from devtools, `fetch(convertFileSrc(…))` returns 200 for the
    five served dirs and 403 for `entropia.sqlite`, `entropia.sqlite-wal`, capture HTML,
    `temp/` and `research/`. Stale wording left for later: `dev_profile.rs` module doc
    (configs "cover the whole shared roots") and `file-import.ts:~413` (thumbnails in
    app_data_dir).
  - [x] 3.4 (A-05, S-06) — no blocking `ui_conn` in async commands, no keyring I/O under
    `ui_conn`. Branch `fix/a05-s06-settings-keyring`, PR #45. RDD `review-aa0641b4fe7a6cbe`
    (high, four lenses, PR slice vs `fix/s05-asset-protocol-scope`) approved. Advisory
    follow-ups: `settings_get` probes the credential store for a log line under the key lock;
    empty-value `settings_set` may orphan a keyring entry; duplicated cloud-key check
    (`ocr/mod.rs`) and secret set sequence (`settings.rs`); hand-rolled guard scanner. Heuristic scan: 17 `async` commands lock
    `ui_conn` outside `spawn_blocking` (settings x4, llm x5, transcription x3, ocr x2, deps x2,
    navegador, nlp, writing publish, zotero_web).
    - [x] A-05a — `settings.rs`: commands on `run_blocking_db_task`; `get_setting` reads the
      ref under the lock and resolves the keyring after; `settings_set` writes the secret
      first without the lock; `settings_delete` deletes the row, then the secret; per-key
      `SETTINGS_KEY_LOCKS` (only the five secret keys, never pruned) with fixed order key →
      `ui_conn` → `APP_CREDENTIAL_LOCK`; injectable `SecretStore`; barrier concurrency test,
      RED with the key lock disabled (`row references missing credential`), 20x loop green.
      Commit `b9dedc2e`; Lite `cargo test` 2790 / 0 / 35.
    - [x] A-05b — the other commands (transcription, llm, ocr, nlp, deps, navegador, writing
      publish, zotero_web; runtime/manager and store_updates readers): SQL on
      `run_blocking_db_task`/`spawn_blocking`, secrets resolved after releasing `ui_conn`
      (`get_secret_setting_unlocked`, `resolve_api_key_input_unlocked`,
      `ensure_selected_cloud_key_unlocked`), no network call under the lock. `settings_set`
      resumes unblocked work on its own connection after the row write. Guard test
      `tests/async_lock_guard.rs`: 25 violations on the previous code, green after. Commit
      `b769fdb8`; Lite `cargo test` 2795 / 0 / 35, Pro focused 630 / 0 / 6, Pro clippy only
      the 9 known findings, desktop 3514. Accepted deviations: a poisoned lock in
      `extract_text`/`transcribe_*` now reads "DB lock error:" (no renderer matches it); the
      Windows-only `store_updates::native::check` call site was checked by reading (no
      Windows target here).
  - [ ] 3.1 (S-02) — SQLite authorizer on the UI connection. Branch `fix/s02-sql-authorizer`.
    Facts: rusqlite 0.31 without `hooks`; renderer SQL runs only on `ui_conn` through
    `db_execute`, `db_execute_batch`, `db_execute_transaction`, `db_select`, `db_select_rows`;
    `app_settings` is reached only through `settings_*` commands; 54 `trg_sync_*` triggers write
    `sync_oplog`; last migration `0058_processing_ner_tasks`; the only write pragma the runner
    emits is `defer_foreign_keys`. Split so every commit stays green:
    - [x] S-02a — authorizer core (layer 2) + comment-stripping text validators (layer 1).
      Commit `400028a3`: new `db/authorizer.rs` (pure `authorize` policy + RAII
      `RendererSqlAuthorizer`), command bodies factored into `*_on` functions. Plain `VACUUM`
      is denied too (SQLite checks SQLITE_ATTACH for it). RED 13 failed; GREEN Lite
      `cargo test` 2744 passed / 0 failed / 34 ignored (+20), `db::` 63, `sync::` 444; fmt,
      clippy `-D warnings` clean; Pro clippy only the 9 known findings. Capture through all
      three write commands still reaches `sync_oplog`. Migrations emit no other sensitive SQL
      (only `defer_foreign_keys`), so layer 1's comment stripping does not reject them.
      Active only while a renderer `db_*` statement runs (RAII guard on `ui_conn`). Denies
      ATTACH/DETACH (so VACUUM INTO), PRAGMA except `defer_foreign_keys` in `db_execute_batch`,
      any access to `app_settings`, writes to `sync_*` unless the accessor is a `trg_sync_*`
      trigger, and CREATE/DROP TRIGGER/VIEW outside `db_execute_batch` (batch keeps today's
      DDL until S-02c). Tests first: the audit's evasions, VACUUM INTO, and a write with
      capture on that must still land in `sync_oplog`.
    - [x] S-02b — lexical `split_sql_statements` (never prepares), the exported migration
      allowlist of SHA-256 hashes (`db/migration_ddl_allowlist.txt`), its generator and a
      consistency test. No behaviour change. Commit `1eeff356`: `db/sql_split.rs`
      (`sqlite3_complete` + code mask; deny-all authorizer sees 0 calls),
      `tests/fixtures/migration_ipc.json` (156 IPC calls of a fresh install through 0058, plus
      16 repair drops, recorded from the real `runMigrations` by
      `pnpm --filter @entropia/store export-migration-ipc`; store drift test), 56 fingerprints
      (49 CREATE TRIGGER, 6 DROP TRIGGER, PRAGMA) regenerated by an ignored Rust test with a
      consistency test. Lite `cargo test` 2759 / 0 / 35; store 377.
    - [x] S-02c — one-shot migration window (`NotStarted → Open → Closed`) with
      `db_migration_window_begin/end`, statement-by-statement `db_execute_batch`, the listed-hash
      exception for TRIGGER/VIEW DDL and `defer_foreign_keys`, frontend calls in `initDb`; tests
      (a)-(e) of the plan and a fresh install replaying the runner's IPC under the authorizer.
      Commit `4c5dce57`. Last migration name lives in the allowlist (`# last-migration:`), not
      in code. Deviations from the plan, reviewed: the runner-swallowed `duplicate column name`
      error does not close the window (0020 hits it on every fresh install); `data_version`
      and `quick_check` stay allowed in batch (SQLite issues them itself for FTS5 and table
      rebuilds; read-only); a DROP/ALTER TABLE inside the window may drop that table's own
      triggers — including its `trg_sync_*`, because setup installs capture before the JS
      migrations and the 0010/0019/0021 rebuilds of `entities`, `llm_results`, `annotations`
      would otherwise fail on an old archive (RED: "denied by the database authorizer").
      Webview reload: a refused `begin` is logged and startup continues. Checks: Lite
      `cargo test` 2780 / 0 / 35; app_acl 15, acl_manifest_guard 7; Pro `db::` 99; fmt, clippy
      clean (Pro: only the 9 known); desktop Pro 3510, Lite 3489 (+ known route-loader flake);
      store 377; typecheck and lint clean (3 pre-existing lint warnings in untouched files).
    - [x] S-02d — fresh install and main views in a dev profile (agent-run, 2026-10-10,
      profile `s02-fresh`, Lite `VITE_LOCAL_ML=0`, isolated Xvfb display). Startup line
      `profile=dev:s02-fresh ... sync=disabled`; archive shared untouched. Fresh DB migrated to
      `0058_processing_ner_tasks` (58 `_migrations` rows) with 54 `trg_sync_*` triggers, so
      `PRAGMA defer_foreign_keys` ran under the authorizer. Walked with mouse: Inicio,
      Colecciones (list), collection detail, item view (viewer + notes panel), Chat,
      Investigación and Escritura — all render, no error dialog, zero `app-log:error` and zero
      authorizer denials in the log. Notes: WebKitGTK needs `GDK_BACKEND=x11` (the shell's
      `WAYLAND_DISPLAY` made it pick Wayland and no window mapped) plus
      `WEBKIT_DISABLE_COMPOSITING_MODE=1` under Xvfb; the window starts hidden until
      `splash_finish`.
    - RDD, per slice: S-02a `review-66bda845393ff635` approved (no findings); S-02b
      `review-ee05776b5134c924` approved, reviewed WITHOUT the generated
      `tests/fixtures/migration_ipc.json` (164 KB exceeded the reviewer context budget; user
      decision; the fixture is covered by the store drift test and the Rust consistency test);
      S-02c `review-af5009826b333044` approved (high tier, 4 lenses). Both high tiers came from
      a false-positive "starts processes" signal on `migration_allowlist.rs` (it only prints
      `pnpm` commands in messages).
    - Follow-ups from the advisories (non-blocking): R4 a failed migration closes the window,
      so a reload cannot retry — tell the user to restart the app (`lib/db.ts`); R2 merge the
      duplicated DropTrigger arms and list `quick_check` in the policy doc table
      (`authorizer.rs`); R2 small readability notes in `migration_allowlist.rs:92-113` and
      `migration-ipc-recorder.ts:56-57,102-107`; R1 the rebuild exception drops capture
      triggers until `sync_ensure_capture` (by design, see S-02c).
- [x] Phase 4 — migrations unified in Rust (A-01, A-03, A-02). PR stack #46 → #48 → #49 → #50 →
  #51 → #52 → #53 → #54. Design:
  `odd/plans/plan-migraciones-rust.md` (invariants: identical `_migrations` names, no new rows,
  `schema_tag` unchanged, same canonical dump, `applied_at` in seconds, capture semantics kept).
  One stacked branch + PR + RDD per unit.
  - [x] 4.0 — design doc. Branch `feat/phase4-migrations-design`, commit `b09f6490`, PR #46.
  - [x] 4.1a — `.sql` files byte-identical to the registry. Branch
    `feat/p4-1a-migration-sql-files`, commit `81e0a034`. Found 7 missing and 19 differing files;
    normalized, only 0020 differed in SQL (its file lacked `DEFAULT '[]'` on `layouts.blocks`).
    The registry wins: files rewritten, header comments dropped, 0020 holds `LAYOUTS_DDL`.
    `migration-files.test.ts` RED 27 → GREEN. RDD `review-b166c9500d706937` approved.
  - [x] 4.1b — TS registry generated from the `.sql` files (`generate-migrations.mjs` →
    `migrations.generated.ts`), rationale comments to `migrations/NOTES.md`, freshness test.
    Commit `b68f30e9`; runner.ts 3182 → 418 lines. Oracles byte-identical: `migration_ipc.json`
    and `schema_full.sql`. Store 441, desktop 3514, Rust 2795 / 0 / 35. RDD over budget with the
    144 KB generated module; by user decision reviewed as a reduced candidate without it (temp
    branch, deleted): `review-e05a9ec28196939a` approved, 3 advisory warnings at line 1 of
    `runner.ts`, `migration-files.test.ts` and `generate-migrations.mjs` (text not stored;
    likely the `.ts`-extension import with `@ts-expect-error`).
  - [x] 4.2a — canonical dump. Branch `feat/p4-2a-canonical-schema-dump`, commit `4ab39b67`.
    `schema-canonical.ts` + `export-schema-canonical` → `tests/fixtures/schema_canonical.json`
    (298 KB; 215 objects, 82 tables, 88 indexes, 58 migrations) with a freshness test;
    `db/canonical_dump.rs` computes the same structure. Cross-check: `schema_full.sql` under rusqlite
    gives the same dump, PRAGMAs included (node:sqlite 3.53.4 vs bundled 3.45.0), except that a real
    archive's `_migrations` and `layouts` carry runner.ts's runtime DDL text — 4.2b must reproduce it.
    Store 445, Rust 2798 / 0 / 35. RDD `review-9ed7ebd508ea9dd9` approved, no findings, reviewed
    without the generated fixture under the user's standing Phase 4 rule (generated,
    freshness-pinned files over the budget are excluded via a temp branch).
  - [x] 4.2b — `db/migrations.rs`, not wired. Branch `feat/p4-2b-rust-migration-runner`, commit
    `60b9456f`. `include_str!` registry (58; 27 atomic, pinned against runner.ts), runtime
    `_migrations`/`layouts` DDL text shared with canonical_dump, every migration in `BEGIN IMMEDIATE
    … COMMIT` with its row (A-03), `applied_at` in seconds, per-statement `duplicate column name`
    tolerance only where TS had it, 0020 and the three 0032 cases ported, `MigrationError` instead
    of panics. Empty DB == `schema_canonical.json` exactly (RED with a wrong `_migrations` text).
    13 tests; Rust 2811 / 0 / 35; Pro clippy only the 9 known. Accepted divergences: all migrations
    transactional; lexical splitting. RDD `review-95a9008a2b9c8fc5` approved, no findings.
  - [x] 4.2c — Rust applies the migrations at startup. Branch `feat/p4-2c-wire-rust-migrations`.
    - `b2216048` fix: FTS5's constructor `PRAGMA data_version` (no value) allowed for Single
      renderer statements. Found in the dev-profile run: on a fresh archive the first FTS5 use on
      `ui_conn` is now a `db_select`, which S-02 denied ("Restricted SQL statement" on Inicio,
      sample collections half seeded). Integration test RED "vtable constructor failed".
    - `84be8c1b`: `apply_migrations` on `ui_conn` after the schema repairs, before
      `worker_conn`/`AppDbState`/capture; `MigrationError` → `fail(...)` with SQLite's message
      only (rusqlite's appended migration SQL dropped, test RED first); `[setup] migrations:` log.
      Upgrade 0033→0058 on 10k items: 248 ms (0034 FTS rebuild 222 ms).
    - Dev profile (Lite, Xvfb): fresh install applied 58, 54 `trg_sync_*`, 3 sample collections,
      no error; existing archive with 0058 pending applied 1, data intact; forced 0058 failure →
      `fail()` reached, logged, nothing committed (57 rows). **For 4.3:** the GTK dialog of
      `fail()` never maps under Xvfb (process stays alive behind the splash) — likely pre-existing
      on Linux, since `fail()` is unchanged; verify and fix with 4.3's acceptance.
    - Rust 2814 / 0 / 36; Pro clippy only the 9 known. RDD `review-3461ac275bca68c2` approved.
  - [x] 4.3 — `setup` patches as idempotent repairs (A-02). Branch `feat/p4-3-setup-repairs`.
    - `3651d2ea` (4.3a): setup's dialogs (`fail()` and "EntropIA ya está abierta") used
      `blocking_show` on the main thread; on Linux rfd queues the GTK dialog on the GLib context
      that thread owns and then blocks it, so the dialog never mapped and the app hung behind the
      splash. `show_setup_dialog` shows it non-blocking and iterates the context until dismissed
      (`glib` 0.18, already in the lockfile). Verified both dialogs appear and OK exits.
    - `dd0bfe3e` (4.3b): `db/repairs.rs::migrate_archive` = pre repairs (dedup + unique indexes,
      method CHECK, llm_results, 0017 fallbacks, app_settings) → `apply_migrations` → post
      (`ensure_layouts_schema`, then the pre repairs as no-ops). Dedups inside
      `without_sync_capture` (moved to `sync/capture.rs`; RED: one 'D' oplog row). All setup
      `.expect` → `fail(...)`. Empty-archive oracle = fixture + exactly the two backend-owned
      tables (`app_settings`, `ocr_correction_backups`), which every archive had since first launch.
    - Dev profile: fresh install (58, 3 samples), existing archive (up to date), read-only DB →
      error dialog instead of a crash, OK exits. Rust 2822 / 0 / 36; Pro clippy only the 9 known.
      RDD `review-4646c0a48d9ac6c3` approved, no findings.
  - [x] 4.4a — renderer verify-only, DDL closed in the IPC (A-01). Branch `feat/p4-4a-close-ddl`.
    - `8a24eeca`: `runMigrations` verify-only (throws naming missing migrations; unknown extra names
      ignored so a downgraded app starts); the applier becomes `applyMigrations` for tests/tooling;
      `initDb` stops calling the migration window. Oracles byte-identical. RDD
      `review-51766dc9c3053257` approved.
    - `7bc09cfd`: authorizer = one policy for every renderer command (all CREATE/DROP incl. TEMP and
      VTABLE, ALTER, REINDEX, ANALYZE denied; only value-less `data_version` pragma allowed);
      layer-1 rejects a leading CREATE/DROP/ALTER after comment stripping on every command; removed
      `MigrationWindow`, the window commands, `migration_allowlist.rs` + DDL fingerprint list, the
      S-02b recorder and `migration_ipc.json`. `DROP TABLE items` RED (returned Ok) → rejected on
      batch/execute/transaction. Renderer DDL grep: none. Rust 2798 / 0 / 35 (−49 removed, +24),
      store 446, desktop 3513. RDD `review-dab76603c00a09fa` approved, reviewed without the deleted
      generated `migration_ipc.json` (standing Phase 4 rule); the full slice and the combined
      candidate exceeded the budget, so each commit was reviewed alone.
    - Dev profile: fresh install seeds the samples through renderer DML; Inicio, collection,
      item, FTS search, Escritura, Investigación, Chat, Colecciones render; no authorizer/DDL
      errors in the log.
  - [x] 4.4b — canonical trigger/view audit at startup. Branch `feat/p4-4b-trigger-view-audit`,
    commit `c1f7646f`. `db/schema_audit.rs` builds the canonical schema in memory (apply_migrations
    + asset-path guard triggers + ensure_capture: 45 migration triggers, 54 `trg_sync_*`, 2 guards,
    0 views; no other production creator), compares every trigger/view by (type, name) with exact
    tbl_name + sql, drops divergent/extra ones and recreates canonical ones (`trg_sync_*` via
    `drop_all_sync_triggers` + `ensure_capture`) in one transaction, before the renderer gets
    `ui_conn`; findings to `app_logs` (warn). Canonical build ~65 ms per startup (debug). Tests
    (a)-(d) + view, clean, idempotent, guards survive; RED with the repair short-circuited. Rust
    2808 / 0 / 35. Dev profile: a copy of a real archive with a planted trigger, a planted view and
    a hijacked `collection_activity_items_update` → 3 dropped, 1 restored byte-identical, next start
    "ok". RDD `review-d44fe8c944ffb916` approved, no findings.
- [x] Phase 5 — performance and observability (P-01..P-04). One stacked branch + PR + RDD per unit.
  PRs #55 → #56 → #57 → #59 → #60 → #61 → #62 → #63 → #64. 5.2b deferred by user decision.
  - [x] 5.1 (P-02) — app log bounded and streamed. Branch `fix/p5-1-app-log-rotation`, commit
    `b3c11a50`. `load_existing_entries` streams with `read_until` (corrupt and non-UTF-8 lines
    skipped one by one; before, one bad byte made `read_to_string` fail and the whole history
    vanish), a file over 2,000 lines is rewritten at startup, appends compact once the file passes
    4,000 lines, and `rewrite_log_file` goes through a synced temp file + rename. Tests RED first:
    50,000-line log → 2,000 entries and 2,000 file lines; corrupt/non-UTF-8 lines; append
    compaction. Rust 2811 / 0 / 35; Pro clippy only the 9 known. RDD `review-1b455d8c3d432daa`
    approved, no findings.
  - [x] 5.3 (P-03) — streaming SHA-256 for the runtime pack. Branch
    `fix/p5-3-streaming-runtime-hash`, commit `529d36b3`. `runtime/hash.rs::sha256_file_streaming`
    (1 MiB chunks, `local-ml` only like the rest of the module) replaces the whole-file reads in
    `download.rs` (post-extraction check) and `manager.rs::file_sha256`. Behaviour-preserving: the
    66 runtime tests pass with `local-ml`, plus helper tests vs the one-shot digest (empty, small,
    multi-buffer) and a missing-file error. Lite 2811 / 0 / 35 unchanged; Pro clippy only the 9
    known. RDD `review-89473222c0de6219` approved, no findings.
  - [x] 5.2a (P-01) — RAG partial selection. Branch `fix/p5-2a-rag-partial-selection`.
    - `a545a7c0`: `rank_assets_with_text` selects the top limit×4 (score desc, scan index asc),
      sorts only that window, walks it with the text filter and falls back to the old full stable
      sort when fewer than `limit` survive. Exact equivalence vs the old algorithm (60 randomized
      cases, ties at the boundary, forced fallback, limit > set, empty); RED vs a fallback-less
      stub. Ranking cost (measure profile, dim 1024): 10k 626→210 µs, 50k 3.17→0.98 ms, 200k
      15.8→4.0 ms.
    - `75a7c02e`: found while reviewing — a stored vector with NaN/inf scored NaN, ranked first
      (RED: ["asset-nan","asset-inf","asset-ok"]) and breaks the comparator's total order (Rust
      ≥1.81 sorts may panic). `score_embeddings` now skips non-finite scores (protects
      `vector_leg` and `chunk_vector_leg`).
    - Lite 2818 / 0 / 36; Pro clippy only the 9 known. RDD `review-ed73671f5cfbf6be` approved.
  - [~] 5.2b (P-01) — cached vector index: **deferred by user decision** after measuring.
    - End-to-end `vector_leg` on an on-disk archive (dim 1024, measure profile, median of 5):
      10k assets 34 ms, 50k assets 172 ms per question; almost all of it is reading the blobs
      (ranking is ~1 ms since 5.2a). A cache would bring it to ~1 ms.
    - Cost: an exact cache keeps f32 vectors resident, ~40 MB per 10k assets; the bibliography's
      int8 `vecscan` index is a quarter of that but only matches an exact scan when the true top-k
      lands in its shortlist, so it cannot meet the plan's same-ids-same-order acceptance.
    - Benefit: 30-170 ms on a question that then waits seconds for the LLM; `vec_assets` holds one
      row per document, so 50k is already a large archive. Not worth the resident memory now.
    - If revisited: invalidate with a SQLite update hook in `open_archive_connection` bumping a
      process-global generation on any `vec_assets` row change (user decision: no migration,
      `schema_tag` unchanged). Verified every production write connection goes through
      `db/open.rs` (`research*` opens another DB; `rag/baseline.rs` is read-only tooling). Plan
      test: an in-place embedding update is reflected by the next query.
  - [x] 5.4 (P-04) — unified backend logging, incremental. One unit per area. eprintln 387 → 1.
    - [x] 5.4a — facade + guard + setup. Branch `feat/p5-4a-backend-logging`, commit `ebc40ecf`.
      `logging.rs` (`log` 0.4, already in the lockfile): stderr `[<target>] <message>`
      byte-identical to the old eprintln lines; our crate's warn/error also persisted to
      `app_logs` once attached; dependency records stderr-only at Warn+. lib.rs, db/repairs.rs,
      db/schema_audit.rs migrated. Parent review caught and fixed two regressions before commit:
      paired eprintln+app_logs sites had lost their persisted Spanish entries (8 restored
      byte-for-byte), and dependency warnings would have reached the user's log. Ratchet
      `tests/eprintln_ratchet.rs` + baseline; eprintln 387 → 330. Lite 2828 / 0 / 37; Pro clippy
      only the 9 known. Dev profile: stderr format and persisted entries unchanged. RDD
      `review-39fda39543844f3f` approved, no findings.
    - [x] 5.4b — sync. Branch `feat/p5-4b-sync-logging`, commit `0f44e40f`. 11 eprintln → `log::`
      with target `sync` (engine start failures → error, now persisted to the user's log;
      asset-file removal/cleanup and pulled-page FK diagnostics → warn; engine ready/shutdown →
      info). No paired app_logs sites. eprintln 330 → 319. Lite 2828 / 0 / 37. Not observable in
      a dev profile (engine disabled there). RDD `review-859cdbb6c24b2037` approved, four lenses
      (high tier: `sync/apply.rs` starts processes), no findings.
    - [x] 5.4c — deps and Python discovery (runtime had no eprintln left). Branch
      `feat/p5-4c-runtime-deps-logging`, commit `d07e55d3`. 52 sites: 49 → `log::` with the old
      prefix as target (per-candidate probes stay info; final "no Python with module X" is a
      cached warn), 3 paired `deps_reset` eprintlns deleted keeping the Spanish `app_logs::warn`
      byte-for-byte. eprintln 319 → 267. Lite 2828 / 0 / 37; Pro `deps::`/`python_discovery::`
      73 pass. RDD `review-c761c3a3117587a9` approved, no findings.
    - [x] 5.4d — OCR. Branch `feat/p5-4d-ocr-logging`, commit `09dbe5b7`. 95 sites: warn 22,
      info 49, debug 20 (`debug_viz.rs`, debug-only), 4 paired PaddleOCR-VL eprintlns deleted
      keeping the Spanish `app_logs::warn`. Parent review moved per-region/per-page pipeline
      decisions (tiny region skipped, layout bypassed, low-confidence rotation skipped) from warn to
      info so one page cannot flood the user's log. eprintln 267 → 172. Lite 2828 / 0 / 37; Pro
      `ocr::` 216 pass. RDD `review-081f1c63296dc86f` approved (four lenses, high tier:
      `debug_viz.rs` starts processes). Advisories: test-only skip messages now go through `log`,
      which has no logger under `cargo test`, so they no longer print (fix with one shared test
      logger in 5.4f); a level/icon mismatch in `pdf.rs:272`; an inconsistent target in
      `mod.rs:1241`; `debug_viz` dump failures at debug level.
    - [x] 5.4e — llm, nlp, transcription, rag. Branch `feat/p5-4e-ml-logging`, commit `21d346c5`.
      102 sites; compound prefixes (`[llm-local][correction]`) via a `log_target` helper with a
      regression test; 5 paired transcription eprintlns deleted keeping `app_logs`. Parent review
      moved an undecodable audio packet (per packet) and auto-title discarded (per conversation)
      from warn to info. No API key at any level. eprintln 172 → 70. Lite 2829 / 0 / 37; Pro the
      four modules 478 pass. RDD `review-da6064ce226ef5e5` approved, no findings.
    - [x] 5.4f — the rest. Branch `feat/p5-4f-remaining-logging`, commit `45c261f1`. 68 production
      sites migrated, 2 paired eprintlns deleted keeping `app_logs`; only `app_logs.rs`'s own echo
      remains (eprintln 70 → 1, baseline = `app_logs.rs 1`). Parent added rate limiting where
      persisted warnings would repeat: the scheduler retry loop (warn on the first failure of a run,
      info until it recovers) and unresolvable secret settings (warn once per key per session).
      Test-only diagnostics use `println!` instead of a test logger (captured by the harness, shown
      with `--nocapture`). Lite 2829 / 0 / 37; Pro lib 2791 / 0 / 14. RDD `review-74260b4df6fedb86`
      approved (four lenses, high tier: `store_updates.rs` starts processes). Advisories: the
      per-key set never resets within a session; identical scheduler match arms; doc wording of the
      "repeats go to stderr" comments (true: release max level is Info).
- [x] Phase 6 — packaging and release (E-02 fix, E-01, E-03, C-02, C-04, D-07, E-04) — PRs #65–#70, #72, #73
  - [x] 6.1 — real Pdfium in the Pro Linux .deb (E-02). Branch `fix/p6-1-pro-linux-pdfium`,
    commit `01107569`. The Pro ubuntu leg of `release.yml` runs `fetch-pdfium.sh linux-x64` and
    bundles through a new explicit overlay `tauri.pro.linux.conf.json` (the Linux resource list
    plus `resources/pdfium/{libpdfium.so,LICENSE}`). Pdfium is deliberately NOT in
    `tauri.linux.conf.json`: Tauri merges that file into every Linux build, `cargo test` included,
    and fails on a missing resource (`cargo check` with `resources/pdfium` removed was the probe).
    The 49/54-byte text stubs moved to `tests/fixtures/resource-audit/*.fixture`; the empty
    `resources/lib/linux-x86_64/**/*` glob left both Linux configs (a zero-match glob fails the
    bundle; user approved). The resolver in `ocr/pdf.rs` skips non-native candidates (ELF/PE/
    Mach-O/fat magic and >= 1 MiB) and falls through. Checks: fmt, Lite clippy clean, Lite
    2833 / 0 / 37, Pro clippy only the 9 known, Pro `ocr::pdf` 94 / 0 / 3, desktop 3515 / 0 / 7
    (first run hit the known flaky timeout), lint 0 errors. RDD `review-f3308e788c8c0ff4`
    approved (high tier, four lenses: shell in `release.yml`; the first group run died with
    "Stream ended without finish_reason", the reoffered group passed). Advisories: the 1 MiB
    floor is duplicated between the test helper and `pdf.rs` (suggestion); the fixtures README
    named the wrong config (fixed in the docs commit). Not run: a real `tauri build` of the Pro
    `.deb` (MNN compile, CI covers it).
  - [x] 6.2 — Lite Linux without the Pro payload (E-01). Branch `fix/p6-2-lite-linux-resources`,
    commit `321cda5f`. `tauri.lite.linux.conf.json` lists only `provider-compatibility.json`,
    `lib/LICENSE`, `fonts/LICENSE` and the fetched `pdfium/{libpdfium.so,LICENSE}`.
    `lite-preview.yml` (also the release's Lite .deb) now fails if the package lacks
    `libpdfium.so` or holds `models/`, `runtime-pack/`, `tools/uv/`, `*.py` or `*.dll`.
    `lite-unix-bundle-config.test.ts` had pinned E-01 itself (Lite must keep every Pro Linux
    resource); rewritten like the Windows Lite tests. Checks: clean worktree (`/tmp/s62-wt`)
    `fetch-pdfium.sh linux-x64` + Lite `tauri build --bundles deb` exit 0, no glob error, 24 MiB
    .deb with only the five resources under `/usr/lib/entropia-lite` (libpdfium.so 5,996,800 B),
    guard OK; desktop 3519 / 0 / 7; lint 0 errors. RDD `review-670521d78cb4d478` approved (high
    tier, four lenses: shell in `lite-preview.yml`). Two runs failed on the relay first (header
    timeout, then xAI 403 out of credits); the user switched `review-resilience` to
    `opencode-go/mimo-v2.6-pro` in `~/.pi/gentle-ai/models.json`. Advisories (readability):
    the Pro-payload denylist is duplicated across the two test files; the workflow guard is
    pinned by literal strings; the new Lite test in `runtime-packaging.test.ts` checks several
    things at once; the `(E-01)` tag in a test name is unexplained; some format churn.
  - [x] 6.3 — `build.rs` guard (E-03). Branch `fix/p6-3-build-rs-guard`, commit `c5e89b2a`. The
    decision moved to `build_support/bootstrap_guard.rs` (std-only pure function, `#[path]`-shared
    by `build.rs` and `tests/bootstrap_guard.rs`): skips non-release, Lite (no
    `CARGO_FEATURE_LOCAL_ML`), non-Windows/Linux, unreadable manifest and real release packs;
    otherwise requires all three `ENTROPIA_RUNTIME_BOOTSTRAP_*` and names the missing ones (the old
    guard ignored `_PUBLIC_KEY_ID`). `release.yml` had the vars at workflow level, so Lite jobs saw
    them too: moved to the Pro `build` job; removed from `build-lite` and `lite-preview.yml`.
    README/README.en/AGENTS.md updated. Checks: 9 guard tests; Lite `cargo check --release`
    without vars passes; Pro `--features local-ml` release without vars panics naming all three,
    with dummy vars passes; Lite 2842 / 0 / 37; Pro clippy only the 9 known; desktop
    3520 / 0 / 7; lint 0 errors. RDD `review-f48afa5cf83d94cd` approved (high tier, four lenses).
    Advisory: the cheap early rules (profile/feature/OS) are duplicated between `build.rs` and the
    module.
  - [x] 6.4 — canonical runtime manifest URL (C-02). Branch `fix/p6-4-runtime-manifest-url`,
    commit `3d8d7cd5`. Checked first (2026-10-12): `HumaLab/EntropIA-Pro` is this repository's old
    name (same repo id 1269759651, GitHub redirects); the `runtime-bootstrap` release exists on
    `EntropIA-Pro-Lite` (manifest.json + 4 parts, 2026-06-19) and both URLs return 200 for the
    same asset. `release.yml` now bakes the canonical URL, exactly what
    `publish-runtime-bootstrap.yml` prints for `github.repository` + default tag; the comment
    records that releases up to 1.0.20 keep the old URL compiled in, so no repository may ever be
    created under the old name. Test RED on the old URL, then GREEN. RDD `review-7dea197ed4de09ed`
    approved (high tier; first group run lost `review-risk` to "Stream ended without
    finish_reason", the reoffered group passed). Advisories: the test hardcodes the repository
    name and parses YAML with regexes.
    - Found while checking (not in 6.4 scope, needs a decision): the signed manifest lists only
      `windows-x86_64` releases. The Pro Linux .deb (6.1) has a baked source but no Linux runtime
      to download, so its local-ML runtime cannot bootstrap.
  - [x] 6.4b — fail-closed Pro runtime release gate (follow-up to C-02; user chose "best practice",
    2026-10-12). Branch `fix/p6-4b-runtime-release-gate`, commit `83a3b999`. Verified by code
    reading: the app picks the manifest entry with the exact `(app_version, platform)` of its
    bundled pack (`runtime/bootstrap.rs:97`, `manifest.rs:123`), and the published manifest
    (generated 2026-06-20) only has windows-x86_64 for app 0.1.1-0.1.3 and 1.0.5, so Pro releases
    since 1.0.6 shipped without a downloadable runtime (not runtime-tested). New
    `apps/desktop/scripts/check-runtime-bootstrap.mjs` (exit 0 found / 1 missing with the
    published pairs and the remedy / 2 usage or fetch errors) runs in `release.yml` before the
    Rust/Tauri build for every matrix entry with a `runtime_platform`; the Pro Linux leg is out of
    the matrix until a linux-x86_64 runtime pack exists (Pdfium step and overlay kept). README/
    README.en: Pro is Windows-only for now, Linux users get Lite. Live: 1.0.20 exits 1, 1.0.5
    exits 0. Desktop 3529 / 0 / 7, lint 0 errors. RDD `review-c8d8c5066ab8e3a0` approved (first
    group run lost `review-risk` to a provider 500; the reoffered group passed). Advisories:
    `fetch` has no timeout (WARNING; add `AbortSignal.timeout`), two readability suggestions.
    - Consequence: the next Pro tag stops until "Build Runtime Pack" + "Publish Runtime
      Bootstrap" publish `(version, windows-x86_64)`.
    - Follow-ups: publish a linux-x86_64 runtime pack (new Linux build pipeline: no
      `materialize_linux_*` exists; `build-runtime-pack.yml` is Windows-only) and re-add the Pro
      Linux leg; make the "runtime no disponible" UI distinguish "no entry for this
      version/platform" from "manifest not published" (`DependenciasTab.svelte:638-665`).
  - [x] 6.5 — single version source (C-04). Branch `fix/p6-5-single-version-source`, commits
    `96ececc6` and `ce79a117`. `apps/desktop/package.json` is the single source;
    `apps/desktop/scripts/version.mjs` (`print [--store]`, `check [--tag]`, `bump x.y.z`) verifies or
    rewrites the nine derived declarations (Cargo.toml, the `entropia-pro-desktop` Cargo.lock
    entry only, both tauri confs, both runtime-pack manifests, the ps1 default + help, msix README)
    with anchored token edits, all-or-nothing. `release.yml` runs `check` (with the tag on tag
    pushes) before both builds and derives the Store version, so it holds no version literal; the
    ref reaches bash through env (script injection, RDD advisory on the first commit). Root
    `pnpm version:check` / `pnpm version:bump`; README/README.en/msix README/AGENTS.md updated.
    Checks: 10 version tests + packaging tests (RED first in both commits), desktop 3536 / 0 / 7
    before the hardening, lint 0 errors, `version:check` OK at 1.0.20, `--tag v1.0.19` exits 1.
    RDD `review-efb9235e3b8b7ee5` (first commit) and `review-5169d28b6f77103a` (hardening), both
    approved. Remaining advisories: retrying a half-failed bump reports "already at X" instead of
    finishing the stale files (check still catches them); test/declaration list duplication;
    the attach jobs' `gh release upload "${{ github.ref_name }}"` has the same injection pattern
    (Phase 7).
  - [x] 6.6 — binary provenance (D-07); user asked for "best practice" (2026-10-12). Branch
    `fix/p6-6-binary-provenance`, commits `646b914b` and `67205d1f`.
    `apps/desktop/src-tauri/binary-provenance.json` pins all 13 tracked binaries (path, sha256,
    size, kind, version, provenance; upstream entries add archive URL + archive sha256 + member).
    `apps/desktop/scripts/verify-binaries.mjs`: `check` (offline: schema, size/hash, every tracked
    binary-extension file listed; also run by vitest in CI) and `verify-upstream` (downloads each
    pinned archive, extracts the member, compares bytes); `.github/workflows/verify-binaries.yml`
    weekly, on PRs touching binaries, and on demand. Verified: `pdfium.dll` byte-identical to
    bblanchon/pdfium-binaries chromium/7543 win-x64; both `uv.exe` byte-identical to Astral uv
    0.6.14 (zips match the published .sha256). The README had the wrong pdfium origin (fixed).
    - Not reproducible: the three OCR `.mnn` (source model and conversion not recorded) and the
      hand-captured base MSIX; hashes pinned, origin marked unknown.
    - Deliberately NOT moved to a build-time download: `tauri.windows.conf.json` is merged into
      every Windows build (CI `cargo test` included), so it needs a fetch step in every Windows
      job and in local dev, plus Windows validation; history keeps the blobs anyway.
    - `artifacts/msix/EntropIALite-Store-HLab-1.0.5.0.msix` (9 MB) is an old release output no
      build reads; listed as `release-output`, removal is the user's call.
    - Checks: 17 provenance tests (RED first in both commits), desktop 3550 / 0 / 7 before the
      hardening, lint 0 errors, `binaries:check` 13 files OK, `binaries:verify-upstream` 3/3 OK.
      RDD `review-f6bea9161325f803` (first commit; `review-risk` GLM kept ending streams, the user
      moved it to `opencode-go/mimo-v2.6-pro`) and `review-8f0b62aa7437ca41` (hardening: member
      and archive-name traversal, report-all instead of fail-fast, same-length tamper test,
      binary-extension PR trigger), both approved.
  - [x] 6.7 — minor config (E-04). Branch `fix/p6-7-minor-config`, commit `08742180`.
    - Dev window title: `apply_development_window_title` put the `tauri.dev.conf.json` title
      ("EntropIA Lite") on every debug build, Pro included; now `"{PRODUCT_NAME} (dev)"`, and the
      dev config title is variant-neutral.
    - `crate::USER_AGENT` (`EntropIA-Desktop/<CARGO_PKG_VERSION> (historical-research-app)`)
      for the five clients that sent `0.1`; the sync client keeps `EntropIA-Desktop-Sync/1.0`
      (protocol identifier). `tests/user_agent_ratchet.rs` blocks new literals.
    - `additionalWatchFolders` (a sibling `EntropIA-Agent` checkout that does not exist in a clone)
      removed; inline `--config` recipe in AGENTS.md / README.
    - `rust:quality:report` goes through `apps/desktop/scripts/run-powershell.mjs` (`pwsh`, then
      Windows PowerShell on win32, exit 127 with guidance).
    - Checks: RED first; Lite 2845 / 0 / 37, Lite clippy clean, Pro clippy only the 9 known,
      desktop 3561 / 0 / 7, lint 0 errors. RDD `review-b9452c4932783608` approved (medium tier,
      one lens), no findings.

- [x] Phase 7 — CI supply chain (C-03, D-05, D-06, C-05). PRs #74–#78 (stacked); #75 merges only
  after the user enables "Actions can create PRs" (needs `gh auth refresh -s admin:org`).
  - [x] 7.1 — Dependabot + blocking audit (C-03). Branch `ci/p7-1-dependabot-audit`, commit
    `56bdecf6`. `.github/dependabot.yml`: npm (`/`), cargo (`/apps/desktop/src-tauri`),
    github-actions, weekly, minor+patch grouped. `ci.yml` job `audit`: `pnpm audit --prod
    --audit-level high` (lockfile only) and `cargo audit` from `apps/desktop/src-tauri`
    (cargo-audit 0.22.2 `--locked`), blocking from the start (today: prod npm 0 high, 1 moderate =
    the accepted Tiptap risk; cargo audit exit 0 with its 7 documented warnings). Test
    `ci-supply-chain.test.ts` RED first. RDD `review-7c46bd4ebcd49d84` approved (high tier; one
    CRITICAL resilience finding refuted by the refuter). Advisories: action refs not SHA-pinned
    yet (7.3), the cargo-audit assertion is loose, the dependabot comment anticipates 7.3, and
    `cargo install` on every run is slow.
  - [x] 7.2 — engine pin bump through a PR (D-05). Branch `ci/p7-2-engine-bump-pr`, commits
    `c340ffbd` and `7c82f839`. `engine-pin-bump.yml` no longer pushes to main: after the
    dispatched CI run passes it opens a PR from `bot/engine-pin-bump` (or refreshes title and body
    of the open one). Permissions `contents: write` (bump branch), `pull-requests: write`,
    `actions: write`; the explicit `gh workflow run ci.yml` stays (GITHUB_TOKEN events start no
    workflows). Tests RED first. RDD `review-edd5c8bd5437d18f` (stale body on reuse → fixed in
    `7c82f839`) and `review-6441a57e3ea6749e`, both approved.
    - Repo settings found (2026-10-12, read-only): "Allow GitHub Actions to create and approve
      pull requests" is OFF (`can_approve_pull_request_reviews=false`), so `gh pr create` with
      GITHUB_TOKEN fails until it is enabled. `main` has no classic protection; ruleset "HLab"
      only blocks deletion and non-fast-forward, so it does NOT require PRs. Both changes are the
      user's (admin settings).
    - User decision (2026-10-12, "hagamos todo" + chose "PR obligatorio para todos"): ruleset
      "HLab" (id 24813692) now also has a `pull_request` rule on `main`, 0 required approvals, no
      bypass actors, merge/squash/rebase allowed. Direct pushes to main are rejected for everyone,
      including the nightly bump still on main until #75 merges (its bump stays on its branch).
      Enabling "Actions can create PRs" failed: the HumaLab org disallows it (409), and the gh token
      lacks `admin:org`; the user must run `gh auth refresh -h github.com -s admin:org` so it can
      be enabled at org then repo level.
  - [x] 7.3 — SHA-pinned actions (D-06). Branch `ci/p7-3-pin-actions`, commit `6d7b1475`. Every
    third-party `uses:` is `@<sha> # vX.Y.Z`, newest release of the major already in use (major
    bumps left to Dependabot). upload-artifact unified on v7, download-artifact on v6 (all
    downloads by name). dtolnay/rust-toolchain pinned to a master SHA with `toolchain: 1.90.0`
    everywhere (newly in ci.yml x5 and engine-pin-bump.yml). One mapping script that refuses
    unknown pairs; SHAs resolved via the GitHub API. Tests RED first. RDD
    `review-6ab70ec8acc468a6` approved (two suggestions).
  - [x] 7.4 — pnpm forensics removed (C-05). Branch `ci/p7-4-ci-forensics` (on 7.3).
    - `41663756`: prettier on four test files left unformatted earlier in the stack, which made
      CI's "Format check" fail on every stacked PR.
    - `cb10d8ab` (7.4a): ci.yml loses the 40 forensics/classification/YAML-parse/upload steps (10
      per job in rust-quality-report, lint-typecheck, test, build; -299 lines, nothing added) and
      the three forensics suites from the Pester gate. Kept: `Restore canonical lockfile`,
      `id: pnpm_setup`, Node 22 setup-node, the explicit pnpm bin install. Evidence: the 18
      classification reports of the last 5 CI runs all say `lockfile_anomaly_detected: False`,
      `runtime_inconsistent: False`. Tests RED first (`ci-supply-chain.test.ts`, Pester).
    - `0503bb38` (7.4b): the two scripts and three suites deleted. Local Pester (Linux): 68
      passed, 3 failed (Windows-only MSIX icons; baseline 101/9).
    - RDD `review-f4d6872c9a954435` (format + 7.4a, high tier) approved, one advisory: if the
      lockfile anomaly returns, the diagnostics are gone. Accepted: the restore mitigation stays
      and the scripts are recoverable from git history (`cb10d8ab^`). RDD
      `review-a0e3cd641a90c0e3` (7.4b, medium) approved, no findings.
  - [x] 7.5 — release upload refs through env (follow-up of the 6.5 advisory; not in plan.md).
    Branch `ci/p7-5-release-ref-env`, commit `567e9319`. The two `gh release upload` steps in
    `release.yml` used `"${{ github.ref_name }}"` / `"${{ github.repository }}"` inside `run:` (a
    crafted tag name is legal and would run as code); now `"$GITHUB_REF_NAME"` /
    `"$GITHUB_REPOSITORY"` (ubuntu-latest, bash). Test RED first in `version-consistency.test.ts`:
    no release.yml run script contains `${{ github.`. User decision (2026-10-12): release.yml
    only; the other 7 workflows' run-script interpolations (`inputs.*`, `matrix.*`,
    `runner.temp`, step outputs; none externally controllable) are a Phase 8 follow-up. RDD
    `review-44d52487155f2baa` approved; advisories (test parser is dense, assumes 10-space
    script indentation) accepted: a layout change fails the test loudly, never silently.
- [ ] Phase 8 — quality, hygiene, docs (Q-01..Q-04, R-01..R-05, A-06, S-07, C-06). Started
  2026-10-12 (user: "arrancá y no pares hasta terminar"). One stacked branch/PR per unit on 7.5.
  - User decisions (2026-10-12): `docs/` is NOT public — untrack it (`git rm -r --cached`, local
    copy kept, published history not rewritten); `artifacts/msix/...1.0.5.0.msix` leaves the repo
    (`git rm --cached`, local copy kept) and its `binary-provenance.json` entry goes.
  - [x] 8.1 — Svelte/ESLint warnings (Q-01). Branch `fix/p8-1-warnings`.
    - `cd0e692c`: desktop svelte-check had 1 ERROR (`runtime-packaging.test.ts:325`, from 6.4b
      `83a3b999`), so CI's Typecheck failed from PR #69 on; capture narrowed.
    - `d7d2be1f`: `reducedMotion` in `EntropicConstellation.svelte` is `$state` (real bug: the
      motion class never dropped for reduced-motion users; regression test RED first); 7
      deliberate init-time captures get a justified `svelte-ignore state_referenced_locally`; 6
      ESLint warnings fixed (typed spy, `Slice.empty` handlePaste, dead helper, stale disable).
      `lint` = `eslint src --max-warnings 0` (desktop, ui, store); desktop/ui `typecheck` =
      svelte-check `--fail-on-warnings` (Pro and Lite at 0/0).
    - RDD `review-7ae51e221a1dd525` (medium) approved, no findings.
  - [x] 8.2 — `renderMarkdown` placeholders for inline code and links (Q-02). Branch
    `fix/p8-2-markdown-inline`, commit `53d6f44d`. Code spans then links are parked behind
    private-use placeholders before strong/emphasis, then restored (labels keep emphasis,
    destinations and code stay literal; placeholder chars stripped from input). 6 tests RED
    first; callers' 13 test files 217/217. RDD `review-ce5e44b8de150100` approved, no findings.
  - [x] 8.3 — shared listener registration with partial-failure cleanup (Q-03). Branch
    `fix/p8-3-download-listeners`, commit `bf4a73a1`. New `$lib/register-listeners`
    (`registerListeners(factories, isCancelled)`: sequential; on a rejection releases what landed,
    guarded, then rethrows; cancelled → release all, return []). SettingsView (9 events) and
    DependenciasTab (11) use it; handlers unchanged (`git diff -w`). A registration failure no
    longer escapes onMount: SettingsView logs, DependenciasTab shows its error banner. Helper +
    one regression test per component RED first; Pro 118/5 skipped, Lite 108/15. RDD
    `review-d8f6db4d8b1399f0` approved, no findings.
  - [x] 8.4 — `sanitizeNoteHtml` escapes when there is no DOM (Q-04). Branch
    `fix/p8-4-sanitize-no-dom`, commit `3e4481d4`. Without `document` the input goes through
    `convertLegacyNoteTextToHtml` (escaped, paragraphed) instead of returning raw `<p>`-wrapped
    markup; DOM path unchanged. Tests RED first (`document` stubbed); NoteEditor 40/40. RDD
    `review-46789f4484391e38` approved, no findings.
  - [x] 8.5 — `validate_external_url` via `url::Url` (S-07). Branch `fix/p8-5-external-url`,
    commit `2da12fd9`. After the existing prefix/denylist checks the URL is parsed with
    `url::Url` (http/https, non-empty host, no credentials) and the function returns the
    re-serialized string, which `open_external_url` now opens; exact Store URI unchanged. `url`
    2.5.8 becomes a direct dependency (Cargo.lock gains only the edge). 3 new tests (RED =
    compile, function returned `()`); 8/8; fmt + Lite clippy `-D warnings` clean (Pro clippy
    not run: no cfg-gated code touched). RDD `review-87239090435bf592` approved, no findings.
  - [x] 8.6 — untrack `docs/` and the unused MSIX (R-01). Branch `chore/p8-6-untrack-private`.
    `e8ac9d48`: `git rm -r --cached docs/` (29 files; docs-only, RDD-exempt; local copy kept,
    history not rewritten). `abb024a2`: MSIX 1.0.5.0 untracked (local copy kept), its
    `binary-provenance.json` entry dropped (13 → 12; `binaries:check` OK, test 17/17), and
    `clippy.toml`'s reason no longer cites a docs/ spec. RDD `review-089528703b0dc113` approved.
  - [x] 8.7 — pin MCP versions in `opencode.json`, drop personal paths (R-02). Branch
    `chore/p8-7-opencode-pins`, commit `5a0810a9`. shadcn 4.21.4, figma-developer-mcp 0.13.2,
    tauri-mcp-server 0.13.1, playwright mcp 0.0.83, mcp-sqlite-server 1.4.1 pinned; serena AND
    semgrep (both `C:\Users\agusn\...`) removed with their permission keys. New
    `dev-config.test.ts` guards (exact versions, `--package=`/`cmd /c` aware; no absolute
    executable paths) read the index copy (`git show :opencode.json`): locally the file is
    skip-worktree (the user's personal Linux variant), which this change left untouched
    (committed via `hash-object` + `update-index --cacheinfo`, flag restored). RED on the old
    file, 7/7 on the new. RDD `review-bf447ca54ea1bc64` approved.
  - [ ] 8.8 — move `plan-editor.md`/`plan-lote.md` to `odd/plans/`; `Manual-de-Uso.md` → link (R-03)
  - [ ] 8.9 + 8.12 — align AGENTS.md/README; shallow-clone audit note (R-05, R-04)
  - [ ] 8.10 — split large files only when touched for another reason (A-06): no forced work
  - [ ] 8.11 — code signing (C-06): budget decision, recorded as open; no code

## Notes

- Local Node is v26: its experimental `localStorage` global shadows happy-dom's and fails 18 desktop
  test files. Run desktop tests with `NODE_OPTIONS=--no-experimental-webstorage` (or Node 22). Under
  heavy machine load a few tests (e.g. `route-loader.test.ts`) can time out at 5 s; rerun alone.

- Pro on Linux: `cargo clippy --features local-ml --all-targets -D warnings` fails with 9
  pre-existing errors in `src/deps/uv.rs` (dead Windows-only consts, unneeded `mut`/`return`) and
  `src/llm/download.rs:351` (unreachable statement in a test). No CI job covers Pro on Linux;
  follow-up candidate for Phase 0/7. Local Pro builds need a working CMake: the `~/.local/bin/cmake`
  pip shim is broken; `/tmp/entropia-cmake-venv/bin/cmake` (3.31, via uv) works with
  `CMAKE=... PATH=...`.

- RDD review: both attempts on Phase 0 (lineage `review-0ebf7219c9620f7f`, docs+CI vs `main`, 128 KB
  prompt; lineage `review-3e8616c5d9968c27`, Phase 0 only vs `40f73ea3`, 17 KB prompt) ended in
  `pi-host-relay-timeout` (~15-17 min) with no reviewer output. The user chose to continue this plan
  without RDD review (2026-10-10).

- The hlab task board API answered 401 ("Clave incorrecta o ausente") on 2026-10-09; no cards moved.
