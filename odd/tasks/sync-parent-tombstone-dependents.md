# Sync: a parent tombstone must never block the whole pull

## Objective

A remote delete of a row that still has local dependents through a plain
`REFERENCES` (no cascade) foreign key must apply cleanly instead of rolling back
the page forever, and local maintenance at startup must never be pushed as a
user edit.

## Problem

Observed on a Pop!_OS notebook upgraded from Lite 1.0.5 to 1.0.16 (2026-09-26):
every pull cycle fails with `[sync] page commit failed (non-FK): FOREIGN KEY
constraint failed` and the cursor never moves (stuck at seq 21668 since July).

Cause chain, reproduced from the captured page and a copy of the archive:

1. At first start, the relative asset-path migration (`lib.rs`, `UPDATE assets
   SET path`) ran with the sync capture triggers active. 391 asset rows were
   queued and pushed (server seq 47677-48067). One of them belonged to an item
   another device had deleted in July, so the push resurrected that asset on the
   server and superseded its tombstone.
2. The pulled page carries the item tombstone. The local asset still references
   the item through `assets.item_id REFERENCES items(id)` (no cascade), so the
   deferred FK check fails at COMMIT.
3. `collect_fk_violators` only inspects tables touched by the page. The violating
   child (`assets`) is not in the page, nothing can be parked, the error is
   labelled "non-FK", and the page is rolled back and retried every minute.

Not platform-specific: step 3 is a general multi-device race (one device adds an
asset offline, another deletes the item).

## Decision (user, 2026-09-26)

Option (b): the remote delete wins. Local dependents reachable through non-cascade
edges are deleted, and each one is journaled as a `parent_deleted` conflict with
its full row as `loser_payload`, the same way a pulled child whose parent is
tombstoned is already handled. A locally dirty dependent still defers the
tombstone (existing skip-if-dirty rule).

## Scope

- In: `sync/apply.rs` tombstone path, `sync/cascade.rs` (or sibling) edge map for
  non-cascade edges, the startup asset-path rewrites in `lib.rs`, tests.
- Out: the Cloud server (it accepted a child upsert under a tombstoned parent);
  repairing server data already resurrected; UI for conflicts.

## Constraints

- TDD: enabled (session configuration). Runner: `cargo test --lib` from
  `apps/desktop/src-tauri`; also `cargo fmt --check` and
  `cargo clippy --lib --tests -- -D warnings`.
- Deletes during apply run with `applying='1'`, so they are not captured back.
- Keep the Lite and Pro command surface untouched.

## Tasks

- [x] T1 — Apply: a tombstone deletes local non-cascade dependents and journals
  each as `parent_deleted`; a dirty non-cascade dependent defers the tombstone.
  Route: delegated writer (apply.rs + cascade.rs + tests, 2+ non-trivial files).
- [x] T2 — Startup: the asset-path rewrites in `lib.rs` are not captured for
  sync. Route: inline (one file with its tests).
- [x] T3 — Remove the throwaway `[DEBUG-fk7r]` replay test; re-run the replay
  against the captured page as final proof; record how the notebook recovers.

## Acceptance criteria

- The captured notebook page applies against the copied archive: `Ok`, cursor at
  22360, one `parent_deleted` conflict for asset `d1a5d849`.
- Unit tests cover: dependent deleted + journaled; dirty dependent defers; a
  collection tombstone with a local item that has an asset; edge map matches the
  schema fixture.
- The asset-path migration leaves `sync_oplog` untouched.

## Evidence and progress

- Replay loop (removed after use): an ignored test replayed the captured page
  (500 rows since seq 21668) against a copy of the notebook archive. Red:
  `assets->items`, "non-FK". Minimal page: the single item tombstone `0643a46d`.
- T1 `c24e4e5e` (delegated writer, reviewed; comments trimmed of task-file
  references). RED: 4 new apply tests failed with the production error. GREEN:
  `cargo test --lib -- sync::` 156 passed; replay `Ok(PageOutcome { applied: 500,
  parked: 0, max_seq: 22360 })` with one `parent_deleted` conflict for asset
  `d1a5d849`; fmt and clippy clean.
- T2 `2e995ded` (inline). RED: both migrations left 1 oplog row. GREEN: 0 rows,
  `applying` restored; `cargo test --lib` 1059 passed; fmt and clippy clean.
- T3: replay test removed before the T1 commit.
- A Windows PC on the same account already shows 28 `parent_deleted` conflicts
  for assets of items it had deleted: the mirror case behaves as option (b).
- Checks not run locally: Pro (`local-ml`) build; CI covers it.

## Next step

Push, build a Lite `.deb` from main and install it on the notebook: its next
pull applies the stuck page (asset `d1a5d849` removed and journaled) and the
revived asset's later upsert (seq 47999) is skipped as already known. The 28
conflicts on the Windows PC can be acknowledged; they are the revived assets.
