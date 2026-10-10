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
- [ ] Phase 3 — SQL IPC and asset protocol hardening (S-02, S-05, A-04, A-05, S-06)
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
  - [ ] 3.4 (A-05, S-06) — no blocking `ui_conn` in async commands, no keyring I/O under
    `ui_conn`. Branch `fix/a05-s06-settings-keyring`. Heuristic scan: 17 `async` commands lock
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
- [ ] Phase 4 — migrations unified in Rust (A-01, A-03, A-02)
- [ ] Phase 5 — performance and observability (P-01..P-04)
- [ ] Phase 6 — packaging and release (E-02 fix, E-01, E-03, C-02, C-04, D-07, E-04)
- [ ] Phase 7 — CI supply chain (C-03, D-05, D-06, C-05)
- [ ] Phase 8 — quality, hygiene, docs (Q-01..Q-04, R-01..R-05, A-06, S-07, C-06)

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
