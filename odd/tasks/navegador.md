# Navegador (in-app web browser for Lite)

## Objective

A "Navegador" section in EntropIA Lite to browse the web inside the app and
save pages, selections and PDFs as web sources with verifiable provenance
(original/final URL, UTC access time, SHA-256). Web sources belong to no
collection or library; the user may copy (never move) one into a corpus
collection or a Zotero library, never by default. Captures sync across devices.

Approved plan (local-only, `docs/` is gitignored): `docs/navegador/plan.md`;
evidence: `docs/navegador/auditoria-fase-0.md`. Plan approved by the user and a
review panel on 2026-09-29.

## Decisions

- 2026-09-30 (plan §12): captured text stays in the row up to 512 KB, larger
  goes to a file; deleting a web source deletes its local files (copies in
  collections are independent and untouched).

- D1: v1 is manual browsing + capture, no agent.
- D2: own tables `web_sources` / `web_captures`, tied to no collection.
- D3: copy, never move, only on request (collection now, Zotero after the
  Zotero branch merges).
- D4: captures sync from v1 (rows gated by capability `web-capture-v1`, own
  blob path). Sync work waits for `feat/investigations-sync` to land on main.
- New migration must not use numbers 0038–0054 (owned by the Zotero branch).

## Constraints

- Work only in this worktree, branch `feat/navegador`.
- Never commit `apps/desktop/src-tauri/Cargo.lock` with the `entropia-agent`
  git `source` line stripped (local cargo runs strip it; CI rejects it).
- Lite build only for local verification; no Pro builds.
- UI checks need the user (Tauri native window).
- TDD: strict, from the user's global config. Runners: Vitest
  (`pnpm --filter @entropia-pro/desktop test`) and `cargo test` in
  `apps/desktop/src-tauri`.

## Phase 1 — engine viability (engine A: native child webview, no IPC)

### Finding that reorders phase 1 (verified 2026-09-29)

Tauri 2.10.3 only checks the ACL for app commands when the app ships an ACL
manifest (`tauri-2.10.3/src/webview/mod.rs:1801-1805`). EntropIA has none
(`build.rs:22`, plain `tauri_build::build()`), and Tauri injects the IPC
initialization scripts, invoke key included, into every page a webview loads
(`tauri-2.10.3/src/manager/webview.rs:151-218`). A remote page in a child
webview could therefore call any of the 163 app commands. Engine A is only
safe after app commands are gated by an ACL manifest, or with engine B.

### Tasks

- [x] T1 — Decide how to isolate remote content. User decision 2026-09-29:
  upgrade Tauri first, then an app ACL manifest (engine A stays).
- [x] T2a — Upgrade Tauri to 2.11.6 (route: delegated writer). 2.11.1 fixes
  GHSA-7gmj-67g7-phm9 (remote `http://<scheme>.evil.com` treated as local
  on Windows); 2.11.6 also fixes GHSA-w28w-mhc8-qvjv (high, <=2.11.5:
  cross-webview theft of channel responses). Not 2.12.0 (new minor, 3 days
  old). Checks: lean `cargo check`/`cargo test`, lint, typecheck, Vitest.
- [x] T2b — App ACL manifest (route: delegated writer, same unit chain):
  `AppManifest::commands` for every registered command, generated
  permissions granted by webview label (`webviews: ["main"]`, not
  `windows`, because a window match covers its child webviews,
  `ipc/authority.rs:459-460`). The external-page webview gets no app or
  plugin permission. Guard test: registered commands == manifest ==
  granted. Security test: from a remote-origin webview, `db_execute` and
  other sensitive commands are rejected by the ACL.
- [x] T2c — User verifies the main app keeps working (Lite) before any
  browser webview is integrated. Passed 2026-09-30: user ran `tauri dev`
  (Lite) on the worktree, imported two PDFs (5 and 77 pages), viewed and
  edited them; no ACL rejection in the console.
- [x] T3 — (route: delegated writer, with T4) URL policy (pure Rust, TDD): allow https, http only when typed by
  the user; block file:, data:, javascript:, loopback, private ranges, cloud
  metadata; applied to navigation, redirects and new windows. Module
  `src-tauri/src/navegador/url_policy.rs`; evidence below.
- [x] T4 — (route: delegated writer; checks observed, UI not yet seen by the user: that is T6) Prototype child webview (label
  `navegador-web`, incognito, no capability) behind Cargo feature `navegador`
  (enables `tauri/unstable`) and `VITE_NAVEGADOR=1`: open,
  navigate, back/forward/reload, bounds follow the pane, close.
- [x] T5 — (route: delegated writer; automated checks observed, runtime behaviour
  not yet seen by the user: that is T6) Capture without IPC: page text,
  selection with context, HTML snapshot (platform script evaluation with
  result), PDF download via `on_download`. T5a `d63f19fe`, T5b in the commit
  that carries this note.
- [x] T5c — (route: delegated writer; automated checks observed; Windows run pending) Fixes from the user's T5 run: real
  isolated popup windows for sign-in flows (keep window.opener), allow `blob:`
  URLs whose origin passes the policy, dismiss/clear downloads, non-PDF
  downloads to the user's Downloads folder or a chosen folder (fix the
  `.zip.pdf` name), keep the browser alive and hidden across section/tab
  switches (close only when the Navegador tab closes or the app exits).
- [x] T6 — Windows verification matrix with the user (plan §10), then decide
  the engine and update the plan.

## Progress

- 2026-09-29: feature document created. Next: T1 (user decision).
- Engram mirror `odd/navegador/tasks`: PENDING (MCP save refused: several
  active sessions; CLI save failed: database locked). Resync when available.

- [x] T8 — (route: delegated writer) Browser tabs inside the Navegador. User
  decision 2026-09-30: links that ask for a new tab/window (target=_blank,
  Ctrl+click, window.open without size features) open as a new tab inside
  the Navegador, max 4 tabs per browser (independent of the app's own tab
  limit); window.open with size/position features (sign-in flows) keeps
  opening an isolated popup window. Each tab is its own child webview
  (`navegador-web-<n>`, no capability), hidden unless active; capture and
  downloads act on the active tab. Automated checks observed; Windows run
  pending (see T8 evidence at the end).
## Phase 2 — local capture persistence (route: delegated writer)

- [x] P2a — (route: delegated writer; automated checks observed; Windows run
  pending) Migration `0055_web_captures` (never 0038–0054, owned by the
  Zotero branch) with `web_sources` and `web_captures` per plan §5. Shared
  archive: both Lite and Pro get it. Commit `c07c5e4f`. Schema, Drizzle entries
  and tests only; the read repository is left to P2c (see evidence).
- [x] P2b — (route: delegated writer; automated checks observed; Windows run
  pending) Save a capture draft or a verified PDF: files under
  `<data>/web-captures/<source_id>/<capture_id>.<ext>`, file first then the
  DB transaction, text over 512 KB to a file, captures immutable. Commits
  `55fe7157` (Rust, ACL) and `89c9603b` (UI).
- [x] P2c — (route: delegated writer; automated checks observed; Windows run
  pending) Saved sources list inside the Navegador: search by title, URL and
  text; source detail with its captures; open the original URL; delete a
  source and its local files. Commits `423880e5` (Rust), `14805b54` (UI).
- [x] P2d — (route: delegated writer; automated checks observed; Windows run
  pending) Startup sweep of partial/orphan capture files; errors that never
  damage the archive; no empty folder for a source with only selections.
  Commits `2f047dcd` (folder fix), `240ddaab` (sweep).

- [ ] P2e — (route: delegated writer; automated checks observed; Windows run
  pending; commits `c5884034` Rust, `73a7263e` UI) PDF sources, from the user's P2c run:
  a saved PDF's source URL must be the page it was downloaded from (the
  download's page snapshot), not the file link; the file link stays on the
  capture. Source detail for PDFs gets two actions (user decision
  2026-10-01): "Abrir página de origen" (loads the origin page in the active
  tab) and "Ver PDF guardado" (opens the local copy in the app's own PDF
  viewer, never re-downloading). A download whose sha256 is already saved is
  flagged "ya está en tus fuentes" instead of offering Guardar again.

- [ ] T7 — Repeat the §10 matrix on macOS (WKWebView) and Linux (WebKitGTK).
  Known gaps there: sign-in popups do not close on `window.close()` (wry does
  not wire `webViewDidClose:` / GTK `close`), and macOS reports no download
  path. Engine A is approved for Windows only until then.

## Open items to decide before merging with the Zotero branch

- [ ] Schema-tag ordering (found in P2a). The client's `X-Schema-Tag` is the
  LAST APPLIED migration, not the highest name: `read_schema_tag`
  (`apps/desktop/src-tauri/src/sync/engine.rs:793-800`, `SELECT name FROM
  _migrations ORDER BY id DESC LIMIT 1`), read fresh each cycle
  (`engine.rs:630`; the writing sync reads it the same way:
  `writing_cycle.rs:300`, `writing_push.rs:235`, `writing_pull.rs:158`) and sent
  as the `X-Schema-Tag` header (`sync/http.rs:37`). The runner applies pending
  names sorted and records them in that order (`packages/store/src/runner.ts:
  1368-1370`). The server keeps `max(stored, X-Schema-Tag)` lexicographically
  on push (`EntropIA-Cloud/src/handlers/sync.rs:179`, `UPDATE accounts SET
  schema_tag = ?1 ... AND ?1 > schema_tag`) and answers 426 to any push or pull
  whose tag is lower (`sync.rs:68-86`, `if client_tag < stored`). Consequence: a
  device that applied `0055_web_captures` and later applies the Zotero
  `0038`..`0054` ends with head `0054_...`, lower than the account's
  `0055_web_captures`, and is locked out with 426 until its head sorts above
  0055. Decide at merge time: renumber `0055` above the Zotero range, or make
  the client send the highest applied name. NOT changed.

## Follow-ups (outside this feature)

- Pre-existing bug on main, not caused by the ACL change: `readAssetSize`
  (`apps/desktop/src/lib/collection-import.ts:88`) calls fs `stat()` on the
  relative storage key that `split_pdf_pages` returns
  (`store_asset_path_at_boundary`, `src-tauri/src/ocr/commands.rs:~747`), so
  the fs plugin rejects it ("forbidden path: assets/...") and every split PDF
  page asset is stored with `size = NULL`. Fix: resolve against the data dir
  before `stat`, or have the command return the size.

## Verification evidence

- 2026-10-01, user's Windows run of P2c/P2d (dev profile): selection-only
  save creates no folder PASS; saved-sources drawer beside the page PASS;
  search by title/URL/text PASS; detail (local + UTC, hash, size) PASS;
  copy URL PASS; delete with confirmation removes row and folder PASS;
  startup sweep log line PASS. Finding: "Abrir en el navegador" on a PDF
  source loads the stored download link (`.../articulos/227/descargar`), so it
  downloads again; the same PDF downloaded four times (sha e33acec22ed8) is
  listed four times with Guardar -> P2e. P2c, P2d checked.


- 2026-10-01, user's Windows run of P2a/P2b on the isolated dev profile
  (`profile=dev:navegador ... sync=disabled` confirmed in the startup log):
  save page, selection and verified PDF PASS; second capture of the same URL
  reuses the source PASS; files under `dev-profiles\navegador\web-captures\`
  PASS; rows visible in the DB browser PASS; survive a restart PASS. Finding:
  a source with only selections gets an empty folder (selections store text
  in the row, no file) -> fix in P2d. P2a, P2b checked.


- 2026-10-01, incident remediation, server step: CHECKED read-only on the
  sync server (container `idhit0wuzbld1u4ee83akvr6-…`, Traefik host
  `entropia-cloud.app.hlab.com.ar`, DB `/data/sync.sqlite`): the account's
  `schema_tag` is `0037_fts_vocab` and no account holds `0055_web_captures`
  (tags present: '', `0023_sync_ids`, `0037_fts_vocab`). The tag was never
  raised, so no UPDATE was run. Incident closed: local archive back on 0037,
  server on 0037.


- 2026-10-01, incident remediation, local step DONE: with every EntropIA app
  closed, the real archive was backed up (sqlite backup API + raw files, in
  `%APPDATA%\com.entropia.shared\backups-0055-rollback-20261001-011914\`,
  integrity ok, 2205 items) and, in one transaction, the two empty tables
  `web_captures`/`web_sources` were dropped and the `_migrations` row
  `0055_web_captures` deleted. Verified after: last migration
  `0037_fts_vocab`, no `web_*` objects, integrity ok, 2205 items. Cause: the
  user's `tauri dev` from this worktree stayed open and Vite reloaded the
  renderer with the new runner, applying 0055 with no explicit run.
  PENDING: reset the account's server `schema_tag` from `0055_web_captures`
  to `0037_fts_vocab` (`UPDATE accounts SET schema_tag='0037_fts_vocab'
  WHERE email=? AND schema_tag='0055_web_captures'`). Until then this machine
  also gets 426.


- 2026-09-30, user's Windows rerun of T8 fixes: single Navegador workspace
  tab (top bar and split view focus the existing one) PASS; download labels
  keep the originating page title PASS; download started in a background tab
  listed with the right origin PASS; one line per download (`c1909790`) PASS.
  T8 checked.


- 2026-09-30, user's Windows run of T8 (tabs, commits `aaf0f868`, `1fcb1c6b`):
  PASS: target=_blank and Ctrl+click open a tab; `window.open(url)` opens a tab;
  `window.open` with a size opens a popup; Google Sign-In still works and closes
  itself; 4-tab cap, the 5th rejected; switching tabs keeps each page; closing
  tabs, closing the last tab; leaving the section and returning; closing the
  Navegador workspace tab; capture acts on the active tab (two tabs, different
  pages, selection captured in the second: quote and URL from the second). Not
  yet exercised: a download started in a background tab. Two findings, fixed in
  the commit that carries them (`b7837ffc` and `c4a0da3e`), rerun pending: (1) a second
  app tab could also choose the Navegador and both showed the one browser; now
  the Navegador lives in one workspace tab (`WorkspaceStore.navegadorOwnerId`,
  same owner rules as Writing: `navigateActive`, `openTab` and the split view
  focus the owner; a tab that reaches it through its own history shows a notice
  in `WorkPane` instead of a second view). (2) A download line showed the tab's
  CURRENT title, so three PDFs from one tab all read like the last page; the
  backend now snapshots the page url and title at `DownloadEvent::Requested`
  (`Registry::set_page`, title cleaned and bounded) into the draft
  (`pageUrl`, `pageTitle`) and the line reads "Desde <host> · <title>" from that.
  RED: workspace 9/9 new tests failed, download 5 new Rust tests failed to
  compile (`set_page`, `PAGE_TITLE_MAX`); the capture and view tests were
  rewritten with the implementation. The WorkPane notice tests passed on first
  run (written after the implementation).

- 2026-09-30, user's Windows rerun after `b538e0db`: Google Sign-In popup
  closes itself after login PASS; a popup closed by hand frees its slot, three
  popups open, the fourth is rejected PASS. T5, T5c checked.
- Phase 1 decision (T6), 2026-09-30: engine A (native child webview, no
  capability, incognito) APPROVED on Windows: isolation, URL policy, viewer,
  capture without IPC (`eval_with_callback`), PDF quarantine, downloads,
  sign-in popups all passed. macOS/Linux pending (T7). Open UX question: links
  with target=_blank now open a separate window; ask the user whether plain
  links should stay in the browser and only script popups get a window.


- 2026-09-30, user's Windows run of T5c: Google Sign-In popup on x.com logs in (PASS) but stays open after login (window.close() not honoured) -> fixed in the commit that carries this note, rerun pending; GitHub blob: PDF download 'PDF verificado' 4.2 MB PASS; dismiss/clear PASS; .ipynb/.zip/.jpg saved to C:\Users\agusn\Downloads and, after 'Cambiar', to S:\Descargas, no .pdf suffix PASS; persistence across section/tab switches PASS; closing the Navegador tab and closing the app with a popup open PASS; Mark-of-the-Web kept on a rerouted download (Zone.Identifier: ZoneId=3, HostUrl=about:internet) PASS.

- 2026-09-30, user's Windows run of T5: page capture (lanacion.com.ar: title,
  final URL, UTC, sha256, 9171 chars, 468.9 KB HTML) PASS; selection capture
  (lanacion, x.com) exact quote PASS; PDF download from a link "PDF verificado"
  PASS; zip rejected "No es un PDF" PASS. Found: (1) Google Sign-In popup
  (accounts.google.com/gsi/select, ux_mode=popup) goes blank because popups
  load in the same webview and lose window.opener; (2) GitHub PDF download via
  a `blob:` URL rejected by the policy; (3) download list cannot be dismissed;
  (4) non-PDF downloads should go to the user's Downloads (or a chosen
  folder), and the rejected zip was shown as `.zip.pdf`; (5) leaving the
  Navegador section/tab and coming back resets the page, history and drafts.


- 2026-09-30, user's Windows run of the prototype (`tauri dev`, Lite,
  `--features navegador`, `VITE_NAVEGADOR=1`), plan §10:
  - Isolation: PASS. From the DevTools console of the child page
    (`location.href` = `https://example.com/`), `invoke('db_execute', ...)`
    and `invoke('navegador_close')` were rejected: "not allowed on window
    \"main\", webview \"navegador-web\", URL: https://example.com/ ...
    allowed on: [webviews: \"main\", URL: local]".
  - URL policy: PASS. `file:`, `localhost`, `127.0.0.1`, `2130706433`
    (decimal 127.0.0.1) and `169.254.169.254` blocked with a message; typed
    `http://example.com` loads.
  - Navigation, resize/zoom/drawer, new-window links, window buttons with the
    browser open, section switch and app close: PASS per the user.
  - Known limitations confirmed: main-UI popovers render under the native
    webview; switching tab closes the page.


- T2a (Tauri 2.11.6, commit d9984c54): Cargo resolved with
  `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback` because the newest
  tauri-build/codegen/macros/runtime/utils need Rust 1.90 and the repo pins
  1.88. Moved: tauri 2.10.3 -> 2.11.6, tauri-build 2.5.6 -> 2.6.3,
  tauri-codegen/macros 2.5.5 -> 2.6.3, tauri-runtime 2.10.1 -> 2.11.3,
  tauri-runtime-wry 2.10.1 -> 2.11.4, tauri-utils 2.8.3 -> 2.9.3, wry 0.54.4 ->
  0.55.1, tao 0.34.8 -> 0.35.3, plus transitive crates (muda, tray-icon, ctor...).
  JS: @tauri-apps/api 2.10.1 -> 2.11.1, @tauri-apps/cli 2.10.1 -> 2.11.5. The
  Rust plugins dialog 2.7.0 / fs 2.5.0 were left as is. entropia-agent pin kept.
  Observed: `cargo check --all-targets` ok; `cargo test --no-fail-fast` 1307
  passed, 1 failed (`db::open::tests::no_other_module_opens_the_archive_by_hand`,
  flags three sync `*_tests.rs` files committed in 56cca4df, untouched by this
  change: pre-existing); `pnpm typecheck` ok; `VITE_LOCAL_ML=0` desktop typecheck
  ok (0 errors); `pnpm test` 2587 passed / 7 skipped; `pnpm lint` fails on
  `WritingView.svelte:1403 svelte/require-each-key` and `pnpm format:check` fails
  on app-close.test.ts, writing-sync-notices.test.ts, writing.ts (all untouched:
  pre-existing).
- T2b (app ACL manifest, the commit that carries this note): build.rs passes
  `AppManifest::commands(APP_COMMANDS)` (163 commands) to `tauri_build::try_build`;
  permissions are named `allow-<command>` with `_` as `-` (e.g. `allow-db-execute`),
  generated to `src-tauri/permissions/autogenerated/*.toml` (gitignored, rebuilt
  from a clean tree) and `gen/schemas` (already gitignored). `default.json` now
  selects `webviews: ["main"]` and grants all 163 permissions; splash.html has no
  script, so the splash window gets nothing. Finding: Tauri 2.11.6 already
  ACL-checks any non-local origin even without a manifest (`webview/mod.rs`
  `|| !is_local`), so the upgrade alone closes the remote-page hole; the manifest
  adds label scoping (a non-main webview on an app URL, and app commands for any
  webview). Guard: `tests/acl_manifest_guard.rs` (4 tests). Security:
  `tests/app_acl.rs` (5 tests, real `generate_context!`, stub command bodies).
  RED observed: guard 3/4 failed before build.rs/capability changed; app_acl
  `the_external_webview_gets_nothing_even_when_its_page_looks_local` failed
  (`db_execute` ran from `navegador-web`), the four other tests already passed
  on 2.11.6. GREEN: 4/4 and 5/5. Test executables on Windows needed a Common
  Controls v6 manifest (STATUS_ENTRYPOINT_NOT_FOUND), added in build.rs for
  test targets only. `cargo fmt --check` ok; `cargo test --no-fail-fast` 1307 +
  4 + 5 + others passed, same single pre-existing lib failure as T2a.
- Rust 1.88 durability (follow-up, `build(rust)` commit): `[package]` now has
  `rust-version = "1.88"` and `resolver = "3"` (accepted for this single-package
  edition-2021 manifest; `cargo tree -e features` output is byte-identical to
  resolver 2). Without the env var, `cargo update --dry-run` no longer proposes
  the 1.90-only tauri-build/codegen/macros/runtime/utils (it picks 1.88-compatible
  tauri-plugin*, aes, thiserror patches instead), and `cargo update -p
  entropia-agent --dry-run` (what engine-pin-bump.yml runs) resolves with "0
  packages to latest Rust 1.88 compatible versions". `cargo check --all-targets`,
  app_acl (5) and acl_manifest_guard (4) pass; Cargo.lock unchanged, pin intact.
- app_acl tightening (follow-up, `test(security)` commit): rejections are now
  matched on the ACL's own wording (`not allowed on window "`, `not allowed on
  origin [`, release `not allowed by ACL`; strings from tauri-2.11.6
  `ipc/authority.rs` and `webview/mod.rs`), after unwrapping the JSON string
  error, so a plugin scope error no longer counts. New test
  `the_main_webview_keeps_the_core_permissions_the_ui_relies_on` (event listen,
  window minimize/start_dragging, set_webview_zoom, path resolve_directory not
  ACL-rejected from main; event listen and window minimize rejected from a
  remote page). RED: with `core:event:default`, `core:path:default` and
  `core:window:allow-minimize` removed from the capability that test failed
  (other 5 passed); capability restored, 6/6 and guard 4/4 pass, fmt ok.
- T3 (url policy): `navegador::url_policy::{check, check_url, NavigationKind,
  Blocked}`, pure, no new dependency (`tauri::Url`; the host string is parsed
  back because the `url` crate already normalises every IPv4 spelling). RED:
  stubbed `unimplemented!()`, 18 of 19 tests failed. GREEN: 19/19 (`cargo test
  --lib navegador`). Covers https/http-only-when-typed, bare host -> https,
  blocked schemes, localhost and `*.localhost` (with trailing dots),
  `metadata.google.internal`, IPv4 private/loopback/link-local/CGNAT/
  multicast/reserved and their public neighbours, decimal/octal/hex/short IPv4
  spellings, IPv6 (loopback, ULA, link-local, multicast, IPv4-mapped, NAT64,
  6to4), userinfo, ports, IDN. Limitations: (1) DNS rebinding (a public name
  resolving to a private IP) cannot be blocked from `on_navigation` without
  resolving DNS: decision for later (options: resolve and pin via a proxy, or
  accept). (2) `http` for non-typed navigation is blocked, so the webview layer
  must let through the single typed http URL it remembers.
- T4a (child webview, Rust side; T4 stays open until the view lands).
  `src-tauri/src/navegador/{bounds,commands,viewer,viewer_unavailable}.rs`,
  Cargo feature `navegador = ["tauri/unstable"]` (`Window::add_child` is
  `cfg(any(test, all(desktop, feature = "unstable")))`, tauri-2.11.6
  `window/mod.rs`). Nine commands (`navegador_open|navigate|back|forward|reload|
  set_bounds|set_visible|close|state`) always registered, in `APP_COMMANDS` and
  `default.json`; without the feature they answer "not available in this
  build". They are `async` because `add_child` blocks on the main thread and a
  sync command would deadlock on Windows. The child (label `navegador-web`,
  `incognito(true)`, no capability) reports through `emit_to("main", ..)` only:
  event `navegador://state` `{url, title, blocked}`. Popups never open a window:
  they load in the same webview if the policy allows. Downloads are refused
  (T5). RED: 3 new `app_acl` tests failed with "navegador_open not allowed.
  Command not found"; bounds sanitizer 5/5 failed on `unimplemented!()`. GREEN:
  app_acl 9/9 (also with `--features navegador`), guard 4/4, `--lib navegador`
  26/26, `cargo check --features navegador` ok, no new clippy warnings in
  either build; full `cargo test`: 1333 passed, 1 failed (the known
  `no_other_module_opens_the_archive_by_hand`). The guard test was not observed
  red in isolation: handler, build.rs and capability were changed together.
  Findings: (1) once a child webview exists, Tauri stops treating `main` as a
  webview window (`Window::is_webview_window` is "every webview shares the
  window label"), so `get_webview_window("main")` returns `None` and commands
  taking a `WebviewWindow` argument fail. The one call that mattered is the
  close path (`lib.rs`, destroy after the flush), which now calls
  `navegador::shutdown` first; `store_updates` (Windows, hwnd of `main`) and
  any future `WebviewWindow` argument degrade while the child is open. (2)
  wry 0.55 on Windows builds one WebView2 environment per webview and applies
  `incognito` per controller (`SetIsInPrivateModeEnabled`), so an incognito
  child in a window whose main webview is persistent is fine; both share the
  default user-data folder, so do not pass `additional_browser_args` or a
  different `data_directory` for one of them. On WebView2 < 101.0.1210.39
  `incognito` silently does nothing. (3) `emit_to("main")` only reaches JS
  listeners registered by a webview with that label; the child could not
  register one anyway (ACL).
- T4b (view, frontend). `VITE_NAVEGADOR=1` (`$lib/capabilities` `NAVEGADOR`,
  defined in vite/vitest configs, default 0) shows a TopBar entry, `View`
  `{name:'navegador'}`, lazy `views/NavegadorView.svelte`. Pure logic in
  `lib/navegador.ts`: `computeBounds(rect, zoom)` (CSS px times the webview zoom
  = logical px, whole pixels, null for an empty rect) and `createViewerSession`
  (one native webview, the view shown last owns it, calls serialised, closes
  when the last view unmounts, hides while another view still holds it).
  Bounds are re-sent on mount, `ResizeObserver`, window resize, capture-phase
  scroll, zoom change and a 400 ms re-measure (drawer moves that no resize
  reveals). Overlay handling: the page is hidden while `[data-overlay-root]`
  has children (dialogs). New icon name `browser` in `ACTION_ICON_NAMES`.
  RED: `navegador.test.ts` 14/14 failed on a throwing stub, tab-meta and
  navigation navegador tests failed; the TopBar "hidden by default" test is a
  guard that passed from the start. GREEN: `navegador.test.ts` 15/15; `pnpm
  test` 299 store + 800 ui + 2605 desktop passed (7 skipped);
  `VITE_LOCAL_ML=0` desktop 2584 passed; `pnpm typecheck` 0 errors (8 old
  warnings), Lite typecheck 0 errors; `pnpm lint` only the known
  `WritingView.svelte:1403`; prettier clean on every touched file;
  `VITE_NAVEGADOR=1 vite build` emits the NavegadorView chunk.
  Known limitations of the prototype: (a) the native webview paints above all
  HTML, so `ToolbarMenu` popovers (portalled to `<body>`) and tooltips over the
  page area are hidden behind it; only dialogs in the overlay root are
  detected. (b) Switching tab closes the browser (AppShell remounts the pane;
  the last unmount closes it), so the page and its history do not survive a tab
  switch; keeping it alive is a phase-2 decision. (c) Back/forward buttons are
  never greyed (Tauri has no `can_go_back`). (d) `incognito` is silently
  ignored by WebView2 older than 101.0.1210.39. (e) DNS rebinding (see T3).
  (f) While the child exists `get_webview_window("main")` is `None` (see T4a).
  (g) Downloads are refused and there is no capture yet (T5).
- T5a (capture page and selection as drafts; T5 stays open until T5b). Route:
  delegated writer (T5 mapping trigger: 4+ files across Rust, JS and Svelte).
  `navegador_capture_page` / `navegador_capture_selection` (async, in
  `APP_COMMANDS`, `default.json`, `generate_handler!`; "not available" without
  the feature). They run `src-tauri/src/navegador/capture.js` (`include_str!`,
  the kind substituted for the `'__KIND__'` placeholder) in the child webview
  with `Webview::eval_with_callback`, bridged to a `tokio` oneshot with a 10 s
  timeout, and `capture::parse_capture` turns the JSON into a typed
  `CaptureDraft`: object only, every field typed, text <= 2 MiB, html <= 10 MiB,
  whole message <= 64 MiB (byte limits, not characters), title/site/lang
  cleaned (control and bidi characters gone, bounded), canonical URL kept only
  if absolute http(s), context around a selection cut to 400 chars on each side,
  `final_url` re-checked with the T3 policy (`kind_for`: a typed `http` page can
  still be captured, a link-followed one cannot), `accessed_at` from the Rust
  clock (own RFC 3339 formatter, no new crate), `sha256` over the html bytes for
  a page and over the quote bytes for a selection (`hashOf` says which). The
  HTML never crosses IPC (`#[serde(skip)]`), only its size. Nothing is written
  to disk or SQLite. UI: two toolbar buttons (`file-text`, `text-quote`) and a
  panel BELOW the placeholder (title, kind, final URL, accessed_at UTC, short
  hash, text length, HTML size, truncated flag, 500-char preview, error); the
  placeholder shrinks and the existing ResizeObserver moves the native webview.
  Pure helpers in `lib/navegador-capture.ts`.
  RED/GREEN: `capture.rs` 25 of 26 failed on `unimplemented!()` then 26/26
  (27/27 with `script_for`); `navegador-capture-script.test.ts` (happy-dom, the
  real `.js` via `?raw`) 23 of 25 failed on the stub then 25/25; it also
  compares the byte/char limits of the script with the Rust constants;
  `navegador-capture.test.ts` failed to import (module missing) then 30/30;
  `app_acl` 3 new-command failures ("Command not found") then 9/9 also with
  `--features navegador`; `acl_manifest_guard` failed on the two new commands
  (handler changed first) then 4/4.
  Security notes: the script runs in the page's main world, so a hostile page can
  alter what it returns (it controls its own content anyway); the draft is data,
  never instructions; the snapshot is the raw `outerHTML` (scripts included), and
  stripping active content stays a display-time job.
  `eval_with_callback` (read in tauri 2.11.6 and wry 0.55.1, `webview2/mod.rs`
  `execute_script`; NOT observed at runtime, I cannot see the app): the callback
  gets the script's return value as a JSON string (`ExecuteScript`), so an
  object arrives as `{"ok":true,...}`; an exception in the page arrives as
  `"null"` on Windows, which is why the script catches everything and returns
  `{ok:false,error}` (a `null` is rejected as `invalid_result`). It goes through
  WebView2 directly, not the page's IPC or CSP, and needs no capability. If the
  page never answers (hung script, document being replaced) the 10 s timeout
  returns `timeout`. The built-in PDF viewer: the script answers `pdf_document`
  when `document.contentType` is `application/pdf` (unverified in WebView2; if
  the viewer is an embedded plugin document the script would otherwise return
  empty text). Unsettled: whether WebView2 accepts a 10+ MB `ExecuteScript`
  result: the script caps at 12 MiB of content, so a page near the cap is the
  case to try.
  Limitations: only the top frame is read (selections and text inside iframes
  are not captured); selection context is whitespace-collapsed `textContent`, so
  anchoring in phase 2 must normalise whitespace the same way; text is
  `innerText` (rendered, so hidden content is excluded), while the HTML snapshot
  is the live DOM, not the bytes the server sent; lone surrogates are replaced
  with U+FFFD; a page can spoof what it returns.
- T5b (PDF download into quarantine). `navegador/download.rs` (pure, tested) plus
  the handler in `viewer.rs`. `DownloadEvent::Requested { url, destination }`
  (tauri 2.11.6; wry 0.55.1 `webview2/mod.rs` `DownloadStarting`): the URL runs
  through the T3 policy (`kind_for`, so a typed `http` file works), a `Registry`
  allows at most 4 at once, and `destination` is rewritten to
  `<cache>/navegador/downloads/<uuid>.part` (`path_utils::cache_dir`, never the
  Downloads folder); a denied download makes wry call `SetCancel(true)`. The
  name WebView2 suggests (`destination`'s file name: Content-Disposition or the
  URL) is only kept for display, after `sanitize_file_name` (last component
  whatever the separator, `<>:"|?*` and control/bidi characters replaced or
  dropped, reserved device names `CON`/`NUL`/`COM1`-9/`LPT1`-9 prefixed with `_`,
  edge dots and spaces trimmed, 120 chars, always `.pdf`). `Finished { url, path,
  success }`: the code looks only at the file name of the path it chose (the OS
  directory is ignored, ids are validated as `[A-Za-z0-9-]`), so nothing outside
  quarantine can be read, renamed or deleted. On success, a thread checks size
  (0 < n <= 100 MiB, cap also enforced while hashing), the `%PDF-` magic at
  offset 0, hashes in 64 KiB blocks and renames to `<uuid>.pdf`; a failed check
  deletes the file and the draft is `rejected` (`not_pdf`, `too_large`, `empty`)
  or `failed` (`io_error`, `interrupted`). Drafts `{ id, url, fileName, size,
  sha256, accessedAt, status, reason }` (no path; the file is `<id>.pdf`) go out
  as `navegador://download` with `emit_to("main", ..)` at start and at the end.
  `sweep_quarantine(&cache_dir)` runs from `setup` on a thread and deletes files
  older than 24 h (also verified PDFs: until phase 2 decides where they live,
  quarantine is their only home). UI: downloads list in the same panel, newest
  first, max 5. No new command, so no new ACL surface; the child webview still
  has no capability.
  RED/GREEN: `download.rs` 33 of 34 failed on `unimplemented!()` then 34/34;
  draft constructors 4 failed then 39/39; `navegador-messages.test.ts` (reads
  the Rust `code::`/`reason::` constants and both language tables) 8 of 10 failed
  then 10/10; `NavegadorView.test.ts` (10 tests: buttons disabled, draft panel
  outside the placeholder, failure messages es/en, dismiss, hostile markup as
  text, download list and rejection) written after the view: 9/10 first run, the
  failure was a test timing issue (label after a locale change), fixed.
  Verification: `cargo test --lib navegador` 92/92 with and without `--features
  navegador`; `--test app_acl` 9/9 and `--test acl_manifest_guard` 4/4 with the
  feature (in a separate target dir: the user's `tauri dev` had the exe locked);
  full `cargo test --no-fail-fast` 1399 lib passed, 1 failed (the known `no_other_module_opens_the_archive_by_hand`) plus every integration test ok; `cargo check`, `cargo clippy --all-targets` with and without the feature: no
  warnings in `src/navegador`; `cargo fmt --check` ok; `pnpm typecheck` 0 errors;
  `VITE_LOCAL_ML=0` desktop typecheck 0 errors; `pnpm test` 299 store + 800 ui +
  2680 desktop passed (7 skipped); `pnpm lint` only the known
  `WritingView.svelte:1403`; prettier clean on every touched file;
  `VITE_NAVEGADOR=1` vite build emits the NavegadorView chunk.
  Not observed (needs the user, T6): WebView2's real behaviour for
  cancellations (source says a cancelled or failed download reports `success:
  false` and `path: None`, matched by URL), whether the built-in PDF viewer
  shows instead of downloading when a link opens a PDF (WebView2 shows its
  viewer for `application/pdf` without `Content-Disposition: attachment`; then
  `on_download` never fires and the capture script answers `pdf_document`), and
  whether the viewer's own save button reaches `on_download` (unknown).
  Limitations: the size cap cannot stop a download in progress (no progress
  event), only refuse it when it ends; `DownloadEvent::Finished` on macOS has no
  path (matched by URL, the file is still ours); Content-Disposition is not
  exposed beyond the suggested name; an inline PDF cannot be captured yet: phase 2
  can add "Guardar PDF", which downloads the current URL through this same
  quarantine path (not built here); concurrent downloads of the same URL are
  matched to failures oldest first.
- T5c-3 (`blob:` in the URL policy; T5c stays open until the user's Windows run).
  `url_policy::check_url` now sends `blob:` to `check_blob`: the inner address
  (`blob:<origin>/<id>`, parsed from the URL path) must be `http` or `https` and
  then passes the same rules as any address for the same kind (typed `http`
  only when typed, blocked hosts blocked as that host, userinfo cannot hide a
  host). Opaque (`blob:null/..`), non-http(s) (`blob:about:blank`, `blob:file:`,
  `blob:data:`, a blob of a blob) and unparsable blobs are refused as the `blob`
  scheme. The old table row that expected every `blob:` to be blocked moved to
  the new tests. RED: 5 of the 6 new tests failed (every `blob:` was
  `Scheme("blob")`); GREEN 26/26 (`cargo test --lib navegador::url_policy`), one
  test adjusted after the first GREEN run because a typed `blob:1234` is read
  as host `blob` port 1234 (existing `host:port` rule), so the bare-blob case
  is asserted for non-typed kinds only. `cargo fmt --check` ok, `cargo test
  --lib navegador` all pass. Limitation: the policy sees the address, not the
  bytes, so a page can still hand the browser a blob of its own making (that is
  what a blob is); downloads still go through quarantine/verification.
- T5c-1 (keep the browser alive across section and tab switches). Route:
  delegated writer, TS only. The Navegador view unmounts on every section or tab
  switch; before, the last unmount closed the native webview (page, history,
  drafts and downloads lost). Now `createViewerSession.detach` only HIDES the
  browser (and only when that view was driving it); the next view to `show`
  puts the same webview back at its own rect (adopting `navegador_state`, which
  the view already did). New `session.close()` closes for good. The `attach`
  bookkeeping is gone. Drafts and downloads moved to a module-level store
  (`lib/navegador-store.ts`: `navegadorStore`, capture draft, capture error,
  downloads), which keeps listening to `navegador://download` for the rest of
  the session, so a download that ends while the person is in another section
  is listed on return. Closing: `WorkspaceStore.onTabClosed` (new, generic,
  listener errors isolated) feeds `lib/navegador-lifecycle.ts`; the shell
  (`AppShell`, behind `NAVEGADOR`) installs it: closing the last tab whose
  current view is the Navegador closes the browser (`navegador_close`) and clears
  the store; the app exit path is the existing `navegador::shutdown`.
  Decision, two Navegador tabs: ONE browser instance shared by every Navegador
  tab (the pane shown last drives it; a second tab adopts the same page).
  Closing one of two Navegador tabs keeps the browser; closing the last one
  closes it. A tab that navigated away from the Navegador before being closed
  does not close it (it stays hidden until a Navegador tab closes or the app
  exits): a deliberate limit. Hiding on unmount is `set_visible(false)`, which the
  backend already implemented; no Rust change in this unit.
  RED: `navegador.test.ts` 6 failed (`session.close is not a function`, detach
  closed the browser), `navegador-store.test.ts` and `navegador-lifecycle.test.ts`
  failed to import; `NavegadorView.test.ts` against the old view: every test failed
  once the session lost `attach`. GREEN: `navegador.test.ts` 17/17,
  `navegador-store.test.ts` 9/9, `navegador-lifecycle.test.ts` 10/10,
  `workspace.test.ts` 48/48 (the four `onTabClosed` tests were written after the
  implementation; the lifecycle tests exercise the same path from the failing
  side), `NavegadorView.test.ts` 14/14 (new: hides but never closes on unmount,
  shows the same browser again at the new rect without `navegador_open`, keeps
  draft and downloads across a remount, lists a download that finished while no
  view was mounted), `AppShell.test.ts` 50/50. `pnpm --filter
  @entropia-pro/desktop typecheck` 0 errors. Not observed (needs the user):
  the real hide/show on WebView2 across tab switches. The AppShell wiring has
  no test of its own (`NAVEGADOR` is a build-time flag, 0 under Vitest); the
  helper it calls is tested.
- T5c-4 (dismiss and clear the download list). `navegadorStore.dismissDownload(id)`
  and `clearDownloads()` (TS only). The panel shows a "Limpiar"/"Clear" button in
  the downloads header and a per-item close button (`Quitar <name> de la lista`);
  the capture draft keeps its own "Descartar" close button and the two coexist in
  the same panel, each dismissible without touching the other. Dismissing
  removes the LIST ENTRY only: a PDF that already reached quarantine stays there
  until the 24 h sweep (`sweep_quarantine`) or, in phase 2, until it is saved; a
  download still running keeps running, and its later updates are ignored
  (the store remembers up to 100 dismissed ids so a late `ready` does not bring
  the entry back; `clearAll`/`reset` forget them). RED: 5 new store tests and 5
  new `NavegadorView` tests failed (missing methods, no buttons). GREEN:
  `navegador-store.test.ts` 15/15, `NavegadorView.test.ts` 19/19 (new: one item,
  clear all and the button goes away, a removed download does not come back,
  capture draft and downloads together with independent dismissals, English
  labels), `navegador-messages.test.ts` 10/10. Prettier clean on the touched
  files.
- T5c-2 (real popup windows for sign-in flows). Route: delegated writer.
  Problem: `on_new_window` returned `Deny` and loaded the address in the same
  webview, so Google Sign-In's `ux_mode=popup` (accounts.google.com opened from
  x.com) lost `window.opener` and went blank. What tauri 2.11.6 / wry 0.55.1 expose
  (read in the source, NOT observed running): `WebviewBuilder`/`WebviewWindowBuilder`
  `on_new_window(Fn(Url, NewWindowFeatures) -> NewWindowResponse<R>)`, with
  `Allow` (default popup), `Deny` and `Create { window: WebviewWindow }`. On
  Windows, wry's `NewWindowRequested` handler takes a deferral, hops to the
  message loop (`dispatch_handler`, so building a webview inside the callback
  does not deadlock) and, for `Create`, calls `args.SetNewWindow(webview)` +
  `SetHandled(true)`: WebView2 then loads the requested address into that
  webview and links it to its opener (this is what keeps `window.opener` and
  `postMessage`). The new webview must share the opener's WebView2 environment,
  which `WebviewWindowBuilder::window_features(features)` does
  (`features.opener().environment`; it also applies the page's requested size and
  position). It must be a `WebviewWindow` (a top-level window), not a child
  webview of `main`; `WebviewBuilder`/`add_child` popups are not supported by the
  API. `window.close()` from the popup reaches wry's `WindowCloseRequested`
  handler, which calls `DestroyWindow` on the popup's hwnd, so a page closing its
  popup needs no code here. How it is used: `navegador/popup.rs` (pure: label
  `navegador-popup-<n>`, `is_popup_label`, `count_open`, limit 3) and
  `viewer::{new_window, open_popup, navigation_allowed, download_handler,
  close_popups}`. The popup is built from `WebviewWindowBuilder` with
  `incognito(true)` (same private profile as the browser, so a sign-in is shared
  with its opener; per-controller, see T4a finding 2), the same T3 navigation
  policy, the same download logic (`download_handler` is shared), its own
  `on_new_window` (same limit: popups opened so far are counted from the live
  windows, so the cap is 3 in all), no capability, and starts on `about:blank`
  (the engine navigates it). Navigation inside a popup only reports a refusal;
  it never rewrites the address bar. `close()` and `shutdown()` destroy every
  popup first (a popup would otherwise keep the app alive after `main` is
  destroyed). Fallback kept: if building the popup fails, the address loads in the
  browser's own webview as before (opener lost; documented in `viewer.rs`); a
  popup refused for the limit is just refused with a message.
  ACL: `tests/app_acl.rs` `a_popup_window_gets_nothing_from_a_page_or_from_a_local_looking_url`
  (three popup labels, remote and local-looking URL, sensitive app commands, all
  navegador commands, fs plugin, event listen: all rejected) and
  `no_capability_selects_a_popup_window` (reads every capability file: no
  wildcard in `windows`/`webviews`, no `windows` entry, nothing named
  `navegador*`). Mutation check instead of RED (they pass on the current
  capability): adding `"navegador-popup-*"` to `webviews` made both fail; the
  capability was restored. Unit tests: `popup.rs` 6/6; mutating `has_room` and
  the label shape failed 2 of them. `cargo test --features navegador --test
  app_acl --test acl_manifest_guard` 11 + 4 pass; `cargo clippy --all-targets
  --features navegador` no warnings in `src/navegador` or the tests; `cargo fmt
  --check` ok. NOT observed (needs the user, cannot run a WebView2 here): that the
  popup opens, loads the sign-in address, reports back to x.com and closes on
  `window.close()`. Known risks to look at in that run: the popup is a separate
  top-level window that is not owned by `main` (owning it needs a
  `WebviewWindow` for `main`, which does not exist while a child webview does; it
  can therefore sit behind the app window); popups are NOT hidden when the
  browser is hidden by a section or tab switch; a popup's downloads share the
  quarantine registry (max 4 in flight in all).
- T5c-5a (non-PDF downloads go to the person's folder; Rust). Product decision:
  what EntropIA does not store is not EntropIA's. `download::route_for(suggested,
  url)`: a `.pdf` name goes to quarantine; any other extension goes straight to the
  download folder (the name wins over the address); a name that says nothing (no
  extension, `.php`/`.aspx`/`.bin`/`.tmp`..., `.hidden`) falls back to the
  address' extension (a `blob:` address has none), and if that says nothing
  either the file is checked in quarantine. A quarantined file that is not a PDF
  at `Finished` is MOVED to the folder (`finalize_or_release`), never deleted;
  only empty or over-cap PDFs and failures are deleted. `verify_part` now judges
  the type before the size, so a 300 MB zip is saved, not "too large" (the cap is
  for PDFs). A file routed to the folder is never read, moved or deleted by us: at
  `Finished` it is only statted for its size, and if it turns out to be a PDF it
  stays there (no auto-import). Names: `sanitize_name` (same rules as
  `sanitize_file_name`, keeps the extension, adds none, cuts the stem to keep
  it), `unique_name` (` (1)`, ` (2)`... before the extension, the number survives
  the 120-char limit; after 9999 it falls back to a UUID), never overwriting:
  `Registry::begin_in_folder` chooses the final name under the registry lock against
  the disk and against downloads in flight; a moved file claims its name with an
  exclusive create first. Display bug: `.pdf` is appended only in
  `DownloadDraft::finished(Ok)`; a rejected `YOLO-object-detection-master.zip`
  keeps that name. Draft: new status `saved`, new field `savedTo` (the
  directory, never a quarantine path). Folder: settings key
  `navegador_download_dir` (`app_settings`, via `settings::persist_setting`, now
  `pub(crate)`), new commands `navegador_download_dir` (get, and primes the
  browser) and `navegador_set_download_dir(path)` (validates: text, absolute,
  exists, is a directory; refuses with the reason), both in `generate_handler!`,
  `APP_COMMANDS`, the `main` capability and the ACL tests (rejected from
  `navegador-web` and popups); `navegador_open` also primes the folder from the
  setting so a first download cannot precede the choice. The folder is resolved at
  download time: the chosen one while it is a directory, else the OS Downloads
  folder (`app.path().download_dir()`), else the download is refused (`io_error`).
  Not offered: "show in explorer" (no existing command opens an arbitrary path
  safely; `app_logs::open_path` is for the log directory), so the UI shows the
  path only.
  RED: `download.rs` failed to compile against 94 missing symbols (route_for,
  sanitize_name, unique_name, resolve_folder, validate_folder,
  finalize_or_release, Outcome, Route, begin_in_folder, saved, saved_to, folder
  errors); `app_acl` 4 of 11 failed with "navegador_download_dir not allowed.
  Command not found" before the command was registered. One GREEN-run failure was
  a real bug found by a test: `unique_name` cut the number off a 120-char name
  (fixed by cutting the stem, not the suffix). GREEN: `cargo test --features
  navegador --lib navegador` 146/146, `--test app_acl` 11/11, `--test
  acl_manifest_guard` 4/4; `cargo test --no-fail-fast` (no feature) 1453 passed, 1
  failed (the known `no_other_module_opens_the_archive_by_hand`, same three sync
  test files) plus every integration test ok; `cargo clippy --all-targets` with and
  without `--features navegador`: no warnings in `src/navegador`, `settings.rs` or
  the tests; `cargo fmt --check` ok. Tooling note: the user's `tauri dev` (a
  running `cargo run`) held both the exe and Cargo's package-cache lock, so the
  writer used `CARGO_TARGET_DIR=src-tauri/target/writer` and a `CARGO_HOME` at
  `C:\cgh` whose `registry` and `git` are junctions to the real ones.
  Mark-of-the-Web: NOT verified (no WebView2 to run here). Source facts: wry
  0.55.1 sets `SetResultFilePath(destination)` + `SetHandled(true)` in
  `DownloadStarting`, and neither wry nor tauri touch `Zone.Identifier`; whether
  WebView2 (Chromium's download quarantine, `IAttachmentExecute`) stamps the file
  is decided by the engine, at completion, on the file at its final path, so
  changing `destination` should not lose it, but that is an inference. A file
  moved out of quarantine keeps its streams (a rename within a volume keeps them; a
  cross-volume copy uses `fs::copy`, which copies alternate data streams on
  NTFS). To check, in PowerShell: `Get-Item -Path 'C:\Users\<you>\Downloads\<file>'
  -Stream *` (a `Zone.Identifier` stream should be listed) and
  `Get-Content -Path '<file>' -Stream Zone.Identifier` (`ZoneId=3`).
  Limitations: names are not checked against dangerous extensions (`.exe`,
  `.lnk`, `.bat` are saved like any file; Windows and SmartScreen handle them by
  Mark-of-the-Web); the exists-check and WebView2's create are not atomic (a
  file created by another program in between can be overwritten); a failed or
  cancelled download in the folder leaves whatever WebView2 left (we never delete
  in that folder).
- T5c-5b (download folder in the panel; frontend). `DownloadDraft` gains `savedTo`
  and status `saved` (mirrors Rust); the downloads header reads "Carpeta de
  descargas: <path>" with a "Cambiar" button that opens the dialog plugin in
  directory mode (`dialog:allow-open`, already granted to `main`) and sends the
  pick to `navegador_set_download_dir`, which validates it; a refusal is shown
  ("No se pudo usar esa carpeta: <reason>") and the old folder stays; cancelling
  the dialog changes nothing. A `saved` item shows the chip "Guardado", its size
  and "Guardado en <carpeta>", no hash and no rejection reason. The section title
  is now just "Descargas" (it is no longer all quarantine). The header only shows
  once the list has an item, so the folder cannot be changed before the first
  download (the first one goes to the OS Downloads folder): a deliberate
  minimum. No "show in explorer" action (see T5c-5a).
  RED: 2 new `describeDownload` tests and 5 new `NavegadorView` tests failed
  (no `savedTo`, no folder row, no `saved` rendering). GREEN: `navegador-capture.test.ts`
  and `NavegadorView.test.ts` pass (view: folder shown, pick and save, dialog
  cancelled, refusal shown with the old folder kept, saved file with its
  folder). Final frontend run for the five fixes: `pnpm test` 299 store + 800 ui +
  2727 desktop passed (7 skipped); `VITE_LOCAL_ML=0` desktop 2706 passed (28
  skipped); `pnpm typecheck` and `VITE_LOCAL_ML=0` desktop typecheck 0 errors;
  `pnpm lint` only the known `WritingView.svelte:1403` error (the warnings are in
  files this work did not touch); prettier clean on every touched file;
  `VITE_NAVEGADOR=1 vite build` emits the NavegadorView chunk.
- T5c-2b (close the sign-in popup when its page closes it). The user's run showed
  the popup stays open after login: `window.close()` is not honoured. Cause, read
  in the source: wry 0.55.1 `attach_handlers` subscribes `WindowCloseRequested` and
  calls `DestroyWindow(hwnd)` on the webview's own container window only, so the
  top-level popup window stays, empty. Checked first, nothing better exists:
  tauri 2.11.6 has no close-requested hook for a webview (the window event
  `CloseRequested` fires only for the OS window, which is never asked), and
  `NewWindowResponse::Create`/`window_features` carry none. Fix, Windows:
  `viewer::close_when_page_closes` runs `WebviewWindow::with_webview` (main thread)
  and calls `controller().CoreWebView2()?.add_WindowCloseRequested(
  &webview2_com::WindowCloseRequestedEventHandler::create(..), &mut token)`; the
  handler destroys the popup's Tauri window (`get_webview_window(label).destroy()`),
  only for labels that pass `popup::is_popup_label`. `webview2-com` was not
  reachable from the app crate, so it is now a direct Windows dependency pinned
  `=0.38.2`, the version already in Cargo.lock (the lock only gains the
  `webview2-com` line in the app's dependency list; no new crate; the
  `entropia-agent` source line is intact). The 3-popup cap counts live windows, so a
  closed popup (by the page or by the person) frees its slot on its own.
  macOS and Linux: not wired, documented in `viewer.rs`. wry has no
  `webViewDidClose:` delegate on macOS; on Linux its `close` signal only destroys
  the GTK widget, not the tao window. There a popup is closed by the person, or
  when the browser or the app closes. No platform code written that cannot be
  compiled here.
  Not unit-testable (a COM subscription on a live WebView2); the `is_popup_label`
  guard it relies on is covered by the `popup.rs` tests. Observed: `cargo check
  --features navegador` compiles, see the verification list in the commit. NOT
  observed: the popup closing. User recheck: sign in with Google on x.com; the popup
  must close by itself right after the login; open a popup and close it by hand,
  then open three more in a row (the cap must have freed the slot).
- T8 (browser tabs; route: delegated writer; T8 stays open until the Windows run).
  Commits `aaf0f868` (Rust) and the one carrying this note (frontend). What
  `on_new_window` exposes (tauri 2.11.6 `NewWindowFeatures`): `size()` and
  `position()` as `Option`, plus the opener. WebView2 fills them only when the page
  passed them (`HasSize`/`HasPosition`), WKWebView likewise; wry's WebKitGTK always
  reports none. Split (`tabs::placement`): no size and no position -> tab (the
  handler answers `Deny` and builds a child webview itself); any size or position
  -> popup window as before (needs `window.opener`). Where the engine reports
  nothing (Linux) every request stays a popup. A tab cannot keep `window.opener`
  (child webview, not a top-level window). Tabs: `navegador/tabs.rs` (labels
  `navegador-web-<n>`, ids never reused, max 4, `TabList`, revisioned
  `BrowserState`); the first tab is `navegador-web-1` (old `navegador-web` label
  retired; ACL tests still reject it). Backend owns the active tab; every
  page-acting command takes the tab id explicitly (justification: what the person
  clicked is what is acted on). New commands `navegador_new_tab|activate_tab|
  close_tab` in handler, `APP_COMMANDS`, capability, ACL tests. Downloads carry
  `tab`. `shutdown`/`close` destroy all tabs and popups. Only the active tab is
  visible, others hidden alive; a blank tab has no webview until first navigation.
  Page-initiated tab in the foreground only if the opener tab is active.
  RED/GREEN: `tabs.rs` 22/22 failed on `unimplemented!()` then 22/22, plus a
  revision test (compile RED) -> 23/23; download `tab` tests failed to compile then
  81/81; frontend `navegador-tabs.test.ts`, store/capture/navegador/view tests
  written first (view 20 failed against the old view, then 42/42). ACL mutation
  checks: adding `navegador-web-*` to the capability failed 4 app_acl tests and 2
  guard tests; changing `MAX_TABS` failed the new guard test.
  Verification: `cargo test --no-fail-fast` 1481 passed, 1 failed (known
  `no_other_module_opens_the_archive_by_hand`); `--features navegador --lib
  navegador` 174/174; `--test app_acl` 12/12, `--test acl_manifest_guard` 6/6;
  `cargo check --features navegador` ok; clippy `--all-targets` with and without
  the feature: no warnings in navegador or the ACL tests; `cargo fmt --check` ok;
  `pnpm typecheck` 0 errors; `VITE_LOCAL_ML=0` desktop typecheck 0 errors;
  `pnpm test` 299 + 800 + 2781 passed (7 skipped); Lite 2760 passed; `pnpm lint`
  only the known WritingView error; prettier clean on touched files;
  `VITE_NAVEGADOR=1` vite build emits NavegadorView. NOT observed: any real
  WebView2 behaviour (tab creation inside the callback thread, hide/show, popup
  split). Limitations: no per-tab back/forward state; a tab made by a page
  needs known bounds (the view must have shown the browser once); opener link
  lost for tabs; Linux always popups; macOS/Linux untested.

- P2a (`c07c5e4f`). `packages/store/src/runner.ts` entry `0055_web_captures`
  (mirrored in `src/migrations/0055_web_captures.sql`, like 0032-0037), applied
  through the atomic `BEGIN IMMEDIATE` path, and `schema_full.sql` regenerated
  (`pnpm --filter @entropia/store export-schema`). Tables as created:
  `web_sources(id PK, original_url, final_url, canonical_url?, title?,
  site_name?, first_accessed_at TEXT, created_at INT, updated_at INT)` with
  indexes `idx_web_sources_final_url` (not unique on purpose: two devices may
  save the same page before they meet) and `idx_web_sources_updated`;
  `web_captures(id PK, web_source_id FK ON DELETE CASCADE, accessed_at TEXT,
  final_url, kind CHECK page|selection|pdf, mime_type, text? CHECK <= 524288
  bytes, text_rel_path?, quote_prefix?, quote_suffix?, rel_path?, sha256,
  hash_of CHECK html|quote|pdf, size_bytes, extractor_version?, title?,
  created_at INT)` with `idx_web_captures_source(web_source_id, accessed_at
  DESC)`. No FTS (P2c searches with LIKE). Deviation from the brief: the rest of
  the archive stores `created_at`/`updated_at` as INTEGER epoch milliseconds
  (not ISO text), so those two follow that; `accessed_at` and
  `first_accessed_at` are RFC 3339 UTC TEXT on purpose (provenance, shown
  exactly as recorded, the form CSL/Zotero take). `original_url` equals the
  first capture's `final_url`: the draft carries no redirect chain.
  Not in `SYNCED_TABLES`, no sync triggers; the Rust sync tests have no "every
  table is classified" guard (they iterate `SYNCED_TABLES` only), so nothing
  needed classifying; `cargo test --lib sync::` 281 passed.
  Migration order and schema tag (findings): the runner computes `pending` as
  every registry name missing from `_migrations`, sorted, so 0055 applies on an
  install that has only 0037, and on one that already recorded a higher name
  (test: `applies by name even when a higher number was recorded first`).
  `read_schema_tag` is `_migrations ORDER BY id DESC LIMIT 1` (the latest APPLIED,
  not the highest name) and the Cloud server keeps `max(stored, X-Schema-Tag)`
  lexicographically (PROTOCOL "Ciclo de vida de schema_tag"). Monotonic today:
  a fresh install ends on 0055 (sorted), an upgrade appends 0055 last. RISK once
  the Zotero branch lands: a device that already applied 0055 and then applies
  0038-0054 gets head `0054_...` < the account's stored `0055_web_captures`, so
  the server answers 426 for it. Plan §5 already says the number is fixed at
  merge time after that range; that is the moment to renumber 0055 (or make the
  tag the max name instead of the last applied). Not changed here.
  RED: 5 of 7 new runner tests failed (no such table / name missing; the other
  two passed vacuously), the drizzle column test failed without the schema
  entries. GREEN: store 309 passed, `tsc --noEmit` 0 errors. Decision: Rust is
  the one writer, so no TS write repository exists; the Drizzle tables are there
  for the P2c reads (the read repository is written with its queries then).
- P2b Rust (`55fe7157`). `navegador/save.rs` (pure over a data dir and a
  connection, compiled in every build) plus commands `navegador_save_capture
  (draft_id)`, `navegador_save_download(download_id)` and
  `navegador_discard_draft(draft_id)` (in `generate_handler!`, `APP_COMMANDS`,
  the `main` capability and the `app_acl` rejections; they do not need the
  browser, so they are not gated by `ensure_available`). Writes use the
  existing `open_archive_connection` (no new module opens the archive by hand).
  `CaptureDraft` got an `id` (uuid); `Holds` (managed lazily like the viewer's
  state) keeps up to 4 drafts with their HTML (oldest evicted, dropped on save
  or `navegador_discard_draft`) and up to 50 verified PDFs (`ReadyPdf`: id, url,
  sanitized name, size, sha256, accessed_at, page title, filled by
  `download_finished`). A failed save keeps the draft held (retry); saves are
  serialised so a double click cannot save twice. Order: validate (kind, sizes,
  http/https/blob URL, sha256 recomputed over html or quote), find the source by
  `final_url` (reuse its directory) or mint ids, write each file as
  `.<name>.tmp` in `web-captures/<source_id>/`, `sync_all`, rename, then one
  `BEGIN IMMEDIATE` transaction (source insert or `title`/`updated_at` update,
  capture insert); on any later failure the files written, the temp file and the
  source directory if new are removed. Stored keys are `web-captures/<source>/
  <capture>.<ext>` (forward slashes). Page: `.html` in `rel_path`; text in the
  row up to 512 KB (bytes), else `.txt` in `text_rel_path`. Selection: quote in
  the row (or `.txt` over 512 KB). PDF deviation from "rename": the file is
  COPIED out of quarantine while hashing (size and sha256 must equal what was
  verified, else `hash_mismatch` and nothing stays) and the quarantined file is
  deleted only after the commit, so a failed save or a crash never loses the
  PDF (a rename into a temp name would need a second move back on failure, and
  cross-volume needs the copy anyway). Directory fsync is not done (not portable
  on Windows). `extractor_version` is `navegador-capture-1` for page and
  selection, NULL for a PDF.
  RED: `capture.rs` id test failed to compile; `save.rs` 19 of 22 tests failed on
  `todo!()` (the 3 passing were the pure holds/serde ones); `app_acl` 1 of 12
  failed ("Command navegador_save_capture not found") before the stubs. GREEN:
  `save` 22/22, `app_acl` 12/12, `acl_manifest_guard` 6/6 (with and without
  `--features navegador`). Mutation check: dropping the file cleanup after a DB
  failure failed 3 tests. Cases covered: page, selection and pdf save, find-or-
  create by final URL (title refreshed, kept when the new capture has none),
  512 KB boundary by bytes, quote over 512 KB, DB failure removes files and
  source dir and keeps an earlier capture's files, PDF stays in quarantine on DB
  failure, tampered PDF refused, missing PDF, id that climbs out of quarantine,
  hash not matching content, invalid drafts, unknown draft/download, retry after
  failure, no double save, bounded holds.
- P2b UI (`89c9603b`). `CaptureDraft.id`; `navegadorStore` gained `saving`,
  `saved` and `saveErrors` and `saveCapture`/`saveDownload` (deduplicated,
  retryable, errors kept as code + detail and mapped to messages in es/en);
  replacing or dismissing a draft calls `navegador_discard_draft`. The panel
  shows a "Guardar" button (draft and each verified PDF), then "Guardando…" and
  "Guardado" (disabled); the title no longer says "todavía no se guarda".
  RED: 16 new lib tests failed, 5 of 7 new view tests failed. GREEN: `pnpm test`
  309 store + 800 ui + 2816 desktop passed (7 skipped); Lite
  (`VITE_LOCAL_ML=0`) navegador view + lib 1722 passed; `pnpm typecheck` and the
  Lite desktop typecheck 0 errors; `pnpm lint` only the known
  `WritingView.svelte:1403`; prettier clean on touched files;
  `VITE_NAVEGADOR=1 vite build` emits NavegadorView.
- P2 verification (Rust, `CARGO_TARGET_DIR=src-tauri/target/writer`): `cargo
  test --no-fail-fast` 1510 lib passed, 1 failed (the known
  `no_other_module_opens_the_archive_by_hand`, still only the three sync
  `*_tests.rs` files) and every integration test ok; `cargo test --features
  navegador --lib navegador` 203 passed; `cargo check --features navegador` ok;
  `cargo clippy --all-targets` with and without the feature: no warnings in
  `save.rs`, `navegador/` or `tests/app_acl.rs` (the remaining warnings are the
  old `sync/writing_*` ones); `cargo fmt --check` ok; Cargo.lock untouched.
  NOT observed: any real run (needs the user): saving from the live browser,
  files under `%APPDATA%\com.entropia.shared\web-captures\`, rows surviving a
  restart (the DB browser view lists `web_sources` and `web_captures`: it shows
  every table `sqlite_master` holds).
  Limitations: no listing yet (P2c); orphan files from a crash between rename
  and commit stay until P2d's sweep (a `.tmp` too); a draft not saved before 4
  newer ones is evicted ("unknown_draft"); a PDF held in quarantine older than
  24 h is swept and its save says `file_missing`; the `original_url` is the
  final URL; saving needs the migration to have run (the renderer runs it at
  startup).

- HAZARD found after P2a/P2b (2026-10-01): `tauri dev` opens the user's REAL
  shared archive (`%APPDATA%\com.entropia.shared`) and the sync session lives in
  the OS keyring (`com.entropia.lite sync`), shared across data directories.
  Applying `0055` there is irreversible and one sync would raise the account's
  `schema_tag` to `0055_web_captures`, locking every other device on 1.0.18 out
  with 426 (rule above). Signing out does not help: the migration stays in the
  archive. Do not run any dev build with a new migration on the real profile.
- Isolated dev profile (commit `feat(dev): run the desktop app on an isolated dev
  profile`). `src-tauri/src/dev_profile.rs`. Debug builds only (`debug_assertions`):
  `ENTROPIA_DEV_PROFILE=<name>` puts the data dir at
  `<data>/com.entropia.shared/dev-profiles/<name>` and the cache at
  `<local>/com.entropia.shared/dev-profiles/<name>`. A name, not a path, on purpose:
  the fs capability scope and `assetProtocol.scope` only cover
  `$DATA/com.entropia.shared/**` and `$LOCALDATA/com.entropia.shared/**`
  (`capabilities/default.json:203-204`, the three tauri configs); a directory
  nested under them is already inside both, so no config widens, while an
  arbitrary path could not be scoped without widening them. The name is
  `[A-Za-z0-9_-]{1,32}`, so it cannot climb out. A set-but-invalid value is a
  startup error, never a fall back to the real archive. Release builds ignore the
  variable: the only read (`std::env::var`) is inside a `#[cfg(debug_assertions)]`
  function and a test pins that (and that no other file reads it). Every
  resolution goes through `path_utils::resolve_and_remember_dirs`, which now
  applies the profile; the renderer's `resolve_data_dir`, `cache_dir`, the
  instance guard, the asset-protocol grant and logs all derive from it (no other
  code asks the OS for the shared dirs; checked with a search for
  `data_dir(`, `app_data_dir`, `local_data_dir`, `dirs::`). The legacy-dir
  migration is skipped in the profile (it would otherwise be eligible to pull
  legacy archives into it). The instance guard is per directory, so a profile
  can run next to the installed app.
  Sync in the profile: `start_engine` returns a dormant engine (no thread, no
  ticker; status `disabled` with message `sync_disabled_in_dev_profile: ...`);
  `session::token_entry` (the single door to the keyring service
  `com.entropia.lite sync`) refuses, so no read, write or delete of the real
  session; `sync_register_account`, `sync_login`, `sync_logout`, `sync_now`,
  `sync_full_resync`, `sync_set_auto` and `session_creds` (which every other
  server command, including `sync_delete_account`, uses) answer the same error;
  a source-reading test fails if any of those loses its `require_sync()`. The
  local-only commands (`sync_ensure_capture`, conflicts) keep working. LLM keys
  (`settings.rs`, a different keyring service) are untouched; a fresh profile
  database simply has no key references. The close sequence does not wait for a
  sync cycle there. UI: the Sync card shows "desactivada en el perfil de
  desarrollo aislado" and disables Iniciar sesion / Registrar cuenta; the error
  maps to a readable message.
  Startup line (stderr and the in-app log): `profile=dev:<name> data_dir=...
  cache_dir=... sync=disabled` (`profile=shared ... sync=enabled` otherwise), plus
  `[sync] disabled in the dev profile: no engine, no keyring access`.
  RED: 8 of 11 `dev_profile` tests failed on `todo!()`, then compile errors for the
  sync guard and the dormant engine; mutation: removing the guard from
  `token_entry` failed the guard test. GREEN: `dev_profile` 13/13, dormant engine
  1/1, UI tests (+3), `pnpm test` 309 store + 800 ui + 2819 desktop. Not done:
  a `cargo test --release` run (a release build compiles the whole crate; the
  property is covered by `requested(false, ..)` and the source test).

- INCIDENT (observed 2026-10-01 01:15, read-only query of the real archive): the
  user's `tauri dev` from this worktree had ALREADY applied `0055_web_captures`
  to `%APPDATA%\com.entropia.shared\entropia.sqlite` (`_migrations` id 38,
  applied_at 2026-09-30 22:58) while its sync session (server
  `https://entropia-cloud.app.hlab.com.ar`, auto-sync every 5 min, last sync
  01:14) was active. The account's `schema_tag` has very likely been raised to
  `0055_web_captures` already, so devices on 1.0.18 (head `0037_fts_vocab`) would
  get 426. Nothing was changed by the writer; remediation (server-side tag reset
  by the Cloud admin, or shipping 0055 in a release) is a decision for the
  owner. The isolated profile prevents any further case, not this one.

- P2d fix (`2f047dcd`). `save::commit` created `web-captures/<source_id>/` before
  knowing whether any file would be written, so a selection that fits the row
  left an empty folder. The folder is now created only when the plan has a
  payload. RED: 2 new tests failed (selection leaves no folder; a failed
  selection save creates none either); the third new test (a selection over
  512 KB still gets its folder for the `.txt`) passed on both sides. GREEN: `save`
  25/25.
- P2d sweep (`240ddaab`). `navegador/capture_files.rs` (the one safe way into the
  tree, shared with the delete: id = `[A-Za-z0-9-]{1,64}`, `locate_source_dir`
  refuses a link or junction at the root or the source folder and anything whose
  canonical path leaves the canonical root, `remove_dir_contents` deletes regular
  files only and never follows or removes links or sub-folders) and
  `navegador/sweep.rs`. Four rules, each only for things older than 1 h
  (`MIN_AGE`; nothing younger is touched, so a save in flight is safe): (1) temp
  files `.<name>.tmp`; (2) files no `web_captures.rel_path`/`text_rel_path`
  points at (key compared as `web-captures/<folder>/<name>`, backslashes
  normalised); (3) empty source folders; (4) folders of sources that no longer
  exist, removed whole only when every file and the folder itself is old. Rules 2
  and 4 need the database: if the tables are missing (the renderer has not
  migrated yet, e.g. the first start after an upgrade) or ANY query fails, they
  are skipped and the report says so; a failed read never means "nothing is
  referenced". Rules 1 and 3 do not depend on it. Only folders named like ids are
  looked at (anything else in `web-captures/`, loose files, `assets/`, the
  database stay), at most 50 000 entries per run. `navegador::sweep_captures`
  runs it from `setup` on a thread (after the sync blob cleanup), opens the
  archive through `open_archive_connection`, catches panics, and logs one line to
  stderr and the in-app log: `[navegador] web-captures sweep: N temporary
  file(s), N orphan file(s), N empty folder(s), N folder(s) of deleted sources
  removed; N refused, N error(s)` (plus `; orphan and deleted-source rules
  skipped (database not ready)` when it applies). It respects the dev profile
  because the data dir already flows through `path_utils`. The folder of a source
  whose delete left files behind (see P2c) is exactly rule 4.
  RED: `capture_files` 5 of 6 and `sweep` 14 of 16 failed on `todo!()` (the two
  passing were the id-shape and summary tests). GREEN: `capture_files` 6/6, `sweep`
  17/17 (one test reads `lib.rs` and fails if setup stops calling the sweep).
  Link tests really ran (a junction or symlink is made with `symlink_dir` or
  `mklink /J`; they skip with a message only where neither works). Mutation
  check: removing the three layers that refuse a link (root/source `is_symlink`,
  the `is_dir` check and the canonical containment) made the link test fail (the
  file behind the link was deleted); removing only the first two layers did not,
  because the third still refused, which is the point of having three.
- P2c Rust (`423880e5`). Reads live in Rust (`navegador/sources.rs`, commands
  `navegador_list_sources(query?, limit?)`, `navegador_source_detail(sourceId)`,
  `navegador_delete_source(sourceId)`), not in a TypeScript repository over
  `db_select`. Why: Rust already owns the writes (P2b), the list shows file
  presence which needs the file system, a delete changes rows and files together,
  and three typed commands are a smaller surface than SQL assembled in the
  renderer. They open the archive with `open_archive_connection` in
  `spawn_blocking` (like the saves), are not gated by `ensure_available`, are in
  `generate_handler!`, `APP_COMMANDS`, the `main` capability and `app_acl` (22
  commands now rejected from `navegador-web-*`, `navegador-popup-*` and lookalike
  origins). The Drizzle entries from P2a stay unused by reads (no TS repository
  was needed). List: newest first by `updated_at`, capture count, distinct kinds,
  `limit` (default 200, max 500). Search: substring over title, site name, the
  three URLs and, through the captures, text, title and final URL, `LIKE ... ESCAPE`
  with `%`, `_` and the backslash made literal, trimmed and cut to 200 characters,
  a source listed once however many captures match. Case: SQLite folds ASCII
  only, so the query is also tried lower, upper and capitalised (`educación`
  finds `EDUCACIÓN` and `Educación`, not `eDUCACIÓN`). Text over 512 KB lives in a
  file and is not searched. Detail: captures newest first by `accessed_at`, UTC
  string as recorded, hash, kind of hash, size, preview (first 2000 characters
  from SQL), `textInFile`, quote prefix/suffix, `filePresent` (true/false/null
  when the capture has no file): the check accepts only a key that is exactly
  `web-captures/<source_id>/<plain name>` and a regular file inside a located
  folder, so a stored path like `../x`, another source's folder or an absolute
  path is never read. Delete: validates the id, then one `BEGIN IMMEDIATE`
  transaction (existence check, captures, source: captures are deleted explicitly
  so it does not depend on `foreign_keys`), then the folder through
  `capture_files`; a folder that is a link, has a sub-folder or will not go
  returns `leftoverFiles: true` and never fails the delete; the folder of a
  deleted source is what the sweep removes (a test runs the sweep on that state).
  Copies in collections are not touched (the module reads and writes only the two
  web tables). Selection saves after the P2d fix leave no folder, which the delete
  also tolerates. RED: 25 of 27 `sources` tests failed on `todo!()` (the 2 passing
  were serialisation shape); `app_acl` 5 of 12 failed ("not allowed. Command not
  found") before registration. GREEN: `sources` 27/27, `app_acl` 12/12,
  `acl_manifest_guard` 6/6 (with `--features navegador`).
- P2c UI (`14805b54`). `lib/navegador-sources.ts` (types, three `invoke`
  wrappers, `parseSourceError`, `formatLocalTime` (local zone, 24 h in Spanish),
  `describeSource`, `describeCapture`), `views/NavegadorSources.svelte` (the
  drawer) and a toolbar button (`list` icon, existing `ActionIcon`) in
  `NavegadorView`. The drawer sits beside the placeholder in a flex row
  (`navegador-view__body`), never over it: opening it narrows the placeholder and
  the existing ResizeObserver moves the native webview; a test checks neither
  contains the other (the real layout is for the user to see). Search is
  debounced 250 ms and the newest request wins (a slow answer to an older search
  is dropped, tested). A capture saved while the drawer is open refreshes the list
  and the open detail (watches `navegadorStore.saved`). Detail shows the three
  addresses, first visit (UTC), every capture with kind, local time + UTC, final
  URL, short hash, size, file presence (a saved HTML snapshot or PDF is only
  reported, never opened: showing HTML needs active content stripped first, not
  built), a missing-file warning, quote with context for selections, text
  preview and "text is in a file". Actions: open in the browser (the view's own
  `go()`, the same path as typing the address: first call opens the browser,
  later ones `navegador_navigate` the active tab, so the T3 policy applies and a
  refusal shows in the status line), copy address (clipboard, with a notice), and
  delete through the existing `ConfirmDialog` (the overlay root hides the webview
  as for any dialog); a failed delete stays in the dialog with its message, a
  `not_found` closes it and refreshes, a leftover shows "some files are cleaned
  up when the app starts". Search field follows the project's search-field
  pattern (a design-tokens guard failed first because the magnifier was missing).
  Everything from pages renders as text (hostile markup tests for the list and the
  detail). es + en keys under `navegador.sources.*`.
  RED: `navegador-sources.test.ts` failed to import, then 2 of 15 failed on the
  12-hour clock of `es-AR` (fixed with `hourCycle: 'h23'`); `NavegadorSources.test.ts`
  27/27 failed against the old view, 26 passed with the component and the last
  was a test set-up error (the session only counts as open when this view opened
  it), fixed. GREEN: 15/15 and 27/27.
- P2c/P2d verification (Rust with `CARGO_TARGET_DIR=src-tauri/target/writer`): `cargo
  test --no-fail-fast` 1577 lib passed, 1 failed (the known
  `no_other_module_opens_the_archive_by_hand`, still only the three sync
  `*_tests.rs` files) and every integration test ok; `cargo test --features
  navegador --lib navegador` 256 passed; `--test app_acl` 12/12 and `--test
  acl_manifest_guard` 6/6 with the feature; `cargo check --features navegador` ok;
  `cargo clippy --all-targets` with and without the feature: no warnings in the
  navegador modules or `tests/app_acl.rs`; `cargo fmt --check` ok; Cargo.lock
  untouched. Frontend: `pnpm typecheck` 0 errors; `VITE_LOCAL_ML=0` desktop
  typecheck 0 errors; `pnpm test` 309 store + 800 ui + 2861 desktop passed (7
  skipped); `VITE_LOCAL_ML=0` desktop 2840 passed (28 skipped); `pnpm lint` only
  the known `WritingView.svelte:1403`; prettier clean on touched files
  (`format:check` still lists only the three known files); `VITE_NAVEGADOR=1` vite
  build emits NavegadorView. No migration was added or changed and no real
  archive was opened (tests use temp dirs and in-memory databases).
  NOT observed (needs the user): the drawer's real layout beside the native
  webview, the sweep log line, a real delete.
  Limitations: the 1 h age uses file and folder modification times (a clock set
  far back could make fresh files look old); the sweep's first run after an
  upgrade skips the database rules because the tables do not exist until the
  renderer migrates (the next start does them); text over 512 KB and saved HTML
  are not searchable or viewable; case folding beyond ASCII covers lower, upper
  and capitalised spellings of the query only; the list shows at most 500 sources
  and has no pagination; the drawer closes when the view is left (it is local
  state); deleting does not touch sync (these tables are not synced yet, P3 must
  add the delete path).
- P2e (user decisions 2026-10-01; `c5884034` Rust and ACL, `73a7263e` UI).
  Provenance: `save::save_pdf` stores the download's page snapshot as the source
  (`original_url` = `final_url` = page URL, `title` = page title) and the file link on
  the `web_captures` row (`CaptureInfo.url`, new; `insert_rows` no longer reuses the
  source address for the capture). The page must be http(s) and pass
  `url_policy::check_url(.., Typed)` (Typed because the person could have typed an
  `http` page; private hosts, `file:`, `about:blank`, `javascript:` are refused); a
  refused or unknown page falls back to the file link as before. Find-or-create by
  page URL means a PDF joins the source of a page capture of that article. With a
  page URL but no title the source title is left alone (COALESCE keeps the page
  capture's title) and only the capture title falls back to the file name.
  Viewer reuse decision: the corpus viewer is `@entropia/ui` `DocumentViewer`
  (pdf.js over `assetUrl`), not tied to `assets` rows (it takes `path`, `type`,
  `assetUrl`, `readOnly`), and `SimilarAssetPreviewDialog` already uses it read only.
  So no pdfium command and no second viewer: the thinnest adapter is ONE command,
  `navegador_pdf_file(captureId) -> path` (`sources::pdf_capture_file`): validates the
  id like a source id, requires `kind = 'pdf'`, then resolves `rel_path` with the same
  rule as the file-presence report (now `capture_file`: exactly
  `web-captures/<source_id>/<plain name>`, real folder inside the root, never a link,
  regular file only). Codes `invalid_id`, `not_found`, `not_a_pdf`, `file_missing`,
  `db_error`. The renderer never sends a path; the asset protocol scope already covers
  the data dir (also the dev profile). New `NavegadorPdfViewer.svelte` is laid over the
  page area (`navegador-view__stage`, absolute) and `NavegadorView` hides the native
  webview while it is open (`covered = overlayOpen || pdf`, same effect that serves
  `[data-overlay-root]` dialogs) and shows it again on close; a missing file shows a
  message and the close button still works. Offline: it only reads the local file.
  Actions: PDF-only sources show "Abrir página de origen", others keep "Abrir en el
  navegador" (same `go()` path, `sourceOpenAction`); each PDF capture whose file is on
  disk gets "Ver PDF guardado". Duplicates: at download finish Rust looks up the sha256
  in `web_captures` (`sources::source_of_pdf_sha`, kind pdf, digest-validated); if
  found the draft carries `alreadySavedIn` (source id), the quarantined file is
  DELETED (identical bytes) and nothing is held, so the entry shows "Ya está en tus
  fuentes" + "Ver fuente" (opens the drawer on that source) and no Guardar. Backstop:
  `save_pdf` refuses a hash already saved with `already_saved`. The same PDF
  downloaded again before saving: `Holds::arrive` keeps one held copy (newest) and
  deletes the older quarantine file; the list collapses same-sha verified entries to
  the newest (`upsertDownload`), chosen over marking duplicates because the older
  entry's file is gone and a list of identical rows is the complaint. Running,
  rejected and user-folder downloads never collapse. No migration touched.
  RED: Rust 17 of the new tests failed (`todo!()` stubs, provenance and hold
  assertions); TS 12 failed in `navegador-capture/sources` tests and 9 of 11 in
  `NavegadorPdf.test.ts`. Mutation: `covered = overlayOpen` alone made the
  hide-while-viewing test fail. GREEN: `cargo test --features navegador --lib
  navegador::` 276 passed, `--test app_acl` 12/12 and `--test acl_manifest_guard` 6/6
  (23 navegador commands rejected from `navegador-web-*`, `navegador-popup-*` and
  remote origins). Verification (`CARGO_TARGET_DIR=src-tauri/target/writer`): `cargo test
  --no-fail-fast` 1597 lib passed, 1 failed (the known
  `no_other_module_opens_the_archive_by_hand`, still only the three sync `*_tests.rs`),
  every integration test ok; `cargo check --features navegador` ok; `cargo clippy
  --all-targets` with and without the feature: nothing in navegador or `app_acl`;
  `cargo fmt --check` ok. Frontend: `pnpm typecheck` 0 errors; `VITE_LOCAL_ML=0` desktop
  typecheck 0 errors; `pnpm test` 309 + 800 + 2887 passed (7 skipped);
  `VITE_LOCAL_ML=0` desktop 2866 passed (28 skipped); `pnpm lint` only the known
  `WritingView.svelte:1403`; prettier clean on touched files (`format:check` lists only
  the three known files); `VITE_NAVEGADOR=1` vite build ok. Three existing tests listed
  different files under one hash; they now use distinct hashes (same hash is the same
  PDF by design).
  NOT observed (needs the user): the real viewer (pdf.js) over the native webview, the
  webview hiding and returning, offline, a real download from an article page.
  Limitations: sources saved before this change keep the file-link URL (and their
  "Abrir página de origen" loads that link, which downloads again); the page URL is the
  tab URL at download time, normalised by the URL parser, so an article captured under
  a different spelling of its address (fragment, tracking query) is a separate source;
  a source deleted after its duplicate was flagged leaves a stale "Ver fuente" (the
  drawer then says it is gone); the duplicate check needs the archive open at finish,
  and any trouble counts as "not saved" (the save itself still refuses a true
  duplicate); Cargo.lock untouched.
  Windows checklist (dev profile): 1. Download a PDF from an article page and save it:
  the source shows the article page URL and title. 2. "Abrir página de origen" loads the
  article and downloads nothing. 3. "Ver PDF guardado" shows the local copy with the
  browser hidden; close it and the page returns; repeat with the network off. 4.
  Download the same PDF again: "Ya está en tus fuentes", no Guardar, "Ver fuente" opens
  its source. 5. Capture the article page and download its PDF: both land in one
  source. 6. Download the same new PDF three times before saving: one entry.
