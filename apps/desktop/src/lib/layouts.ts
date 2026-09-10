import type { AssetLayout, LayoutBlock, LayoutBoundingBox, LayoutRegion } from '@entropia/store'
import { getStore } from './db'

export type { AssetLayout } from '@entropia/store'

const BLOCK_PREVIEW_MAX = 120

const EPSILON = 0.0001

export type LayoutBlockFilterId = 'all' | 'titles' | 'text' | 'tables' | 'figures' | 'notes'

export interface LayoutBlockFilterOption {
  id: LayoutBlockFilterId
  label: string
}

export type LayoutBlockFilterCounts = Record<LayoutBlockFilterId, number>

export const LAYOUT_BLOCK_FILTERS: LayoutBlockFilterOption[] = [
  { id: 'all', label: 'Todos' },
  { id: 'titles', label: 'Títulos' },
  { id: 'text', label: 'Texto' },
  { id: 'tables', label: 'Tablas' },
  { id: 'figures', label: 'Figuras' },
  { id: 'notes', label: 'Notas' },
]

const CATEGORY_ALIASES: Record<string, Set<string>> = {
  title: new Set(['title', 'doc_title', 'paragraph_title']),
  plain_text: new Set([
    'plain_text',
    'text',
    'paragraph',
    'reference',
    'abstract',
    'code',
    'page_header',
    'page_footer',
  ]),
  table: new Set(['table', 'table_caption', 'table_note']),
  figure: new Set(['figure', 'image', 'chart', 'figure_title', 'figure_note']),
  abandoned: new Set(['abandoned', 'vision_footnote']),
  formula: new Set(['formula']),
  seal: new Set(['seal']),
}

const FILTER_ALIASES: Record<Exclude<LayoutBlockFilterId, 'all'>, Set<string>> = {
  titles: new Set(['title', 'doc_title', 'paragraph_title']),
  text: new Set([
    'plain_text',
    'text',
    'paragraph',
    'reference',
    'abstract',
    'code',
    'page_header',
    'page_footer',
  ]),
  tables: new Set(['table', 'table_caption']),
  figures: new Set(['figure', 'image', 'chart', 'figure_title']),
  notes: new Set(['note', 'notes', 'vision_footnote', 'abandoned', 'table_note', 'figure_note']),
}

export interface NormalizedLayoutBBox extends LayoutBoundingBox {
  right: number
  bottom: number
  area: number
}

export interface LayoutRegionMatch {
  region: LayoutRegion
  source: 'groupId' | 'category' | 'overlap'
  score: number
}

export interface LayoutBlockView {
  id: string
  regionId: string
  label: string
  order: number
  content: string
  preview: string
  bbox: LayoutBlock['bbox']
  overlayBbox: LayoutBlock['bbox']
  overlaySource: 'region' | 'block'
  page: number
  groupId: number
  imageWidth: number
  imageHeight: number
}

export type LayoutPageEntry = { page?: number }
export type LayoutPageBlockCounts = Record<number, number>

export interface LayoutInteractionState {
  blockId: string | null
  regionId: string | null
  hasMatch: boolean
}

export interface LayoutInteractionSelectionState {
  selectedBlockId: string | null
  selectedRegionId: string | null
  hoveredBlockId: string | null
  hoveredRegionId: string | null
}

function toFiniteNumber(value: number | undefined, fallback = 0) {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function canonicalCategory(value: string | undefined) {
  const normalized = normalizeLayoutToken(value)
  for (const [canonical, aliases] of Object.entries(CATEGORY_ALIASES)) {
    if (aliases.has(normalized)) {
      return canonical
    }
  }

  return normalized
}

function normalizeLayoutToken(value: string | undefined) {
  return (
    value
      ?.trim()
      .toLowerCase()
      .replace(/[\s-]+/g, '_') ?? 'unknown'
  )
}

export function normalizeLayoutPage(page: number | undefined, fallback = 1) {
  if (typeof page !== 'number' || !Number.isFinite(page)) {
    return fallback
  }

  const normalized = Math.trunc(page)
  return normalized >= 1 ? normalized : fallback
}

function getPage(value: LayoutPageEntry) {
  return normalizeLayoutPage(value.page, 1)
}

export function normalizeLayoutBBox(
  bbox: Partial<LayoutBoundingBox> | null | undefined
): NormalizedLayoutBBox {
  const x = toFiniteNumber(bbox?.x)
  const y = toFiniteNumber(bbox?.y)
  const width = Math.max(0, toFiniteNumber(bbox?.width))
  const height = Math.max(0, toFiniteNumber(bbox?.height))

  return {
    x,
    y,
    width,
    height,
    right: x + width,
    bottom: y + height,
    area: width * height,
  }
}

export function getIntersectionArea(a: Partial<LayoutBoundingBox>, b: Partial<LayoutBoundingBox>) {
  const boxA = normalizeLayoutBBox(a)
  const boxB = normalizeLayoutBBox(b)
  const width = Math.max(0, Math.min(boxA.right, boxB.right) - Math.max(boxA.x, boxB.x))
  const height = Math.max(0, Math.min(boxA.bottom, boxB.bottom) - Math.max(boxA.y, boxB.y))
  return width * height
}

export function getOverlapRatio(a: Partial<LayoutBoundingBox>, b: Partial<LayoutBoundingBox>) {
  const boxA = normalizeLayoutBBox(a)
  const boxB = normalizeLayoutBBox(b)
  const intersection = getIntersectionArea(boxA, boxB)
  const baseArea = Math.min(boxA.area, boxB.area)
  if (baseArea <= EPSILON) {
    return 0
  }

  return intersection / baseArea
}

export function getIntersectionOverUnion(
  a: Partial<LayoutBoundingBox>,
  b: Partial<LayoutBoundingBox>
) {
  const boxA = normalizeLayoutBBox(a)
  const boxB = normalizeLayoutBBox(b)
  const intersection = getIntersectionArea(boxA, boxB)
  const union = boxA.area + boxB.area - intersection
  if (union <= EPSILON) {
    return 0
  }

  return intersection / union
}

export function belongsToLayoutPage(entry: LayoutPageEntry | null | undefined, page: number) {
  return getPage(entry ?? {}) === normalizeLayoutPage(page, 1)
}

export function filterBlocksByPage<T extends LayoutPageEntry>(blocks: T[], page: number): T[] {
  return blocks.filter((block) => belongsToLayoutPage(block, page))
}

export function filterRegionsByPage<T extends LayoutPageEntry>(regions: T[], page: number): T[] {
  return regions.filter((region) => belongsToLayoutPage(region, page))
}

export function getPagesFromLayout(
  layout: Pick<AssetLayout, 'blocks' | 'regions'> | null | undefined
): number[] {
  if (!layout) {
    return []
  }

  const pages = new Set<number>()

  for (const block of layout.blocks) {
    pages.add(getPage(block))
  }

  for (const region of layout.regions) {
    pages.add(getPage(region))
  }

  return [...pages].sort((a, b) => a - b)
}

export function getBlockCountByPage<T extends LayoutPageEntry>(blocks: T[]): LayoutPageBlockCounts {
  const counts: LayoutPageBlockCounts = {}

  for (const block of blocks) {
    const page = getPage(block)
    counts[page] = (counts[page] ?? 0) + 1
  }

  return counts
}

export function findLayoutBlockById<T extends Pick<LayoutBlockView, 'id'>>(
  blocks: T[],
  blockId: string | null
): T | null {
  if (!blockId) return null
  return blocks.find((block) => block.id === blockId) ?? null
}

export function findLayoutBlockByRegionId<T extends Pick<LayoutBlockView, 'regionId'>>(
  blocks: T[],
  regionId: string | null
): T | null {
  if (!regionId) return null
  return blocks.find((block) => block.regionId === regionId) ?? null
}

export function getLayoutInteractionStateFromBlockId<
  T extends Pick<LayoutBlockView, 'id' | 'regionId'>,
>(blocks: T[], blockId: string | null): LayoutInteractionState {
  const block = findLayoutBlockById(blocks, blockId)

  return {
    blockId: block?.id ?? null,
    regionId: block?.regionId ?? null,
    hasMatch: Boolean(block),
  }
}

export function getLayoutInteractionStateFromRegionId<
  T extends Pick<LayoutBlockView, 'id' | 'regionId'>,
>(blocks: T[], regionId: string | null): LayoutInteractionState {
  const block = findLayoutBlockByRegionId(blocks, regionId)

  return {
    blockId: block?.id ?? null,
    regionId: block?.regionId ?? null,
    hasMatch: Boolean(block),
  }
}

export function pruneLayoutInteractionSelectionState<
  T extends Pick<LayoutBlockView, 'id' | 'regionId'>,
>(blocks: T[], state: LayoutInteractionSelectionState): LayoutInteractionSelectionState {
  const selected = getLayoutInteractionStateFromBlockId(blocks, state.selectedBlockId)
  const hovered = getLayoutInteractionStateFromBlockId(blocks, state.hoveredBlockId)

  return {
    selectedBlockId: selected.blockId,
    selectedRegionId: selected.regionId,
    hoveredBlockId: hovered.blockId,
    hoveredRegionId: hovered.regionId,
  }
}

export function categoriesMatch(blockLabel: string, regionCategory: string) {
  return canonicalCategory(blockLabel) === canonicalCategory(regionCategory)
}

function getCenterDistanceScore(a: Partial<LayoutBoundingBox>, b: Partial<LayoutBoundingBox>) {
  const boxA = normalizeLayoutBBox(a)
  const boxB = normalizeLayoutBBox(b)
  const centerAX = boxA.x + boxA.width / 2
  const centerAY = boxA.y + boxA.height / 2
  const centerBX = boxB.x + boxB.width / 2
  const centerBY = boxB.y + boxB.height / 2
  const distance = Math.hypot(centerAX - centerBX, centerAY - centerBY)
  const reference = Math.max(
    Math.hypot(boxA.width, boxA.height),
    Math.hypot(boxB.width, boxB.height),
    1
  )
  return Math.max(0, 1 - distance / reference)
}

function getGeometryScore(a: Partial<LayoutBoundingBox>, b: Partial<LayoutBoundingBox>) {
  const overlap = getOverlapRatio(a, b)
  const iou = getIntersectionOverUnion(a, b)
  const proximity = getCenterDistanceScore(a, b)
  return overlap * 0.6 + iou * 0.3 + proximity * 0.1
}

function pickBestRegion(
  block: LayoutBlock,
  candidates: LayoutRegion[],
  source: LayoutRegionMatch['source'],
  minimumScore: number
): LayoutRegionMatch | null {
  let best: LayoutRegionMatch | null = null

  for (const region of candidates) {
    const score = getGeometryScore(block.bbox, region.bbox)
    if (score < minimumScore) {
      continue
    }

    if (!best || score > best.score) {
      best = { region, source, score }
    }
  }

  return best
}

export function matchLayoutRegionToBlock(
  block: LayoutBlock,
  regions: LayoutRegion[]
): LayoutRegionMatch | null {
  const page = getPage(block)
  const pageRegions = filterRegionsByPage(regions, page)
  if (pageRegions.length === 0) {
    return null
  }

  const blockGroupId = Number.isFinite(block.groupId) ? block.groupId : 0
  if (blockGroupId > 0) {
    const groupMatches = pageRegions.filter((region) => region.groupId === blockGroupId)
    const groupMatch = pickBestRegion(block, groupMatches, 'groupId', 0.15)
    if (groupMatch) {
      return groupMatch
    }
  }

  const categoryMatches = pageRegions.filter((region) =>
    categoriesMatch(block.label, region.category)
  )
  const categoryMatch = pickBestRegion(block, categoryMatches, 'category', 0.25)
  if (categoryMatch) {
    return categoryMatch
  }

  return pickBestRegion(block, pageRegions, 'overlap', 0.4)
}

function normalizeTextPreview(value: string) {
  const compact = value.replace(/\s+/g, ' ').trim()
  if (compact.length <= BLOCK_PREVIEW_MAX) {
    return compact
  }

  return `${compact.slice(0, BLOCK_PREVIEW_MAX - 1).trimEnd()}…`
}

function sortBlocks(a: LayoutBlockView, b: LayoutBlockView) {
  if (a.page !== b.page) return a.page - b.page
  if (a.order !== b.order) return a.order - b.order
  if (a.groupId !== b.groupId) return a.groupId - b.groupId
  return a.id.localeCompare(b.id)
}

export function buildLayoutBlockViews(layout: AssetLayout): LayoutBlockView[] {
  return layout.blocks
    .map<LayoutBlockView>((block, index) => {
      const id = `layout-block-${index}`

      return {
        id,
        regionId: `${id}::overlay`,
        label: block.label || 'unknown',
        order: Number.isFinite(block.order) ? block.order : index + 1,
        content: block.content,
        preview: normalizeTextPreview(block.content),
        bbox: block.bbox,
        overlayBbox: block.bbox,
        overlaySource: 'block',
        page: getPage(block),
        groupId: Number.isFinite(block.groupId) ? block.groupId : 0,
        imageWidth: block.imageWidth ?? layout.imageWidth,
        imageHeight: block.imageHeight ?? layout.imageHeight,
      }
    })
    .sort(sortBlocks)
}

export function filterLayoutBlocksByPage(
  blocks: LayoutBlockView[],
  page: number
): LayoutBlockView[] {
  return filterBlocksByPage(blocks, page)
}

export function getLayoutBlockFilterId(
  label: string | undefined
): Exclude<LayoutBlockFilterId, 'all'> | null {
  const normalized = normalizeLayoutToken(label)

  for (const [filterId, aliases] of Object.entries(FILTER_ALIASES) as Array<
    [Exclude<LayoutBlockFilterId, 'all'>, Set<string>]
  >) {
    if (aliases.has(normalized)) {
      return filterId
    }
  }

  return null
}

export function matchesLayoutBlockFilter(
  block: Pick<LayoutBlockView, 'label'>,
  filterId: LayoutBlockFilterId
) {
  if (filterId === 'all') {
    return true
  }

  return getLayoutBlockFilterId(block.label) === filterId
}

export function filterLayoutBlocksByType(blocks: LayoutBlockView[], filterId: LayoutBlockFilterId) {
  return blocks.filter((block) => matchesLayoutBlockFilter(block, filterId))
}

export function countLayoutBlocksByFilter(blocks: LayoutBlockView[]): LayoutBlockFilterCounts {
  const counts: LayoutBlockFilterCounts = {
    all: blocks.length,
    titles: 0,
    text: 0,
    tables: 0,
    figures: 0,
    notes: 0,
  }

  for (const block of blocks) {
    const filterId = getLayoutBlockFilterId(block.label)
    if (filterId) {
      counts[filterId] += 1
    }
  }

  return counts
}

export async function getLayoutByAsset(assetId: string): Promise<AssetLayout | null> {
  return getStore().layouts.findByAsset(assetId)
}
