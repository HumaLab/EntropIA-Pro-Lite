import { save } from '@tauri-apps/plugin-dialog'
import { readFile, stat, writeFile } from '@tauri-apps/plugin-fs'
import { invoke } from '@tauri-apps/api/core'
import { settingsGet, SETTINGS_KEYS, DEFAULT_OPENROUTER_MODEL } from './settings'
import type {
  Annotation,
  Asset,
  Collection,
  Extraction,
  Item,
  Note,
  StoreApi,
  Transcription,
  Entity,
  Triple,
  Topic,
  AssetLayout,
} from '@entropia/store'

type JsonRecord = Record<string, unknown>

type ExportBbox = {
  x: number
  y: number
  width: number
  height: number
}

type ExportAsset = {
  id: string
  itemId: string
  filename: string
  type: string
  size: number | null
  path: string
  originalPath: string
  sortIndex: number
  createdAt: number
  text: string | null
  bboxes: ExportBbox[]
  extraction: Extraction | null
  transcription: Transcription | null
  annotations: Annotation[]
  layout: AssetLayout | null
  notes: Note[]
  entities: Entity[]
  triples: Triple[]
  llmResults: JsonRecord[]
  embeddings: JsonRecord[]
  references: JsonRecord[]
  /**
   * Raw file bytes as base64, without the `data:` prefix.
   *
   * Only present when the user opted into embedding assets. `path`/`originalPath`
   * point at the exporting machine's disk, so this field is the only way the
   * picture itself crosses the bridge to the web app.
   */
  data?: string
}

type ExportNote = {
  content: string
  createdAt: number
  updatedAt: number
}

type ExportItem = {
  id: string
  title: string
  metadata: string | null
  metadataParsed: unknown | null
  createdAt: number
  updatedAt: number
  assets: ExportAsset[]
  notes: ExportNote[]
  notesRaw: Note[]
  entities: Entity[]
  triples: Triple[]
  topics: Array<{ topic: Topic; link: JsonRecord | null }>
  llmResults: JsonRecord[]
  references: JsonRecord[]
}

/**
 * The AI credentials, when the user explicitly asks to carry them over so the
 * web app does not have to be set up again. Never written unless ticked by hand
 * at export time — see `loadExportAiConfig`.
 */
export type ExportAiConfig = {
  /** Lowercase, and it must match the web app's list exactly, or the key lands unusable. */
  provider: string
  apiKey: string
  model: string
  baseUrl: string
  name: string
}

/**
 * Which slice of a split export this file is.
 *
 * Absent means the file is a whole export. Present, it lets the web app tell
 * that files 1..N belong together and offer to add them to the same project
 * instead of creating one project per file.
 */
export type ExportPart = {
  index: number
  total: number
}

export interface CollectionExportPayload {
  version: number
  exportedAt: string
  collection: {
    id: string
    name: string
    description: string | null
    createdAt: number
    updatedAt: number
  }
  items: ExportItem[]
  llmResults: JsonRecord[]
  aiConfig?: ExportAiConfig
  part?: ExportPart
}

function toRelativeAssetPath(path: string): string {
  const normalized = path.replace(/\\/g, '/')
  const marker = '/assets/'
  const markerIndex = normalized.lastIndexOf(marker)
  if (markerIndex >= 0) {
    return normalized.slice(markerIndex + 1)
  }
  return normalized
}

function getFilename(path: string): string {
  return path.split(/[/\\]/).pop() ?? path
}

// ---------------------------------------------------------------------------
// Embedded asset bytes (opt-in)
// ---------------------------------------------------------------------------

/** Base64 grows the payload by 4/3; used for the size shown before confirming. */
const BASE64_OVERHEAD = 4 / 3

/**
 * Hard ceiling per exported file, fixed by the web app: it reads the whole file
 * into memory and then decodes it, and both live at once. Roughly 40-50 scans of
 * 1-1.2 MB. Collections past this are split into several files, which the web
 * imports one after another (re-importing updates, it does not duplicate).
 */
export const WEB_UPLOAD_LIMIT_BYTES = 64 * 1024 * 1024

/** Audio never crosses the bridge — its transcription already travels in `text`. */
function assetTypeCarriesData(type: string): boolean {
  return type !== 'audio'
}

/**
 * Parent assets of a split PDF hold the same bytes as their page assets, which
 * are the units the app actually shows and OCRs. Embedding both would double
 * the file for no extra content.
 */
function collectSupersededParentIds(assetsByItemId: Record<string, Asset[]>): Set<string> {
  const parentIds = new Set<string>()
  for (const assets of Object.values(assetsByItemId)) {
    for (const asset of assets) {
      if (asset.parentAssetId) parentIds.add(asset.parentAssetId)
    }
  }
  return parentIds
}

/** Assets whose bytes get embedded when the user asks for images in the file. */
function selectEmbeddableAssets(assetsByItemId: Record<string, Asset[]>): Asset[] {
  const superseded = collectSupersededParentIds(assetsByItemId)
  return Object.values(assetsByItemId)
    .flat()
    .filter((asset) => assetTypeCarriesData(asset.type) && !superseded.has(asset.id))
}

/**
 * Encode bytes as base64 in chunks — spreading a whole multi-megabyte array into
 * `String.fromCharCode` overflows the call stack.
 */
export function bytesToBase64(bytes: Uint8Array): string {
  const CHUNK_SIZE = 0x8000
  let binary = ''
  for (let offset = 0; offset < bytes.length; offset += CHUNK_SIZE) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + CHUNK_SIZE))
  }
  return btoa(binary)
}

/** Magic-number signatures, so a filename without extension still tells the web the type. */
const FILE_SIGNATURES: Array<{ extension: string; matches: (bytes: Uint8Array) => boolean }> = [
  {
    extension: '.png',
    matches: (b) => b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47,
  },
  { extension: '.jpg', matches: (b) => b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff },
  { extension: '.gif', matches: (b) => b[0] === 0x47 && b[1] === 0x49 && b[2] === 0x46 },
  {
    extension: '.pdf',
    matches: (b) => b[0] === 0x25 && b[1] === 0x50 && b[2] === 0x44 && b[3] === 0x46,
  },
  {
    extension: '.webp',
    matches: (b) =>
      b[0] === 0x52 && b[1] === 0x49 && b[8] === 0x57 && b[9] === 0x45 && b[10] === 0x42,
  },
  { extension: '.bmp', matches: (b) => b[0] === 0x42 && b[1] === 0x4d },
  {
    extension: '.tif',
    matches: (b) => (b[0] === 0x49 && b[1] === 0x49) || (b[0] === 0x4d && b[1] === 0x4d),
  },
]

function hasExtension(filename: string): boolean {
  return /\.[a-z0-9]{1,5}$/i.test(filename)
}

/**
 * The web app derives the file type from `filename`, so a name without extension
 * would arrive unusable. Recover it from the bytes we already hold.
 */
export function ensureFilenameExtension(filename: string, bytes: Uint8Array): string {
  if (hasExtension(filename)) return filename
  const signature = FILE_SIGNATURES.find((candidate) => candidate.matches(bytes))
  return signature ? `${filename}${signature.extension}` : filename
}

// ---------------------------------------------------------------------------
// AI credentials (opt-in, per export)
// ---------------------------------------------------------------------------

/**
 * Exact spelling the web app expects. Anything it does not recognise is stored
 * as OpenAI with the wrong address, and the key arrives unusable without anyone
 * noticing — so this string is agreed on both sides of the bridge.
 */
export const EXPORT_AI_PROVIDER = 'openrouter'

/** Sent as-is: the web app completes it to the endpoint it needs. */
export const EXPORT_AI_BASE_URL = 'https://openrouter.ai/api/v1'

/**
 * Read the configured AI credentials for an export the user asked for by hand.
 *
 * Returns null when there is no key to carry. Nothing else in the app calls
 * this, which is what keeps the key out of automatic backups.
 */
export async function loadExportAiConfig(name: string): Promise<ExportAiConfig | null> {
  const apiKey = (await settingsGet(SETTINGS_KEYS.OPENROUTER_API_KEY))?.trim()
  if (!apiKey) return null

  const model =
    (await settingsGet(SETTINGS_KEYS.OPENROUTER_MODEL))?.trim() || DEFAULT_OPENROUTER_MODEL

  return {
    provider: EXPORT_AI_PROVIDER,
    apiKey,
    model,
    baseUrl: EXPORT_AI_BASE_URL,
    name,
  }
}

/** One exported file: the documents it carries and what it will weigh. */
export type ExportBatch = {
  itemIds: string[]
  encodedBytes: number
  /** A single document heavier than the ceiling — it cannot be split further. */
  exceedsWebLimit: boolean
}

export type EmbeddedAssetEstimate = {
  /** Assets that would carry their bytes. */
  assetCount: number
  /** Raw bytes on disk. */
  bytes: number
  /** Rough size of the whole export once base64-encoded, across every file. */
  encodedBytes: number
  /** How the documents get distributed across files to stay under the ceiling. */
  batches: ExportBatch[]
  /** Documents that bust the ceiling on their own; the web app will reject those files. */
  oversizedItemIds: string[]
}

/**
 * Distribute documents across files so each stays under the ceiling.
 *
 * Documents are never split across files: an asset's text and its picture have
 * to arrive together. A document heavier than the ceiling on its own gets a file
 * of its own and is reported as oversized.
 */
export function planExportBatches(
  itemIds: string[],
  encodedBytesByItemId: Record<string, number>,
  limitBytes: number = WEB_UPLOAD_LIMIT_BYTES
): ExportBatch[] {
  const batches: ExportBatch[] = []
  let current: ExportBatch | null = null

  for (const itemId of itemIds) {
    const itemBytes = encodedBytesByItemId[itemId] ?? 0

    if (itemBytes > limitBytes) {
      batches.push({ itemIds: [itemId], encodedBytes: itemBytes, exceedsWebLimit: true })
      current = null
      continue
    }

    if (!current || current.encodedBytes + itemBytes > limitBytes) {
      current = { itemIds: [], encodedBytes: 0, exceedsWebLimit: false }
      batches.push(current)
    }

    current.itemIds.push(itemId)
    current.encodedBytes += itemBytes
  }

  return batches.length > 0 ? batches : [{ itemIds: [], encodedBytes: 0, exceedsWebLimit: false }]
}

async function readAssetSize(asset: Asset): Promise<number> {
  if (typeof asset.size === 'number' && Number.isFinite(asset.size) && asset.size > 0) {
    return asset.size
  }
  try {
    const metadata = await stat(asset.path)
    const size = Number(metadata.size ?? 0)
    return Number.isFinite(size) ? size : 0
  } catch (e) {
    console.warn(
      '[export] Could not measure asset, excluded from the size estimate:',
      asset.path,
      e
    )
    return 0
  }
}

/** Size the embedded-asset option would add, shown before the user confirms. */
export async function estimateEmbeddedAssets(
  assetsByItemId: Record<string, Asset[]>,
  itemIds: string[] = Object.keys(assetsByItemId)
): Promise<EmbeddedAssetEstimate> {
  const superseded = collectSupersededParentIds(assetsByItemId)
  const encodedBytesByItemId: Record<string, number> = {}
  let assetCount = 0
  let bytes = 0

  for (const itemId of itemIds) {
    const embeddable = (assetsByItemId[itemId] ?? []).filter(
      (asset) => assetTypeCarriesData(asset.type) && !superseded.has(asset.id)
    )
    const sizes = await Promise.all(embeddable.map((asset) => readAssetSize(asset)))
    const itemBytes = sizes.reduce((total, size) => total + size, 0)
    assetCount += embeddable.length
    bytes += itemBytes
    encodedBytesByItemId[itemId] = Math.round(itemBytes * BASE64_OVERHEAD)
  }

  const batches = planExportBatches(itemIds, encodedBytesByItemId)
  return {
    assetCount,
    bytes,
    encodedBytes: Math.round(bytes * BASE64_OVERHEAD),
    batches,
    oversizedItemIds: batches.filter((batch) => batch.exceedsWebLimit).flatMap((b) => b.itemIds),
  }
}

export type LoadedAssetData = {
  /** base64 payload per asset id. */
  dataByAssetId: Record<string, string>
  /** Corrected filename per asset id, when the original had no extension. */
  filenameByAssetId: Record<string, string>
  /** Assets whose file could not be read; the export continues without them. */
  skippedAssetIds: string[]
}

/**
 * Read every embeddable asset from disk and encode it.
 *
 * A file missing from disk is skipped rather than aborting the export — losing
 * one picture must not cost the whole collection.
 */
export async function loadEmbeddedAssetData(
  assetsByItemId: Record<string, Asset[]>
): Promise<LoadedAssetData> {
  const dataByAssetId: Record<string, string> = {}
  const filenameByAssetId: Record<string, string> = {}
  const skippedAssetIds: string[] = []

  for (const asset of selectEmbeddableAssets(assetsByItemId)) {
    try {
      const bytes = await readFile(asset.path)
      dataByAssetId[asset.id] = bytesToBase64(bytes)
      filenameByAssetId[asset.id] = ensureFilenameExtension(getFilename(asset.path), bytes)
    } catch (e) {
      console.warn('[export] Could not read asset, exported without its file:', asset.path, e)
      skippedAssetIds.push(asset.id)
    }
  }

  return { dataByAssetId, filenameByAssetId, skippedAssetIds }
}

/**
 * Collect all rectangle annotation bounding boxes.
 * Underline annotations are excluded — they are not representative bboxes.
 */
function collectBboxes(annotations: Annotation[]): ExportBbox[] {
  return annotations
    .filter((a) => a.kind === 'rectangle')
    .map((a) => ({ x: a.x, y: a.y, width: a.width, height: a.height }))
}

function parseJsonOrNull(value: string | null | undefined): unknown | null {
  if (!value) return null
  try {
    return JSON.parse(value)
  } catch {
    return null
  }
}

function byKey<T extends Record<string, unknown>>(rows: T[], key: keyof T): Record<string, T[]> {
  const grouped: Record<string, T[]> = {}
  for (const row of rows) {
    const value = row[key]
    if (typeof value !== 'string') continue
    grouped[value] = [...(grouped[value] ?? []), row]
  }
  return grouped
}

function normalizeLayout(layout: AssetLayout | null): AssetLayout | null {
  return layout ?? null
}

async function selectRows<T extends JsonRecord>(sql: string, params: unknown[]): Promise<T[]> {
  const rows = await invoke<T[]>('db_select', { sql, params })
  return Array.isArray(rows) ? rows : []
}

async function loadCompleteExportRows(collectionId: string) {
  const [itemTopics, topics, itemLlmResults, collectionLlmResults, assetLlmResults, embeddings] =
    await Promise.all([
      selectRows(
        'SELECT it.* FROM item_topics it JOIN items i ON i.id = it.item_id WHERE i.collection_id = ?',
        [collectionId]
      ),
      selectRows(
        'SELECT t.* FROM topics t JOIN item_topics it ON it.topic_id = t.id JOIN items i ON i.id = it.item_id WHERE i.collection_id = ?',
        [collectionId]
      ),
      selectRows(
        'SELECT lr.* FROM llm_results lr JOIN items i ON i.id = lr.target_id WHERE i.collection_id = ?',
        [collectionId]
      ),
      selectRows('SELECT lr.* FROM llm_results lr WHERE lr.target_id = ?', [collectionId]),
      selectRows(
        'SELECT lr.* FROM llm_results lr JOIN assets a ON a.id = lr.target_id JOIN items i ON i.id = a.item_id WHERE i.collection_id = ?',
        [collectionId]
      ),
      selectRows(
        'SELECT v.asset_id, v.item_id, v.embedding FROM vec_assets v JOIN items i ON i.id = v.item_id WHERE i.collection_id = ?',
        [collectionId]
      ),
    ])

  return {
    itemTopics,
    topics,
    itemLlmResults,
    collectionLlmResults,
    assetLlmResults,
    embeddings,
  }
}

export function buildCollectionExportData(
  collection: Collection,
  items: Item[],
  assetsByItemId: Record<string, Asset[]>,
  notesByItemId: Record<string, Note[]>,
  extractionsByAssetId: Record<string, Extraction | null>,
  annotationsByAssetId: Record<string, Annotation[]>,
  transcriptionsByAssetId: Record<string, Transcription | null> = {},
  layoutsByAssetId: Record<string, AssetLayout | null> = {},
  entitiesByItemId: Record<string, Entity[]> = {},
  entitiesByAssetId: Record<string, Entity[]> = {},
  triplesByItemId: Record<string, Triple[]> = {},
  triplesByAssetId: Record<string, Triple[]> = {},
  itemTopicsByItemId: Record<string, Array<{ topic: Topic; link: JsonRecord | null }>> = {},
  itemLlmResultsByTargetId: Record<string, JsonRecord[]> = {},
  assetLlmResultsByTargetId: Record<string, JsonRecord[]> = {},
  embeddingsByAssetId: Record<string, JsonRecord[]> = {},
  collectionLlmResults: JsonRecord[] = [],
  embedded: Pick<LoadedAssetData, 'dataByAssetId' | 'filenameByAssetId'> = {
    dataByAssetId: {},
    filenameByAssetId: {},
  },
  aiConfig: ExportAiConfig | null = null,
  part: ExportPart | null = null
): CollectionExportPayload {
  return {
    version: 3,
    exportedAt: new Date().toISOString(),
    ...(aiConfig ? { aiConfig } : {}),
    ...(part ? { part } : {}),
    collection: {
      id: collection.id,
      name: collection.name,
      description: collection.description,
      createdAt: collection.createdAt,
      updatedAt: collection.updatedAt,
    },
    items: items.map((item) => ({
      id: item.id,
      title: item.title,
      metadata: item.metadata,
      metadataParsed: parseJsonOrNull(item.metadata),
      createdAt: item.createdAt,
      updatedAt: item.updatedAt,
      assets: (assetsByItemId[item.id] ?? []).map((asset) => ({
        id: asset.id,
        itemId: asset.itemId,
        filename: embedded.filenameByAssetId[asset.id] ?? getFilename(asset.path),
        type: asset.type,
        size: asset.size ?? null,
        path: toRelativeAssetPath(asset.path),
        originalPath: asset.path,
        sortIndex: asset.sortIndex,
        createdAt: asset.createdAt,
        text:
          extractionsByAssetId[asset.id]?.textContent ??
          transcriptionsByAssetId[asset.id]?.textContent ??
          null,
        bboxes: collectBboxes(annotationsByAssetId[asset.id] ?? []),
        extraction: extractionsByAssetId[asset.id] ?? null,
        transcription: transcriptionsByAssetId[asset.id] ?? null,
        annotations: annotationsByAssetId[asset.id] ?? [],
        layout: normalizeLayout(layoutsByAssetId[asset.id] ?? null),
        notes: (notesByItemId[item.id] ?? []).filter((note) => note.assetId === asset.id),
        entities: entitiesByAssetId[asset.id] ?? [],
        triples: triplesByAssetId[asset.id] ?? [],
        llmResults: assetLlmResultsByTargetId[asset.id] ?? [],
        embeddings: embeddingsByAssetId[asset.id] ?? [],
        references: [],
        ...(embedded.dataByAssetId[asset.id] ? { data: embedded.dataByAssetId[asset.id] } : {}),
      })),
      notes: (notesByItemId[item.id] ?? []).map((note) => ({
        content: note.content,
        createdAt: note.createdAt,
        updatedAt: note.updatedAt,
      })),
      notesRaw: notesByItemId[item.id] ?? [],
      entities: entitiesByItemId[item.id] ?? [],
      triples: triplesByItemId[item.id] ?? [],
      topics: itemTopicsByItemId[item.id] ?? [],
      llmResults: itemLlmResultsByTargetId[item.id] ?? [],
      references: [],
    })),
    llmResults: collectionLlmResults,
  }
}

/**
 * Weigh a collection's assets without exporting anything, so the export dialog
 * can show what the file would end up weighing before the user commits.
 */
export async function estimateCollectionEmbeddedAssets(
  store: StoreApi,
  collectionId: string
): Promise<EmbeddedAssetEstimate> {
  const items = await store.items.findByCollection(collectionId)
  const assetsByItemId: Record<string, Asset[]> = {}
  for (const item of items) {
    assetsByItemId[item.id] = await store.assets.findByItem(item.id)
  }
  return estimateEmbeddedAssets(
    assetsByItemId,
    items.map((item) => item.id)
  )
}

export type CollectionExportOptions = {
  /**
   * Embed each asset's bytes in the file. Off by default: it turns a few-megabyte
   * file into a hundreds-of-megabytes one.
   */
  includeAssetData?: boolean
  /**
   * Carry the AI credentials in the file. Off by default, and only ever set from
   * a box the person ticked in that moment — whoever holds the file can spend
   * with that account.
   */
  aiConfig?: ExportAiConfig | null
  /**
   * Warning woven into the exported filename when the key travels, e.g. `con llave`.
   * The view passes it translated.
   */
  aiConfigFilenameMarker?: string
}

export type CollectionExportResult = {
  /** Every file written. Empty when the user cancelled the save dialog. */
  paths: string[]
  /** Assets whose file could not be read; exported without their picture. */
  skippedAssetIds: string[]
  /** How many assets carried their bytes. */
  embeddedAssetCount: number
}

/** `Archivo.json` + (2 of 3) → `Archivo (2 de 3).json`. */
function numberedExportPath(basePath: string, index: number, total: number): string {
  const match = basePath.match(/^(.*?)(\.[^.\\/]+)?$/)
  const stem = match?.[1] ?? basePath
  const extension = match?.[2] ?? '.json'
  return `${stem} (${index} de ${total})${extension}`
}

export async function exportCollectionById(
  store: StoreApi,
  collectionId: string,
  options: CollectionExportOptions = {}
): Promise<CollectionExportResult | null> {
  const collection = await store.collections.findById(collectionId)
  if (!collection) return null

  const items = await store.items.findByCollection(collectionId)

  const assetsByItemId: Record<string, Asset[]> = {}
  const notesByItemId: Record<string, Note[]> = {}
  const extractionsByAssetId: Record<string, Extraction | null> = {}
  const annotationsByAssetId: Record<string, Annotation[]> = {}
  const transcriptionsByAssetId: Record<string, Transcription | null> = {}
  const layoutsByAssetId: Record<string, AssetLayout | null> = {}
  const entitiesByItemId: Record<string, Entity[]> = {}
  const entitiesByAssetId: Record<string, Entity[]> = {}
  const triplesByItemId: Record<string, Triple[]> = {}
  const triplesByAssetId: Record<string, Triple[]> = {}

  for (const item of items) {
    const [assets, notes, entities, triples] = await Promise.all([
      store.assets.findByItem(item.id),
      store.notes.findByItem(item.id),
      store.entities?.findByItemId?.(item.id) ?? Promise.resolve([]),
      store.triples?.findByItemId?.(item.id) ?? Promise.resolve([]),
    ])
    assetsByItemId[item.id] = assets
    notesByItemId[item.id] = notes
    entitiesByItemId[item.id] = entities
    triplesByItemId[item.id] = triples

    for (const entity of entities) {
      if (!entity.assetId) continue
      entitiesByAssetId[entity.assetId] = [...(entitiesByAssetId[entity.assetId] ?? []), entity]
    }
    for (const triple of triples) {
      if (!triple.assetId) continue
      triplesByAssetId[triple.assetId] = [...(triplesByAssetId[triple.assetId] ?? []), triple]
    }

    for (const asset of assets) {
      const [extraction, assetAnnotations, transcription, layout] = await Promise.all([
        store.extractions.findByAsset(asset.id),
        store.annotations.findByAsset(asset.id),
        store.transcriptions.findByAsset(asset.id),
        store.layouts?.findByAsset?.(asset.id) ?? Promise.resolve(null),
      ])
      extractionsByAssetId[asset.id] = extraction
      annotationsByAssetId[asset.id] = assetAnnotations
      transcriptionsByAssetId[asset.id] = transcription
      layoutsByAssetId[asset.id] = layout
    }
  }

  const itemIds = items.map((item) => item.id)
  const batches = options.includeAssetData
    ? (await estimateEmbeddedAssets(assetsByItemId, itemIds)).batches
    : [{ itemIds, encodedBytes: 0, exceedsWebLimit: false }]

  const completeRows = await loadCompleteExportRows(collectionId)
  const topicById = new Map(
    completeRows.topics.map((topic) => [String(topic.id), topic as unknown as Topic])
  )
  const itemTopicsByItemId: Record<string, Array<{ topic: Topic; link: JsonRecord | null }>> = {}
  for (const link of completeRows.itemTopics) {
    const itemId = String(link.item_id)
    const topic = topicById.get(String(link.topic_id))
    if (!topic) continue
    itemTopicsByItemId[itemId] = [...(itemTopicsByItemId[itemId] ?? []), { topic, link }]
  }

  const keyMarker = options.aiConfig ? (options.aiConfigFilenameMarker ?? 'con llave') : null
  const defaultName = keyMarker
    ? `${collection.name} (${keyMarker}).json`
    : `${collection.name}.json`
  const basePath = await promptForExportPath(defaultName)
  if (!basePath) return { paths: [], skippedAssetIds: [], embeddedAssetCount: 0 }

  const paths: string[] = []
  const skippedAssetIds: string[] = []
  let embeddedAssetCount = 0

  // One batch at a time: each file is built, written and released before the
  // next one starts, so the exporting machine never holds the whole collection
  // encoded in memory at once.
  for (const [index, batch] of batches.entries()) {
    const batchItemIds = new Set(batch.itemIds)
    const batchItems = items.filter((item) => batchItemIds.has(item.id))
    const batchAssetsByItemId: Record<string, Asset[]> = {}
    for (const itemId of batch.itemIds) {
      batchAssetsByItemId[itemId] = assetsByItemId[itemId] ?? []
    }

    const embedded: LoadedAssetData = options.includeAssetData
      ? await loadEmbeddedAssetData(batchAssetsByItemId)
      : { dataByAssetId: {}, filenameByAssetId: {}, skippedAssetIds: [] }

    const payload = buildCollectionExportData(
      collection,
      batchItems,
      batchAssetsByItemId,
      notesByItemId,
      extractionsByAssetId,
      annotationsByAssetId,
      transcriptionsByAssetId,
      layoutsByAssetId,
      entitiesByItemId,
      entitiesByAssetId,
      triplesByItemId,
      triplesByAssetId,
      itemTopicsByItemId,
      byKey(completeRows.itemLlmResults, 'target_id'),
      byKey(completeRows.assetLlmResults, 'target_id'),
      byKey(completeRows.embeddings, 'asset_id'),
      completeRows.collectionLlmResults,
      embedded,
      // Every file is standalone, so the key rides in each one: the person may
      // upload them in any order, and they all travel together anyway.
      options.aiConfig ?? null,
      // Only a split export is numbered — a lone file is a whole export, and
      // the web app treats a missing `part` that way.
      batches.length > 1 ? { index: index + 1, total: batches.length } : null
    )

    const targetPath =
      batches.length > 1 ? numberedExportPath(basePath, index + 1, batches.length) : basePath
    await writeJsonFile(targetPath, payload)

    paths.push(targetPath)
    skippedAssetIds.push(...embedded.skippedAssetIds)
    embeddedAssetCount += Object.keys(embedded.dataByAssetId).length
  }

  return { paths, skippedAssetIds, embeddedAssetCount }
}

/** Ask where to save. Returns null when the user cancels. */
async function promptForExportPath(defaultName: string): Promise<string | null> {
  return save({
    defaultPath: defaultName,
    filters: [
      {
        name: 'JSON',
        extensions: ['json'],
      },
    ],
  })
}

async function writeJsonFile(filePath: string, data: object): Promise<void> {
  const json = JSON.stringify(data, null, 2)
  const bytes = new TextEncoder().encode(json)
  await writeFile(filePath, bytes)
}

/**
 * Export data as a JSON file via the native save dialog.
 * Returns the chosen file path, or null if the user cancelled.
 */
export async function exportCollectionToJson(
  data: object,
  defaultName: string
): Promise<string | null> {
  const filePath = await promptForExportPath(defaultName)
  if (!filePath) return null
  await writeJsonFile(filePath, data)
  return filePath
}
