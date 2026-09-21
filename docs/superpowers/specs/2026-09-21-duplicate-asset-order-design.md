# Duplicate Asset Order Design

## Goal

Duplicating an asset inserts and selects the copy immediately after its source. The persisted order remains identical after reopening the item, re-entering the collection, or restarting the application.

## Scope

Change only asset duplication ordering and the in-session insertion that reflects it. File copying, annotation behavior, OCR state, asset deletion, and non-duplication imports remain unchanged.

Affected units:

- `packages/store/src/repos/asset.repo.ts`: owns canonical persisted insertion after a source asset.
- `packages/store/src/repos/asset.repo.test.ts`: proves normalization, insertion, repetition, and reload ordering at the repository boundary.
- `apps/desktop/src/views/ItemView.svelte`: requests ordered insertion and places the returned asset at the same local index.
- `apps/desktop/src/views/ItemView.test.ts`: proves selection, counter, and previous/next navigation.

## Current Failure

`ItemView` currently assigns the copy `max(sortIndex) + 1`, appends it to the local array, and selects the last element. `AssetRepo.findByItem()` treats `sort_index` as authoritative once any asset has a non-zero value. The copy therefore appears last immediately and after every reload.

Legacy items may have every `sort_index` set to zero. Their current logical order is the repository's deterministic path-and-ID fallback, not their shared numeric value. Inserting relative to `sort_index = 0` without first normalizing that displayed order would be ambiguous and could create duplicate positions.

## Repository Contract

`AssetRepo.createAfter(sourceId, data)` owns ordered creation with this contract:

- Input: source asset ID plus the new asset fields.
- Output: the newly persisted asset with its final `sortIndex`.
- The source must exist and belong to the input item.
- The operation requires the raw database client's parameterized transaction facility.
- The copy's final position is exactly the source's normalized position plus one.
- Every asset in the item has a unique contiguous `sort_index` after success.
- Any failure rolls back both normalization and insertion.

The existing general `create` method remains unchanged for imports and other callers that are not relative insertions.

## Ordering Algorithm

The repository first reads the item's rows and a deterministic revision fingerprint in one SQLite statement. It applies the same `orderAssetsForDisplay` comparator already used by `findByItem()`, so accented, Unicode, and punctuation-heavy paths cannot change order merely because a copy was created.

One parameterized database transaction then:

1. Inserts the copy only when the source still exists and the revision still matches the snapshot.
2. Assigns every existing asset its final contiguous index, shifting rows after the source by one.

The revision covers every asset ID, path, and sort index. A concurrent insertion, deletion, rename, or reorder makes the guarded non-null insert fail and rolls back the transaction; `createAfter` then takes a fresh snapshot and retries. This preserves one canonical ordering rule while preventing stale snapshots from creating duplicate positions or returning a stale copy index.

Normalization includes hidden PDF container rows as well as visible leaf assets. This prevents duplicate database positions and preserves the relative order of every visible asset after `visibleAssets()` removes containers.

Consequences:

- First asset duplicated: copy becomes index `1`.
- Intermediate asset duplicated: following rows shift by one.
- Last asset duplicated: copy becomes the new last row.
- Same original duplicated repeatedly: the newest copy occupies `original + 1`; earlier copies shift right.
- A copy duplicated: the new row occupies `copy + 1`.

## View State

After the repository transaction succeeds, `ItemView` locates the source again in the current local array, inserts the returned asset immediately after it, and selects the copy. Recomputing the source position after the filesystem and database awaits prevents concurrent deletion events from making the local counter/navigation order diverge from persistence.

The existing paginator then produces the required behavior without special cases:

- the counter changes from `n / total` to `n + 1 / total + 1`;
- previous from the copy selects the source;
- next from the source selects the copy;
- next from the copy selects the asset that originally followed the source.

The existing copied-file cleanup remains in place: if database persistence fails, the newly copied filesystem entry is deleted and the source remains selected.

## Failure Handling

The ordered creation rejects a missing source, an item mismatch, or a database client without transactional DML support. A stale snapshot is retried from a fresh read; all other database failures surface to the existing item-view error state. The guarded transaction prevents a partially shifted order, a copy created from stale positions, or a returned asset with the wrong persisted index.

No timestamp, generated ID, filename, append position, or previous maximum index determines the copy's logical position.

## Verification

TDD covers these observable contracts:

1. Duplicating the first, intermediate, and last assets inserts each copy at source plus one.
2. Repeated duplication of the same original places the newest copy immediately after it.
3. Duplicating a copy inserts after that copy.
4. Reloading through `findByItem()` returns the persisted order.
5. All persisted positions are unique and contiguous.
6. Legacy all-zero positions are normalized from the existing path-and-ID display order.
7. Existing non-zero positions preserve their current displayed order before insertion.
8. The new copy is selected and the counter reports its one-based position and updated total.
9. Previous and next navigation traverse source, copy, and former successor in that order.
10. A stale ordering snapshot is rejected, retried, and returns the final persisted copy index.
11. Deleting an earlier local asset while duplication is pending still inserts and selects the copy after its source.
12. A failed database write removes the copied file and leaves the visible order unchanged.

Focused store and desktop Vitest files, the Lite frontend typecheck, and the Svelte autofixer provide completion evidence. Persistence is proven by closing the SQLite database, opening a new connection and repository, and re-reading the order through `AssetRepo.findByItem()`.
