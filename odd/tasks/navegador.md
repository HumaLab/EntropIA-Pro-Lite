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
- [ ] T5 — (route: delegated writer) Capture without IPC: page text, selection with context, HTML
  snapshot (platform script evaluation with result), PDF download via
  `on_download`.
- [ ] T6 — Windows verification matrix with the user (plan §10), then decide
  the engine and update the plan.

## Progress

- 2026-09-29: feature document created. Next: T1 (user decision).
- Engram mirror `odd/navegador/tasks`: PENDING (MCP save refused: several
  active sessions; CLI save failed: database locked). Resync when available.

## Follow-ups (outside this feature)

- Pre-existing bug on main, not caused by the ACL change: `readAssetSize`
  (`apps/desktop/src/lib/collection-import.ts:88`) calls fs `stat()` on the
  relative storage key that `split_pdf_pages` returns
  (`store_asset_path_at_boundary`, `src-tauri/src/ocr/commands.rs:~747`), so
  the fs plugin rejects it ("forbidden path: assets/...") and every split PDF
  page asset is stored with `size = NULL`. Fix: resolve against the data dir
  before `stat`, or have the command return the size.

## Verification evidence

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
