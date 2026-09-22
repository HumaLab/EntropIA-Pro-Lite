# Microsoft Store update notice (Lite)

## Objective

EntropIA Lite, when installed from Microsoft Store on Windows, detects on
startup whether Store has an update for its main package and shows a
dismissible notice that opens the Lite Store listing. Nothing is downloaded,
installed or published.

## Problem

Lite users installed from Store have no in-app signal that a newer version is
waiting. The app never asks Store.

## Why

Requested directly. The audited plan is
`instruccion-actualizaciones-microsoft-store-entropia-lite.md` (repo root).

## Scope

- Rust: `apps/desktop/src-tauri/src/store_updates.rs` (new), `lib.rs`
  (register command/state, exact Store URI in `open_external_url`),
  `Cargo.toml`/`Cargo.lock` (direct `windows` dependency, Windows only).
- Frontend: `src/lib/store-updates.ts` (new), `src/App.svelte`,
  `src/layout/AppShell.svelte`, `src/lib/external-links.ts`, `src/lib/i18n.ts`,
  and their tests.

Out of scope: Store HTML, GitHub Releases, Tauri updater,
download/installation, mandatory updates, MSIX generation, publication.

## Constraints

- Only Windows Lite (no `local-ml`) queries Store. Pro, macOS and Linux return
  `skipped` without WinRT or cache access.
- Store family `CONICET.EntropIALite_b16na7gwepwme`, product
  `9N328K9L95JD`, URI `ms-windows-store://pdp/?ProductId=9N328K9L95JD`.
- One query per session, deduplicated; 6 h cache in
  `app_settings.microsoft_store_update_cache`, keyed by package full name.
- 20 s bound including main-thread dispatch; native cancellation on timeout.
- Plan correction found while verifying `windows 0.61.3` source:
  `IInitializeWithWindow` lives under feature `Win32_UI_Shell`, not
  `Win32_System_WinRT`.

## TDD

Strict TDD on (session config). Runners: Vitest (`pnpm --filter
@entropia-pro/desktop test -- <file>`), `cargo test --locked
--no-default-features store_updates` from `apps/desktop/src-tauri`.

## Tasks

- [x] T1 Backend: `store_updates.rs` core (cache, dedup, family filter,
      timeout) + native WinRT path + command/state registration + exact URI in
      `validate_external_url`. Route: inline (single cohesive module; see note).
- [x] T2 Frontend: wrapper, allowlisted URI, notice in `AppShell`, root-owned
      status/dismissal in `App`, i18n, tests. Route: inline.
- [x] T3 Verification: lint, typecheck, tests (Lite and Pro frontend), cargo
      fmt/check/test (Lite). Manual: visual check and real Store test pending
      the user.

Route note: kept inline; the plan already pins every file and the WinRT
signatures had to be verified against crate source directly.

## Acceptance

See plan §8. Store integration is only validated by a Store-distributed
install or an authorized Package Flight; not claimed here.

## Progress

- T1 done in `4f18c07`. RED observed for the Store URI in
  `validate_external_url`; the core tests were written with the module, not
  before it. Second plan correction: `IntoFuture` for WinRT operations needs
  `windows-future`'s `std` feature, which `windows` leaves off, so
  `windows-future = "0.2"` is a direct dependency (same resolved 0.2.1).
- T2 done (frontend commit). RED observed: 1 failing external-links test and 5
  failing App tests before the implementation. An existing source guard in
  `AppShell.test.ts` pins the `<main>` tag, so the focus hand-off sets a
  temporary `tabindex` on close instead of changing the tag.
- T3 evidence (2026-09-22):
  - `VITE_LOCAL_ML=0`: `pnpm lint` 0, `pnpm typecheck` 0, `pnpm test` 0
    (desktop 1899 passed, 26 skipped).
  - `VITE_LOCAL_ML=1`: lint 0, typecheck 0, test 0 (desktop 1918 passed,
    7 skipped). `pnpm format:check` 0.
  - `cargo fmt --check` 0. `cargo clippy --no-default-features --all-targets`
    no warnings.
  - `cargo check --locked --no-default-features` fails against the committed
    lock while the local `[patch]` in `.cargo/config.toml` is on (it strips the
    `entropia-agent` pin); passes (0) against the patch-rewritten lock.
  - `cargo test --locked --no-default-features --lib store_updates`: 16 passed.
    `... --lib validate_external_url`: 5 passed. `--lib` because the debug
    exe was running and could not be relinked; all these tests live in the lib.
- Not verified: Rust Pro (`local-ml`), macOS, Linux (CI); `tauri dev` Lite
  run without identity; visual check (themes, locales, keyboard, focus,
  narrow widths); real opening of the Store URI; real Store detection
  (needs a Store install or Package Flight).

## Next step

User: visual check of the notice (a temporary harness or a mocked status) and
a `tauri dev` Lite run with no identity. Then a Store-distributed test per
plan §8. Push stays the user's call.
