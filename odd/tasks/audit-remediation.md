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

- [ ] Phase 2 — vulnerable dependencies (D-01, D-02, D-08, D-03, D-04). Baseline `pnpm audit --prod`:
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
  - [ ] 2.2.2 (D-02) — `markdown-it` >= 14.3.1, `linkify-it` >= 5.0.2 (override).
  - [ ] 2.2.3 (D-02) — `drizzle-orm` 0.40 → >= 0.45.2.
  - [ ] 2.2.4 (D-02) — Tiptap 2 → 3: own PR or documented accepted risk (moderate).
  - [ ] 2.3 (D-08) — Rust advisories (`cargo audit`).
  - [ ] 2.4 (D-03) — unused/duplicated deps.
  - [ ] 2.5 (D-04) — test tooling (vitest, happy-dom, vite).
- [ ] Phase 3 — SQL IPC and asset protocol hardening (S-02, S-05, A-04, A-05, S-06)
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
