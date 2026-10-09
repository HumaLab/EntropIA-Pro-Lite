# Layouts schema guard before the store bootstraps `assets`

Branch `fix/layouts-schema-before-assets`.

## Problem

`ensure_layouts_schema` (Rust setup, `apps/desktop/src-tauri/src/lib.rs`) runs before the store
migrations that the frontend applies. On a database where `assets` does not exist yet it creates
`layouts` with a foreign key to `assets`. If the frontend then fails to bootstrap (seen on Ubuntu
with a stale `node_modules`), the next launch runs `DELETE FROM layouts ...` with foreign keys on
and SQLite fails with `no such table: main.assets`; setup `.expect`s it and the app panics on every
start.

Store migration `0020_layouts.sql` already owns `CREATE TABLE IF NOT EXISTS layouts` and the same
dedupe/indexes, so the Rust repair only matters for legacy archives that already have `assets`.

## Tasks

- [x] T1 — Skip `ensure_layouts_schema` while `assets` does not exist (regression test first).
  Commit `70005f6f`. RED: 2 of 3 new tests failed with `no such table: main.assets`; GREEN after the guard.
- [x] T2 — Verify: Lite Rust tests, and the half-initialized dev profile boots again under `tauri dev`.
  Lite `cargo test --lib`: 2439 passed, 1 failed (the pre-existing Linux-only failure fixed in T3), 12 ignored;
  clippy `-D warnings` and fmt clean. Dev profile `lite-ubuntu`, which panicked before, now boots and the store
  applies all 58 migrations with an empty `foreign_key_check`.
- [x] T3 — Make `a_finished_folder_download_is_found_by_its_file_name_alone` portable: its
  `Z:\elsewhere\data.zip` path has no file name separator on Linux, so it only passed on Windows.
  Commit `a64ba4f2`; `navegador::download` 88/88 green on Linux.
