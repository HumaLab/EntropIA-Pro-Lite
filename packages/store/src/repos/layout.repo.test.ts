import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LayoutRepo } from './layout.repo'
import type { DrizzleClient } from '../types'

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

function createMockDrizzle() {
  const selectMock = createChainMock([])
  const deleteMock = createChainMock([])

  const db = {
    select: vi.fn().mockReturnValue(selectMock.proxy),
    delete: vi.fn().mockReturnValue(deleteMock.proxy),
  } as unknown as DrizzleClient

  return { db, deleteMock }
}

describe('LayoutRepo', () => {
  let db: ReturnType<typeof createMockDrizzle>
  let repo: LayoutRepo

  beforeEach(() => {
    db = createMockDrizzle()
    repo = new LayoutRepo(db.db)
  })

  it('returns null when asset has no stored layout', async () => {
    const selectResult = createChainMock([])
    ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

    await expect(repo.findByAsset('missing-asset')).resolves.toBeNull()
  })

  it('parses persisted layout JSON into frontend-friendly types', async () => {
    const row = {
      id: 'layout-1',
      assetId: 'asset-1',
      regions: JSON.stringify([
        {
          page: 2,
          image_width: 1200,
          image_height: 1800,
          group_id: 7,
          category: 'title',
          bbox: { x: 10, y: 20, width: 300, height: 80 },
          confidence: 0.98,
        },
      ]),
      blocks: JSON.stringify([
        {
          page: 2,
          image_width: 1200,
          image_height: 1800,
          label: 'plain_text',
          content: 'Hola mundo',
          bbox: { x: 15, y: 40, width: 320, height: 90 },
          order: 4,
          group_id: 7,
        },
      ]),
      model: 'paddle_vl',
      imageWidth: 1200,
      imageHeight: 1800,
      createdAt: 123456,
    }

    const selectResult = createChainMock([row])
    ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

    const result = await repo.findByAsset('asset-1')

    expect(result).toEqual({
      id: 'layout-1',
      assetId: 'asset-1',
      model: 'paddle_vl',
      imageWidth: 1200,
      imageHeight: 1800,
      createdAt: 123456,
      regions: [
        {
          page: 2,
          imageWidth: 1200,
          imageHeight: 1800,
          groupId: 7,
          category: 'title',
          bbox: { x: 10, y: 20, width: 300, height: 80 },
          confidence: 0.98,
        },
      ],
      blocks: [
        {
          page: 2,
          imageWidth: 1200,
          imageHeight: 1800,
          label: 'plain_text',
          content: 'Hola mundo',
          bbox: { x: 15, y: 40, width: 320, height: 90 },
          order: 4,
          groupId: 7,
        },
      ],
    })
  })

  it('throws a useful error when stored layout JSON is invalid', async () => {
    const selectResult = createChainMock([
      {
        id: 'layout-bad',
        assetId: 'asset-1',
        regions: '{invalid',
        blocks: '[]',
        model: 'paddle_vl',
        imageWidth: 1,
        imageHeight: 1,
        createdAt: 1,
      },
    ])
    ;(db.db.select as ReturnType<typeof vi.fn>).mockReturnValue(selectResult.proxy)

    await expect(repo.findByAsset('asset-1')).rejects.toThrow('Failed to parse layouts.regions')
  })

  it('invalidates the stored layout for an asset', async () => {
    await expect(repo.deleteByAssetId('asset-1')).resolves.toBeUndefined()

    expect(db.db.delete).toHaveBeenCalledTimes(1)
    expect(db.deleteMock.chain['where']).toHaveBeenCalledTimes(1)
  })
})
