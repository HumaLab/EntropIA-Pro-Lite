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
- [ ] T2 — Verify: Lite Rust tests, and the half-initialized dev profile boots again under `tauri dev`.
