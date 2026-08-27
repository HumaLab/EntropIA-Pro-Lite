import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { invoke } from '@tauri-apps/api/core'
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
  collectionLlmResults: JsonRecord[] = []
): CollectionExportPayload {
  return {
    version: 3,
    exportedAt: new Date().toISOString(),
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
        filename: getFilename(asset.path),
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

export async function exportCollectionById(
  store: StoreApi,
  collectionId: string
): Promise<string | null> {
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

  const payload = buildCollectionExportData(
    collection,
    items,
    assetsByItemId,
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
    completeRows.collectionLlmResults
  )
  return exportCollectionToJson(payload, `${collection.name}.json`)
}

/**
 * Export data as a JSON file via the native save dialog.
 * Returns the chosen file path, or null if the user cancelled.
 */
export async function exportCollectionToJson(
  data: object,
  defaultName: string
): Promise<string | null> {
  const filePath = await save({
    defaultPath: defaultName,
    filters: [
      {
        name: 'JSON',
        extensions: ['json'],
      },
    ],
  })

  if (!filePath) return null

  const json = JSON.stringify(data, null, 2)
  const bytes = new TextEncoder().encode(json)
  await writeFile(filePath, bytes)

  return filePath
}

function serializeCsvCell(value: unknown): string {
  if (value === null || value === undefined) return ''

  const text = typeof value === 'object' ? JSON.stringify(value) : String(value)
  if (!/[;"\r\n]/.test(text)) return text

  return `"${text.replaceAll('"', '""')}"`
}

/**
 * Export database rows as an Excel-compatible semicolon-delimited CSV file.
 * Returns the chosen file path, or null if the user cancelled.
 */
export async function exportCollectionToCsv(
  rows: Record<string, unknown>[],
  columns: string[],
  defaultName: string
): Promise<string | null> {
  const filePath = await save({
    defaultPath: defaultName,
    filters: [
      {
        name: 'CSV',
        extensions: ['csv'],
      },
    ],
  })

  if (!filePath) return null

  const records = [columns.map(serializeCsvCell).join(';')]
  for (const row of rows) {
    records.push(columns.map((column) => serializeCsvCell(row[column])).join(';'))
  }

  const csv = `\uFEFF${records.join('\r\n')}\r\n`
  await writeFile(filePath, new TextEncoder().encode(csv))

  return filePath
}
