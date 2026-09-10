import { describe, expect, it, vi } from 'vitest'
import {
  belongsToLayoutPage,
  buildLayoutBlockViews,
  categoriesMatch,
  countLayoutBlocksByFilter,
  filterBlocksByPage,
  filterRegionsByPage,
  filterLayoutBlocksByType,
  filterLayoutBlocksByPage,
  findLayoutBlockById,
  findLayoutBlockByRegionId,
  getBlockCountByPage,
  getLayoutInteractionStateFromBlockId,
  getLayoutInteractionStateFromRegionId,
  getLayoutBlockFilterId,
  getIntersectionArea,
  getPagesFromLayout,
  getLayoutByAsset,
  getOverlapRatio,
  matchLayoutRegionToBlock,
  normalizeLayoutBBox,
  pruneLayoutInteractionSelectionState,
} from './layouts'
import type { LayoutBlockView } from './layouts'
import type { LayoutBlock, LayoutRegion } from '@entropia/store'

const findByAsset = vi.fn()

vi.mock('./db', () => ({
  getStore: () => ({
    layouts: {
      findByAsset,
    },
  }),
}))

function layoutBlockView(overrides: Partial<LayoutBlockView> = {}): LayoutBlockView {
  return {
    id: 'layout-block-1',
    regionId: 'layout-block-1::overlay',
    label: 'text',
    order: 1,
    content: 'content',
    preview: 'content',
    bbox: { x: 0, y: 0, width: 10, height: 10 },
    overlayBbox: { x: 0, y: 0, width: 10, height: 10 },
    overlaySource: 'block',
    page: 1,
    groupId: 1,
    imageWidth: 100,
    imageHeight: 100,
    ...overrides,
  }
}

describe('getLayoutByAsset', () => {
  it('delegates layout lookup to the store repo', async () => {
    const layout = {
      id: 'layout-1',
      assetId: 'asset-1',
      model: 'paddle_vl',
      imageWidth: 1000,
      imageHeight: 1400,
      createdAt: 1,
      regions: [],
      blocks: [],
    }
    findByAsset.mockResolvedValueOnce(layout)

    await expect(getLayoutByAsset('asset-1')).resolves.toEqual(layout)
    expect(findByAsset).toHaveBeenCalledWith('asset-1')
  })
})

describe('buildLayoutBlockViews', () => {
  it('normalizes block previews and sorts blocks by page/order', () => {
    const layout = {
      id: 'layout-1',
      assetId: 'asset-1',
      model: 'paddle_vl',
      imageWidth: 1200,
      imageHeight: 1800,
      createdAt: 1,
      regions: [
        {
          category: 'doc_title',
          groupId: 7,
          confidence: 0.98,
          bbox: { x: 2, y: 3, width: 90, height: 50 },
          page: 1,
        },
        {
          category: 'text',
          confidence: 0.97,
          bbox: { x: 8, y: 18, width: 32, height: 44 },
          page: 2,
        },
      ],
      blocks: [
        {
          label: 'text',
          content: '  Segundo   bloque con   espacios  ',
          bbox: { x: 10, y: 20, width: 30, height: 40 },
          order: 2,
          groupId: 9,
          page: 2,
        },
        {
          label: 'title',
          content: 'A'.repeat(140),
          bbox: { x: 1, y: 2, width: 3, height: 4 },
          order: 1,
          groupId: 7,
          page: 1,
          imageWidth: 900,
          imageHeight: 1300,
        },
      ],
    }

    const blocks = buildLayoutBlockViews(layout)

    expect(blocks).toHaveLength(2)
    expect(blocks[0]).toMatchObject({
      id: 'layout-block-1',
      regionId: 'layout-block-1::overlay',
      label: 'title',
      order: 1,
      content: 'A'.repeat(140),
      page: 1,
      imageWidth: 900,
      imageHeight: 1300,
      overlayBbox: { x: 1, y: 2, width: 3, height: 4 },
      overlaySource: 'block',
    })
    expect(blocks[0]?.preview).toHaveLength(120)
    expect(blocks[0]?.preview.endsWith('…')).toBe(true)

    expect(blocks[1]).toMatchObject({
      id: 'layout-block-0',
      regionId: 'layout-block-0::overlay',
      label: 'text',
      order: 2,
      content: '  Segundo   bloque con   espacios  ',
      page: 2,
      imageWidth: 1200,
      imageHeight: 1800,
      preview: 'Segundo bloque con espacios',
      overlayBbox: { x: 10, y: 20, width: 30, height: 40 },
      overlaySource: 'block',
    })
  })

  it('uses the block bbox for the overlay even when a region matches', () => {
    const layout = {
      id: 'layout-2',
      assetId: 'asset-2',
      model: 'paddle_vl',
      imageWidth: 1000,
      imageHeight: 1500,
      createdAt: 1,
      regions: [
        {
          category: 'table',
          confidence: 0.94,
          bbox: { x: 500, y: 500, width: 100, height: 100 },
          page: 1,
        },
      ],
      blocks: [
        {
          label: 'plain_text',
          content: 'bloque sin region',
          bbox: { x: 10, y: 20, width: 100, height: 30 },
          order: 1,
          groupId: 0,
          page: 1,
        },
      ],
    }

    expect(buildLayoutBlockViews(layout)).toEqual([
      expect.objectContaining({
        content: 'bloque sin region',
        overlayBbox: { x: 10, y: 20, width: 100, height: 30 },
        overlaySource: 'block',
      }),
    ])
  })
})

describe('filterLayoutBlocksByPage', () => {
  it('returns only blocks for the requested page', () => {
    const blocks: LayoutBlockView[] = [
      {
        id: 'layout-block-0',
        regionId: 'layout-block-0::overlay',
        label: 'title',
        order: 1,
        content: 'uno',
        preview: 'uno',
        bbox: { x: 0, y: 0, width: 10, height: 10 },
        overlayBbox: { x: 0, y: 0, width: 10, height: 10 },
        overlaySource: 'block',
        page: 1,
        groupId: 1,
        imageWidth: 100,
        imageHeight: 200,
      },
      {
        id: 'layout-block-1',
        regionId: 'layout-block-1::overlay',
        label: 'text',
        order: 2,
        content: 'dos',
        preview: 'dos',
        bbox: { x: 10, y: 10, width: 20, height: 20 },
        overlayBbox: { x: 10, y: 10, width: 20, height: 20 },
        overlaySource: 'block',
        page: 2,
        groupId: 2,
        imageWidth: 100,
        imageHeight: 200,
      },
    ]

    expect(filterLayoutBlocksByPage(blocks, 2)).toEqual([blocks[1]])
  })
})

describe('layout interaction helpers', () => {
  const blocks = [
    layoutBlockView({ id: 'block-1', regionId: 'region-1' }),
    layoutBlockView({ id: 'block-2', regionId: 'region-2' }),
  ]

  it('finds visible layout blocks by block id or region id', () => {
    expect(findLayoutBlockById(blocks, 'block-1')).toBe(blocks[0])
    expect(findLayoutBlockByRegionId(blocks, 'region-2')).toBe(blocks[1])
    expect(findLayoutBlockById(blocks, null)).toBeNull()
    expect(findLayoutBlockByRegionId(blocks, 'missing')).toBeNull()
  })

  it('builds interaction state from a block id', () => {
    expect(getLayoutInteractionStateFromBlockId(blocks, 'block-1')).toEqual({
      blockId: 'block-1',
      regionId: 'region-1',
      hasMatch: true,
    })
    expect(getLayoutInteractionStateFromBlockId(blocks, 'missing')).toEqual({
      blockId: null,
      regionId: null,
      hasMatch: false,
    })
  })

  it('builds interaction state from a region id', () => {
    expect(getLayoutInteractionStateFromRegionId(blocks, 'region-2')).toEqual({
      blockId: 'block-2',
      regionId: 'region-2',
      hasMatch: true,
    })
  })

  it('prunes selected and hovered layout ids that are no longer visible', () => {
    expect(
      pruneLayoutInteractionSelectionState(blocks, {
        selectedBlockId: 'block-1',
        selectedRegionId: 'region-1',
        hoveredBlockId: 'missing',
        hoveredRegionId: 'missing-region',
      })
    ).toEqual({
      selectedBlockId: 'block-1',
      selectedRegionId: 'region-1',
      hoveredBlockId: null,
      hoveredRegionId: null,
    })
  })
})

describe('multi-page layout helpers', () => {
  it('filters persisted blocks and regions by normalized page number', () => {
    const blocks: LayoutBlock[] = [
      {
        label: 'title',
        content: 'missing page should stay on page 1',
        bbox: { x: 0, y: 0, width: 10, height: 10 },
        order: 1,
        groupId: 1,
      },
      {
        label: 'text',
        content: 'page two',
        bbox: { x: 10, y: 10, width: 10, height: 10 },
        order: 2,
        groupId: 2,
        page: 2,
      },
      {
        label: 'table',
        content: 'page zero falls back to page 1',
        bbox: { x: 20, y: 20, width: 10, height: 10 },
        order: 3,
        groupId: 3,
        page: 0,
      },
    ]
    const regions: LayoutRegion[] = [
      {
        category: 'doc_title',
        confidence: 0.9,
        bbox: { x: 0, y: 0, width: 10, height: 10 },
      },
      {
        category: 'text',
        confidence: 0.8,
        bbox: { x: 10, y: 10, width: 10, height: 10 },
        page: 2,
      },
      {
        category: 'table',
        confidence: 0.7,
        bbox: { x: 20, y: 20, width: 10, height: 10 },
        page: -3,
      },
    ]

    expect(filterBlocksByPage(blocks, 1)).toEqual([blocks[0], blocks[2]])
    expect(filterBlocksByPage(blocks, 2)).toEqual([blocks[1]])
    expect(filterRegionsByPage(regions, 1)).toEqual([regions[0], regions[2]])
    expect(filterRegionsByPage(regions, 2)).toEqual([regions[1]])
  })

  it('derives sorted unique pages from blocks and regions', () => {
    const layout = {
      id: 'layout-pages',
      assetId: 'asset-pages',
      model: 'paddle_vl',
      imageWidth: 1000,
      imageHeight: 1500,
      createdAt: 1,
      regions: [
        {
          category: 'text',
          confidence: 0.8,
          bbox: { x: 0, y: 0, width: 10, height: 10 },
          page: 3,
        },
      ],
      blocks: [
        {
          label: 'title',
          content: 'missing page becomes page 1',
          bbox: { x: 0, y: 0, width: 10, height: 10 },
          order: 1,
          groupId: 1,
        },
        {
          label: 'text',
          content: 'page three',
          bbox: { x: 10, y: 10, width: 10, height: 10 },
          order: 2,
          groupId: 2,
          page: 3,
        },
        {
          label: 'figure',
          content: 'page two',
          bbox: { x: 20, y: 20, width: 10, height: 10 },
          order: 3,
          groupId: 3,
          page: 2,
        },
      ],
    }

    expect(getPagesFromLayout(layout)).toEqual([1, 2, 3])
    expect(getPagesFromLayout({ ...layout, blocks: [], regions: [] })).toEqual([])
  })

  it('counts blocks by normalized page and preserves sparse pages', () => {
    const blocks: LayoutBlock[] = [
      {
        label: 'title',
        content: 'first',
        bbox: { x: 0, y: 0, width: 10, height: 10 },
        order: 1,
        groupId: 1,
      },
      {
        label: 'text',
        content: 'second',
        bbox: { x: 10, y: 10, width: 10, height: 10 },
        order: 2,
        groupId: 2,
        page: 2,
      },
      {
        label: 'table',
        content: 'third',
        bbox: { x: 20, y: 20, width: 10, height: 10 },
        order: 3,
        groupId: 3,
        page: 2,
      },
      {
        label: 'figure',
        content: 'fourth',
        bbox: { x: 30, y: 30, width: 10, height: 10 },
        order: 4,
        groupId: 4,
        page: 5,
      },
    ]

    expect(getBlockCountByPage(blocks)).toEqual({
      1: 1,
      2: 2,
      5: 1,
    })
  })
})

describe('layout type filters', () => {
  const blocks: LayoutBlockView[] = [
    {
      id: 'layout-block-0',
      regionId: 'layout-block-0::overlay',
      label: 'title',
      order: 1,
      content: 'uno',
      preview: 'uno',
      bbox: { x: 0, y: 0, width: 10, height: 10 },
      overlayBbox: { x: 0, y: 0, width: 10, height: 10 },
      overlaySource: 'block',
      page: 1,
      groupId: 1,
      imageWidth: 100,
      imageHeight: 200,
    },
    {
      id: 'layout-block-1',
      regionId: 'layout-block-1::overlay',
      label: 'plain_text',
      order: 2,
      content: 'dos',
      preview: 'dos',
      bbox: { x: 10, y: 10, width: 20, height: 20 },
      overlayBbox: { x: 10, y: 10, width: 20, height: 20 },
      overlaySource: 'block',
      page: 1,
      groupId: 2,
      imageWidth: 100,
      imageHeight: 200,
    },
    {
      id: 'layout-block-2',
      regionId: 'layout-block-2::overlay',
      label: 'table',
      order: 3,
      content: 'tres',
      preview: 'tres',
      bbox: { x: 15, y: 15, width: 20, height: 20 },
      overlayBbox: { x: 15, y: 15, width: 20, height: 20 },
      overlaySource: 'block',
      page: 1,
      groupId: 3,
      imageWidth: 100,
      imageHeight: 200,
    },
    {
      id: 'layout-block-3',
      regionId: 'layout-block-3::overlay',
      label: 'image',
      order: 4,
      content: 'cuatro',
      preview: 'cuatro',
      bbox: { x: 20, y: 20, width: 20, height: 20 },
      overlayBbox: { x: 20, y: 20, width: 20, height: 20 },
      overlaySource: 'block',
      page: 1,
      groupId: 4,
      imageWidth: 100,
      imageHeight: 200,
    },
    {
      id: 'layout-block-4',
      regionId: 'layout-block-4::overlay',
      label: 'vision_footnote',
      order: 5,
      content: 'cinco',
      preview: 'cinco',
      bbox: { x: 25, y: 25, width: 20, height: 20 },
      overlayBbox: { x: 25, y: 25, width: 20, height: 20 },
      overlaySource: 'block',
      page: 1,
      groupId: 5,
      imageWidth: 100,
      imageHeight: 200,
    },
  ]

  it('maps labels into premium filter buckets', () => {
    expect(getLayoutBlockFilterId('paragraph_title')).toBe('titles')
    expect(getLayoutBlockFilterId('page_header')).toBe('text')
    expect(getLayoutBlockFilterId('table_caption')).toBe('tables')
    expect(getLayoutBlockFilterId('figure_title')).toBe('figures')
    expect(getLayoutBlockFilterId('table_note')).toBe('notes')
    expect(getLayoutBlockFilterId('formula')).toBeNull()
  })

  it('filters blocks by selected type and computes counters', () => {
    expect(filterLayoutBlocksByType(blocks, 'all')).toHaveLength(5)
    expect(filterLayoutBlocksByType(blocks, 'figures')).toEqual([blocks[3]])
    expect(filterLayoutBlocksByType(blocks, 'notes')).toEqual([blocks[4]])
    expect(countLayoutBlocksByFilter(blocks)).toEqual({
      all: 5,
      titles: 1,
      text: 1,
      tables: 1,
      figures: 1,
      notes: 1,
    })
  })
})

describe('layout geometry helpers', () => {
  it('normalizes bbox values and computes intersections', () => {
    expect(normalizeLayoutBBox({ x: 5, y: 6, width: -10, height: 8 })).toEqual({
      x: 5,
      y: 6,
      width: 0,
      height: 8,
      right: 5,
      bottom: 14,
      area: 0,
    })

    expect(
      getIntersectionArea(
        { x: 0, y: 0, width: 10, height: 10 },
        { x: 5, y: 5, width: 10, height: 4 }
      )
    ).toBe(20)
    expect(
      getOverlapRatio({ x: 0, y: 0, width: 10, height: 10 }, { x: 5, y: 5, width: 10, height: 10 })
    ).toBeCloseTo(0.25)
  })

  it('checks page membership and category aliases', () => {
    expect(belongsToLayoutPage({ page: 2 }, 2)).toBe(true)
    expect(belongsToLayoutPage({}, 1)).toBe(true)
    expect(belongsToLayoutPage({ page: 0 }, 1)).toBe(true)
    expect(belongsToLayoutPage({ page: 3 }, 2)).toBe(false)
    expect(categoriesMatch('title', 'doc_title')).toBe(true)
    expect(categoriesMatch('plain_text', 'text')).toBe(true)
    expect(categoriesMatch('table', 'figure')).toBe(false)
  })
})

describe('matchLayoutRegionToBlock', () => {
  it('prefers groupId matches when available', () => {
    const block = {
      label: 'title',
      content: 'Titulo',
      bbox: { x: 10, y: 10, width: 100, height: 30 },
      order: 1,
      groupId: 9,
      page: 1,
    }

    const match = matchLayoutRegionToBlock(block, [
      {
        category: 'text',
        confidence: 0.7,
        bbox: { x: 10, y: 10, width: 100, height: 30 },
        groupId: 3,
        page: 1,
      },
      {
        category: 'doc_title',
        confidence: 0.99,
        bbox: { x: 11, y: 9, width: 102, height: 32 },
        groupId: 9,
        page: 1,
      },
    ])

    expect(match).toMatchObject({
      source: 'groupId',
      region: expect.objectContaining({ groupId: 9 }),
    })
  })

  it('matches by overlap when group/category are not enough', () => {
    const block = {
      label: 'unknown',
      content: 'texto',
      bbox: { x: 100, y: 100, width: 80, height: 40 },
      order: 1,
      groupId: 0,
      page: 1,
    }

    const match = matchLayoutRegionToBlock(block, [
      {
        category: 'table',
        confidence: 0.9,
        bbox: { x: 400, y: 400, width: 50, height: 50 },
        page: 1,
      },
      {
        category: 'figure',
        confidence: 0.95,
        bbox: { x: 95, y: 95, width: 90, height: 50 },
        page: 1,
      },
    ])

    expect(match).toMatchObject({
      source: 'overlap',
      region: expect.objectContaining({ category: 'figure' }),
    })
  })

  it('returns null for controlled mismatches', () => {
    const block = {
      label: 'title',
      content: 'texto',
      bbox: { x: 10, y: 10, width: 40, height: 20 },
      order: 1,
      groupId: 0,
      page: 1,
    }

    const match = matchLayoutRegionToBlock(block, [
      {
        category: 'table',
        confidence: 0.99,
        bbox: { x: 500, y: 500, width: 200, height: 100 },
        page: 1,
      },
      {
        category: 'doc_title',
        confidence: 0.99,
        bbox: { x: 800, y: 200, width: 100, height: 40 },
        page: 2,
      },
    ])

    expect(match).toBeNull()
  })
})
