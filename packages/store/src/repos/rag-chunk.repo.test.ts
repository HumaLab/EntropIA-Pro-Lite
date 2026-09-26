import { describe, it, expect, vi, beforeEach } from 'vitest'
import { RagChunkRepo } from './rag-chunk.repo'
import type { DrizzleClient } from '../types'

// Helper: create a chainable mock that resolves with the given value
// (same shape as note.repo.test.ts's mock — a Proxy that resolves any
// chained call and is awaitable itself).
function createChainMock(resolveValue: unknown = []) {
  const chain: Record<string, ReturnType<typeof vi.fn>> = {}

  const createProxy = (): unknown =>
    new Proxy(() => {}, {
      apply: () => (resolveValue instanceof Promise ? resolveValue : Promise.resolve(resolveValue)),
      get: (_target, prop) => {
        if (prop === 'then') {
          return (resolve: (v: unknown) => void) => resolve(resolveValue)
        }
        if (!chain[prop as string]) {
          chain[prop as string] = vi.fn().mockReturnValue(createProxy())
        }
        return chain[prop as string]
      },
    })

  return { proxy: createProxy(), chain }
}

function createMockDrizzle(resolveValue: unknown[] = []) {
  const selectMock = createChainMock(resolveValue)

  const db = {
    select: vi.fn().mockReturnValue(selectMock.proxy),
  } as unknown as DrizzleClient

  return { db, mocks: { select: selectMock } }
}

describe('RagChunkRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: RagChunkRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new RagChunkRepo(db.db)
  })

  describe('findById', () => {
    // A cited chunk resolves to exactly one asset: the caller (the
    // Investigation source panel) needs assetId/itemId to pick the single
    // path that matches, instead of listing every file of the item.
    it('returns the chunk row with its asset and item ids', async () => {
      db = createMockDrizzle([{ id: 'ragchk-84f46f', assetId: 'asset-page-2', itemId: 'item-2' }])
      repo = new RagChunkRepo(db.db)

      const result = await repo.findById('ragchk-84f46f')

      expect(result).toEqual({ id: 'ragchk-84f46f', assetId: 'asset-page-2', itemId: 'item-2' })
    })

    // A re-indexed archive can drop the old chunk row while the citation
    // still names it: the caller falls back rather than crashing.
    it('returns null when the chunk no longer exists', async () => {
      db = createMockDrizzle([])
      repo = new RagChunkRepo(db.db)

      const result = await repo.findById('ragchk-gone')

      expect(result).toBeNull()
    })
  })
})
