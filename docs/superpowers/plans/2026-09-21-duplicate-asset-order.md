# Duplicate Asset Order Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist every duplicated asset immediately after its source and keep the new copy selected in that same logical order.

**Architecture:** Add one relative-insertion operation to `AssetRepo`. It normalizes the item's current canonical display order, shifts following rows, and inserts the copy through the existing parameterized transaction port; `ItemView` only mirrors the returned insertion at `selectedAssetIndex + 1`.

**Tech Stack:** TypeScript, Svelte 5 runes, parameterized SQLite transactions, Drizzle sqlite-proxy, Vitest, Testing Library, happy-dom.

## Global Constraints

- The copy's persisted position is exactly the source's normalized position plus one.
- The database, not the frontend array, is the ordering authority.
- Every asset row in the item has a unique contiguous `sort_index` after insertion, including hidden PDF container rows.
- Legacy all-zero rows preserve their current case-insensitive path-and-ID order before normalization.
- Existing non-zero rows preserve their current `sort_index`, case-insensitive path, and ID order before normalization.
- Repeated duplication puts the newest copy at `original + 1`; duplicating a copy puts the new row at `copy + 1`.
- The new copy remains selected; the paginator must traverse source → copy → former successor.
- File copying and copied-file cleanup behavior remain unchanged.
- Preserve the user's existing uncommitted changes in `ItemView.svelte` and `ItemView.test.ts`, especially the pending-navigation selection tests.
- Commit creation is deferred until all verification passes, per the user's instruction to commit the current workspace afterward.
- Every production change follows RED → GREEN with the focused command shown below.

---

### Task 1: Persist relative insertion in AssetRepo

**Files:**
- Modify: `packages/store/src/repos/asset.repo.test.ts`
- Modify: `packages/store/src/repos/asset.repo.ts`

**Interfaces:**
- Consumes: `DbClient.executeTransaction(statements)` and the canonical ordering already implemented by `AssetRepo.findByItem(itemId)`.
- Produces: `AssetRepo.createAfter(sourceId: string, data: Omit<NewAsset, 'id' | 'createdAt' | 'sortIndex'>): Promise<Asset>`.
- Preserves: `AssetRepo.create(...)` for imports and non-relative creation.

- [ ] **Step 1: Add a real-SQLite ordered repository fixture**

At the top of `asset.repo.test.ts`, add Node SQLite imports:

```ts
import { DatabaseSync, type SQLInputValue } from 'node:sqlite'
```

Inside the `AssetRepo` describe block, add a helper that uses the same `DbClient` transaction shape as Tauri:

```ts
function createOrderedRepo() {
  const sqlite = new DatabaseSync(':memory:')
  sqlite.exec(`
    PRAGMA foreign_keys = ON;
    CREATE TABLE items (id TEXT PRIMARY KEY);
    CREATE TABLE assets (
      id TEXT PRIMARY KEY,
      item_id TEXT NOT NULL REFERENCES items(id),
      path TEXT NOT NULL,
      type TEXT NOT NULL,
      sort_index INTEGER NOT NULL DEFAULT 0,
      size INTEGER,
      parent_asset_id TEXT,
      page_number INTEGER,
      created_at INTEGER NOT NULL
    );
    INSERT INTO items(id) VALUES ('item-1');
  `)

  const params = (values: unknown[]) => values as SQLInputValue[]
  const rawClient: DbClient = {
    async execute(sql, values = []) {
      const result = sqlite.prepare(sql).run(...params(values))
      return { rowsAffected: Number(result.changes) }
    },
    async executeBatch(sql) {
      sqlite.exec(sql)
    },
    async executeTransaction(statements) {
      sqlite.exec('BEGIN IMMEDIATE')
      try {
        for (const statement of statements) {
          sqlite.prepare(statement.sql).run(...params(statement.params ?? []))
        }
        sqlite.exec('COMMIT')
      } catch (error) {
        sqlite.exec('ROLLBACK')
        throw error
      }
    },
    async select<T>(sql: string, values: unknown[] = []) {
      return sqlite.prepare(sql).all(...params(values)) as T[]
    },
    async selectRows(sql, values = []) {
      return sqlite
        .prepare(sql)
        .all(...params(values))
        .map((row) => Object.values(row))
    },
  }

  return {
    sqlite,
    repo: new AssetRepo({} as DrizzleClient, rawClient),
  }
}
```

- [ ] **Step 2: Write the failing persisted-order scenarios**

Add a `describe('createAfter')` suite using the real SQLite helper. Seed legacy all-zero assets in path order traps, duplicate first/middle/last, then exercise repeated-original and copied-copy insertion. Re-read through `findByItem` after every operation and assert contiguous persisted indices rather than inspecting frontend state.

```ts
describe('createAfter', () => {
  function seed(sqlite: DatabaseSync) {
    const insert = sqlite.prepare(
      'INSERT INTO assets (id, item_id, path, type, sort_index, size, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)'
    )
    insert.run('a', 'item-1', '/A.png', 'image', 0, 10, 1)
    insert.run('b', 'item-1', '/B.png', 'image', 0, 20, 2)
    insert.run('c', 'item-1', '/C.png', 'image', 0, 30, 3)
    insert.run('d', 'item-1', '/D.png', 'image', 0, 40, 4)
  }

  async function copy(repo: AssetRepo, sourceId: string, path: string) {
    return repo.createAfter(sourceId, {
      itemId: 'item-1',
      path,
      type: 'image',
      size: 10,
    })
  }

  async function expectOrder(repo: AssetRepo, ids: string[]) {
    const rows = await repo.findByItem('item-1')
    expect(rows.map((asset) => asset.id)).toEqual(ids)
    expect(rows.map((asset) => asset.sortIndex)).toEqual(ids.map((_, index) => index))
  }

  it('persists first, middle and last copies at source plus one with contiguous positions', async () => {
    const { sqlite, repo } = createOrderedRepo()
    try {
      seed(sqlite)

      const middle = await copy(repo, 'b', '/B-copy.png')
      await expectOrder(repo, ['a', 'b', middle.id, 'c', 'd'])

      const first = await copy(repo, 'a', '/A-copy.png')
      await expectOrder(repo, ['a', first.id, 'b', middle.id, 'c', 'd'])

      const last = await copy(repo, 'd', '/D-copy.png')
      await expectOrder(repo, ['a', first.id, 'b', middle.id, 'c', 'd', last.id])
    } finally {
      sqlite.close()
    }
  })

  it('puts the newest repeated copy after the original and can duplicate that copy', async () => {
    const { sqlite, repo } = createOrderedRepo()
    try {
      seed(sqlite)
      const older = await copy(repo, 'b', '/B-copy-1.png')
      const newer = await copy(repo, 'b', '/B-copy-2.png')
      const copyOfCopy = await copy(repo, newer.id, '/B-copy-2-copy.png')

      await expectOrder(repo, ['a', 'b', newer.id, copyOfCopy.id, older.id, 'c', 'd'])
    } finally {
      sqlite.close()
    }
  })
})
```

Keep one additional case with deliberately duplicated non-zero positions so `findByItem`'s current `sortIndex → path → id` order is normalized without reordering visible rows:

```ts
it('normalizes an existing non-zero display order before insertion', async () => {
  const { sqlite, repo } = createOrderedRepo()
  try {
    sqlite.exec(`
      INSERT INTO assets VALUES
        ('late', 'item-1', '/Z.png', 'image', 4, NULL, NULL, NULL, 1),
        ('source', 'item-1', '/A.png', 'image', 2, NULL, NULL, NULL, 2),
        ('tie', 'item-1', '/B.png', 'image', 2, NULL, NULL, NULL, 3)
    `)
    const created = await copy(repo, 'source', '/source-copy.png')
    await expectOrder(repo, ['source', created.id, 'tie', 'late'])
  } finally {
    sqlite.close()
  }
})
```

- [ ] **Step 3: Verify RED**

Run:

```bash
pnpm --filter @entropia/store test -- src/repos/asset.repo.test.ts
```

Expected: FAIL because `AssetRepo.createAfter` does not exist.

- [ ] **Step 4: Implement `createAfter` with one parameterized write transaction**

In `asset.repo.ts`, add a named input type:

```ts
export type NewRelativeAsset = Omit<NewAsset, 'id' | 'createdAt' | 'sortIndex'>
```

Implement the method after `create` with these invariants:

1. Require `rawClient.executeTransaction`.
2. Read all item assets plus one `snapshot_revision` in a single SQL statement. Build the revision from each row's ID, path, and sort index in deterministic ID order.
3. Map the rows to `Asset` and pass them through the existing `orderAssetsForDisplay()` function. Do not recreate its comparator in SQL.
4. Reject a source not present in that ordered snapshot.
5. Set the new asset's returned `sortIndex` to the source snapshot index plus one.
6. Build parameterized per-row `UPDATE` statements assigning the final contiguous indices; every row after the source shifts by one.
7. Make the transaction's first statement a guarded `INSERT`. Its non-null `sort_index` expression returns the computed index only when the source still exists and the current database revision matches the snapshot. Otherwise it returns `NULL`, causing SQLite to reject and roll back the transaction before any position update.
8. Execute the guarded insert and all position updates in one transaction.
9. On the specific guarded `assets.sort_index` non-null conflict, take a fresh snapshot and retry up to three times. Propagate every unrelated database error unchanged.

This optimistic guard solves both concurrency boundaries without adding a second ordering rule: a serialized concurrent write cannot leave duplicate positions, and the returned asset index comes from the exact snapshot accepted by the transaction. The final scalar guard also rejects a source deleted between the read and write instead of returning a phantom asset.

- [ ] **Step 5: Verify GREEN**

Run the focused store command from Step 3.

Expected: every `asset.repo.test.ts` test passes. The new real-SQLite assertions prove reload order and unique contiguous positions.

- [ ] **Step 6: Run the store typecheck**

Run:

```bash
pnpm --filter @entropia/store typecheck
```

Expected: TypeScript exits 0.

### Task 2: Mirror persisted insertion and selection in ItemView

**Files:**
- Modify: `apps/desktop/src/views/ItemView.test.ts`
- Modify: `apps/desktop/src/views/ItemView.svelte`

**Interfaces:**
- Consumes: `store.assets.createAfter(sourceAsset.id, { itemId, path, type, size })`.
- Produces: local `assets` order with the returned row at `selectedAssetIndex + 1`, and selects that index.
- Preserves: existing duplicate-file cleanup and the user's in-flight parent/child navigation changes in both files.

- [ ] **Step 1: Extend the ItemView store mock without removing general creation**

In `StoreOptions.assetsRows`, add `sortIndex?: number`. In `createStore().assets`, retain `create` for unrelated asset creation and add:

```ts
createAfter: vi.fn().mockImplementation(async (_sourceId: string, data) => ({
  ...data,
  id: 'asset-duplicate-1',
  sortIndex: 1,
  createdAt: 2,
})),
```

- [ ] **Step 2: Rewrite the existing duplication test as the failing observable contract**

Replace the one-asset duplication test with three ordered assets. Duplicate the first and assert repository call, selected copy, `2 / 4`, and source/copy/successor traversal:

```ts
it('inserts and selects the duplicate after its source for counter and paginator navigation', async () => {
  storeRef.current = createStore({
    assetsRows: [
      {
        id: 'asset-source',
        itemId: 'item-1',
        path: 'docs/acta.pdf',
        type: 'pdf',
        sortIndex: 0,
        createdAt: 1,
        size: 2048,
      },
      {
        id: 'asset-next',
        itemId: 'item-1',
        path: 'docs/acta-2.pdf',
        type: 'pdf',
        sortIndex: 1,
        createdAt: 2,
        size: 1024,
      },
      {
        id: 'asset-last',
        itemId: 'item-1',
        path: 'docs/acta-3.pdf',
        type: 'pdf',
        sortIndex: 2,
        createdAt: 3,
        size: 512,
      },
    ],
  })
  duplicateAssetFileMock.mockResolvedValue({
    name: 'acta_c1.pdf',
    path: 'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf',
  })

  render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
  await fireEvent.click(
    await screen.findByRole('button', { name: /Duplicar asset|Duplicate asset/i })
  )

  await waitFor(() => {
    expect(storeRef.current.assets.createAfter).toHaveBeenCalledWith('asset-source', {
      itemId: 'item-1',
      path: 'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf',
      type: 'pdf',
      size: 2048,
    })
  })
  expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
    'data-path',
    'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf'
  )
  expect(await screen.findByText(/2\s*\/\s*4/)).toBeInTheDocument()

  await fireEvent.click(screen.getByRole('button', { name: /Página anterior|Previous page/i }))
  expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute('data-path', 'docs/acta.pdf')

  await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
  expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
    'data-path',
    'docs/11111111-1111-4111-8111-111111111111_acta_c1.pdf'
  )

  await fireEvent.click(screen.getByRole('button', { name: /Página siguiente|Next page/i }))
  expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute(
    'data-path',
    'docs/acta-2.pdf'
  )
})
```

Update the existing database-failure and pending-duplication tests to reject/expect `createAfter` rather than `create`. Their copied-file rollback assertions stay unchanged.

- [ ] **Step 3: Verify RED**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/ItemView.test.ts
```

Expected: FAIL because `handleDuplicateAsset` still calls `assets.create`, appends the result, and selects the last index.

- [ ] **Step 4: Implement the clean frontend cutover**

Replace only the persistence and local insertion inside `handleDuplicateAsset`:

```ts
const insertionIndex = selectedAssetIndex + 1
const createdAsset = await getStore().assets.createAfter(sourceAsset.id, {
  itemId: sourceAsset.itemId,
  path: duplicate.path,
  type: sourceAsset.type,
  size: sourceAsset.size,
})

assets = [...assets.slice(0, insertionIndex), createdAsset, ...assets.slice(insertionIndex)]
selectedAssetIndex = insertionIndex
lastHandledNavigationAssetId = null
```

Delete the obsolete `max(sortIndex) + 1` comment and calculation. Do not alter the event dispatch, error handling, copied-file cleanup, or surrounding edit locks.

- [ ] **Step 5: Verify GREEN**

Run the focused desktop command from Step 3.

Expected: all `ItemView.test.ts` tests pass, including the user's pre-existing pending-navigation cases and the duplication rollback/pending cases.

- [ ] **Step 6: Run Svelte analysis**

Run:

```bash
npx @sveltejs/mcp svelte-autofixer apps/desktop/src/views/ItemView.svelte --svelte-version 5
```

Expected: no new issue caused by `handleDuplicateAsset`. Existing broad `$effect` suggestions in this large component are out of scope.

### Task 3: Focused verification, runtime smoke, and cleanup

**Files:**
- Modify only if a verification failure identifies a defect in the changed path.
- Keep: `docs/superpowers/specs/2026-09-21-duplicate-asset-order-design.md`.
- Keep: `docs/superpowers/plans/2026-09-21-duplicate-asset-order.md`.

**Interfaces:** None.

- [ ] **Step 1: Run the complete focused behavior set**

```bash
pnpm --filter @entropia/store test -- src/repos/asset.repo.test.ts
pnpm --filter @entropia-pro/desktop test -- src/views/ItemView.test.ts
```

Expected: both Vitest commands pass with no unhandled errors. The store scenario covers first, middle, last, repeated-original, copied-copy, persisted reload, and contiguous positions. The desktop scenario covers selected copy, counter, and `< >` traversal.

- [ ] **Step 2: Run focused static verification**

```bash
pnpm --filter @entropia/store typecheck
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: both commands exit 0; Svelte Check reports zero errors.

- [ ] **Step 3: Smoke the repository against an actual SQLite database**

Use the real-SQLite Vitest scenarios as the runtime boundary: they execute the guarded insert, final position updates, stale-snapshot retry, and subsequent `findByItem()` reload against SQLite rather than mocks. The disk-backed scenario closes the database, opens a new connection and repository, and proves restart persistence. A full Tauri build is intentionally excluded because this change does not modify Rust and the repo rules prohibit casual Pro builds.

- [ ] **Step 4: Clean up**

Remove no tests: each new assertion protects a requested persisted or navigation contract. Remove no temporary scripts because none are created. Confirm there is no old `max(sortIndex) + 1` duplication path and no duplication call to general `assets.create`.

- [ ] **Step 5: Commit after all checks pass**

Per the user's instruction, inspect the complete workspace only after verification and commit afterward. Do not discard or overwrite pre-existing changes. Keep the asset-order fix, its tests, spec, and plan together in the reviewable commit; if the user still wants every unrelated current diff in the same commit, follow that explicit instruction at execution time.
