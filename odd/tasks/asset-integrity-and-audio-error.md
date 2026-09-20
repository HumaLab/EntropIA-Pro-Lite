# Asset integrity check and a truthful audio error

## Objective

Make a missing asset file visible instead of silent, and stop telling the user
to convert a file format when the file simply is not there.

## Problem

Preparing the 1.0.13 release, a scan of the real archive found 1 asset row out
of 4133 whose file is absent from disk (an 8.7 MB `.wav`). Nothing in the app
noticed: there is no reconciliation between `assets` rows and files on disk
anywhere in the migration or the startup path. The gap had been sealed since
2026-09-09 by the one-time migration marker (`lib.rs:1059`), which makes the
whole legacy convergence a no-op on every later launch.

Opening that item shows: *"No se pudo reproducir el audio. Probá abrir el
archivo original o convertirlo a un formato compatible. Detalle: HTTP 404."*
The format has nothing to do with it. `fetchFallbackBlob`
(`AudioPlayer.svelte:196`) throws `HTTP 404` when the asset protocol reports no
such file, and `failFallbackLoad` renders the generic format message.

## Scope

Authorized:
- `apps/desktop/src-tauri/src/**` — a command that reconciles `assets` rows
  against files on disk, plus a startup scan that logs the result.
- `packages/ui/src/components/AudioPlayer/**` — tell a missing file apart from
  a format/decode failure and word each case truthfully.

Not authorized: changing the migration itself, adding a repair/re-migrate
command, or new settings UI. Those are follow-ups.

## Constraints

- TDD is enabled: a failing test first, then the implementation.
- Test runners: `pnpm --filter @entropia/ui test` and, from
  `apps/desktop/src-tauri`, `cargo test`. **EntropIA must be closed** or cargo
  cannot replace the debug exe.
- Any local cargo run strips the `entropia-agent` git pin from `Cargo.lock`;
  restore it with `git checkout -- apps/desktop/src-tauri/Cargo.lock`.
- Prettier is gated in CI in a step separate from lint: run
  `pnpm format:check` before claiming done.
- User-facing strings in this app are Spanish; keep the existing register.

## Tasks

- [x] T1 — Rust: reconciliation command with tests (missing file, absent row,
      empty archive, path outside the archive).
- [x] T2 — Rust: startup scan that logs one line, without blocking startup.
- [x] T3 — UI: AudioPlayer distinguishes a missing file from a format failure.
- [x] T4 — Full check: ui + store + desktop tests, cargo test, format:check.

## Acceptance

A missing asset file produces a visible, correctly worded signal in both
places: a logged count at startup and an honest message in the player.

## Progress

TDD followed throughout: every new function was tested RED (stubbed to fail)
before being implemented GREEN, observed by running the actual test command.

- **T1** — `apps/desktop/src-tauri/src/asset_integrity.rs` (new module,
  registered in `lib.rs`). `reconcile_assets(conn, data_dir, sample_cap)`
  walks `assets`, resolving each `path` the way the rest of the app does
  (`path_utils::resolve_asset_path`), and reports `checked`/`missing`/a
  capped `missing_sample` (`MISSING_SAMPLE_CAP = 20`). A relative key that
  resolves outside `data_dir` (via `path_utils::ensure_within_dir`) is
  treated as missing even if a file genuinely exists at the escaped
  location — an absolute stored path is trusted as-is, matching how
  `resolve_asset_path` already treats it (an external file never copied
  in). 4 unit tests in the module: file exists, file missing (and
  sampled), empty archive (zero rows), and the escape case. Exposed as the
  Tauri command `asset_integrity::assets_check_integrity` (own DB
  connection via `spawn_blocking`, registered in `invoke_handler!`).
- **T2** — `asset_integrity::spawn_startup_scan(db_path, data_dir)`: a
  plain `std::thread::spawn` (not the async runtime) that opens its own
  connection, runs the same `reconcile_assets`, and logs one
  `[setup] asset integrity: checked N, missing M` line via
  `format_integrity_log_line` (TDD'd separately — RED/GREEN on the pure
  formatter). Never blocks or can fail `setup()`; wired in right after
  `app.manage(AppDbState::new(...))` in `lib.rs`.
- **T3** — `packages/ui/src/components/AudioPlayer/AudioPlayer.svelte`.
  `fetchFallbackBlob` now throws a distinct `AssetNotFoundError` on
  HTTP 404/410 (the asset protocol's "no such file" signal) instead of a
  bare `Error`. `failFallbackLoad` sets a new `fallbackMissing` state from
  `error instanceof AssetNotFoundError`. The template branches on it: a
  missing file gets "No se pudo reproducir el audio: el archivo no está
  donde la aplicación lo tiene registrado. Puede haberse movido o
  eliminado por fuera de la app." (no format-conversion suggestion, no raw
  `HTTP 404` shown); everything else keeps the original format/convert
  message and its `Detalle: …` line. The raw diagnostic is still
  `console.error`'d in both cases. Two new tests in
  `AudioPlayer.test.ts`: 404 → truthful missing-file message; 500 →
  unchanged format-failure message with its `Detalle: HTTP 500`.
- **T4** — see Verification below; all green. `Cargo.lock` restored with
  `git checkout` after every cargo run; `git status` confirms it is clean.

### Verification (all observed, not assumed)

- `pnpm --filter @entropia/ui test`: 50 files, 701 passed.
- `pnpm --filter @entropia-pro/desktop test`: 144 files, 1827 passed, 7
  skipped (pre-existing, unrelated).
- `pnpm --filter @entropia/store test`: 18 files, 276 passed.
- `pnpm lint`: clean (3 packages).
- `pnpm typecheck`: clean, 0 errors/warnings in both `svelte-check` runs.
- `pnpm format:check`: initially flagged `AudioPlayer.svelte` (my edit's
  own formatting drift); fixed with `prettier --write` on that one file,
  then AudioPlayer tests re-run and reconfirmed green; final
  `format:check` clean.
- `cargo test` (src-tauri): 1013 passed, 0 failed, 4 ignored
  (pre-existing); the two other integration suites pass or skip as
  before (sync_e2e needs a live server, expected).
- `cargo clippy --all-targets`: clean, no warnings.

Scope respected: no changes to the migration itself, no repair/re-migrate
command, no new settings UI.
