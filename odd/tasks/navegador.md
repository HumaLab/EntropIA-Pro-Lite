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
- [ ] T2a — Upgrade Tauri to 2.11.6 (route: delegated writer). 2.11.1 fixes
  GHSA-7gmj-67g7-phm9 (remote `http://<scheme>.evil.com` treated as local
  on Windows); 2.11.6 also fixes GHSA-w28w-mhc8-qvjv (high, <=2.11.5:
  cross-webview theft of channel responses). Not 2.12.0 (new minor, 3 days
  old). Checks: lean `cargo check`/`cargo test`, lint, typecheck, Vitest.
- [ ] T2b — App ACL manifest (route: delegated writer, same unit chain):
  `AppManifest::commands` for every registered command, generated
  permissions granted by webview label (`webviews: ["main"]`, not
  `windows`, because a window match covers its child webviews,
  `ipc/authority.rs:459-460`). The external-page webview gets no app or
  plugin permission. Guard test: registered commands == manifest ==
  granted. Security test: from a remote-origin webview, `db_execute` and
  other sensitive commands are rejected by the ACL.
- [ ] T2c — User verifies the main app keeps working (Lite) before any
  browser webview is integrated.
- [ ] T3 — URL policy (pure Rust, TDD): allow https, http only when typed by
  the user; block file:, data:, javascript:, loopback, private ranges, cloud
  metadata; applied to navigation, redirects and new windows.
- [ ] T4 — Prototype child webview behind an experimental flag: open,
  navigate, back/forward/reload, bounds follow the pane, close.
- [ ] T5 — Capture without IPC: page text, selection with context, HTML
  snapshot (platform script evaluation with result), PDF download via
  `on_download`.
- [ ] T6 — Windows verification matrix with the user (plan §10), then decide
  the engine and update the plan.

## Progress

- 2026-09-29: feature document created. Next: T1 (user decision).
- Engram mirror `odd/navegador/tasks`: PENDING (MCP save refused: several
  active sessions; CLI save failed: database locked). Resync when available.

## Verification evidence

(none yet)
