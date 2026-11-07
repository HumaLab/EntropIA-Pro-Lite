import { describe, expect, it, vi } from 'vitest'
import { AssetProcessEventRepo } from './asset-process-event.repo'
import type { DrizzleClient } from '../types'

function dbReturning(rows: unknown[]) {
  const chain = {
    from: vi.fn().mockReturnThis(),
    where: vi.fn().mockReturnThis(),
    orderBy: vi.fn().mockResolvedValue(rows),
  }
  return { db: { select: vi.fn().mockReturnValue(chain) } as unknown as DrizzleClient, chain }
}

describe('AssetProcessEventRepo', () => {
  it('maps rows and parses params, tolerating malformed JSON', async () => {
    const { db } = dbReturning([
      {
        id: 'e1',
        assetId: 'a1',
        process: 'ocr',
        tool: 'GLM-OCR API (z.ai layout_parsing)',
        toolVersion: 'paas/v4',
        model: 'glm-ocr',
        modelVersion: null,
        appVersion: '1.4.0',
        paramsJson: '{"pageCount":2}',
        inputSha256: null,
        outputSha256: 'abc',
        createdAt: 10,
      },
      {
        id: 'e2',
        assetId: 'a1',
        process: 'text_edit',
        tool: 'EntropIA (edición manual)',
        toolVersion: null,
        model: null,
        modelVersion: null,
        appVersion: '1.4.0',
        paramsJson: 'not json',
        inputSha256: null,
        outputSha256: null,
        createdAt: 20,
      },
    ])

    const events = await new AssetProcessEventRepo(db).findByAsset('a1')

    expect(events[0]).toMatchObject({
      process: 'ocr',
      model: 'glm-ocr',
      params: { pageCount: 2 },
      outputSha256: 'abc',
    })
    expect(events[1]?.params).toBeNull()
  })
})
