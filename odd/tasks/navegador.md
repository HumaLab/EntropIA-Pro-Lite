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

- [ ] P2a — Migration `0055_web_captures` (never 0038–0054, owned by the
  Zotero branch) with `web_sources` and `web_captures` per plan §5, plus a
  store repository with tests. Shared archive: both Lite and Pro get it.
- [ ] P2b — Save a capture draft or a verified PDF: files under
  `<data>/web-captures/<source_id>/<capture_id>.<ext>`, file first then the
  DB transaction, text over 512 KB to a file, captures immutable.
- [ ] P2c — Saved sources list inside the Navegador: search by title, URL and
  text; source detail with its captures; open the original URL; delete a
  source and its local files.
- [ ] P2d — Startup sweep of partial/orphan capture files; errors that never
  damage the archive.

- [ ] T7 — Repeat the §10 matrix on macOS (WKWebView) and Linux (WebKitGTK).
  Known gaps there: sign-in popups do not close on `window.close()` (wry does
  not wire `webViewDidClose:` / GTK `close`), and macOS reports no download
  path. Engine A is approved for Windows only until then.

## Follow-ups (outside this feature)

- Pre-existing bug on main, not caused by the ACL change: `readAssetSize`
  (`apps/desktop/src/lib/collection-import.ts:88`) calls fs `stat()` on the
  relative storage key that `split_pdf_pages` returns
  (`store_asset_path_at_boundary`, `src-tauri/src/ocr/commands.rs:~747`), so
  the fs plugin rejects it ("forbidden path: assets/...") and every split PDF
  page asset is stored with `size = NULL`. Fix: resolve against the data dir
  before `stat`, or have the command return the size.

## Verification evidence

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

