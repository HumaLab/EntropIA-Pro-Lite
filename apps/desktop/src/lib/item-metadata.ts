import type { Asset, Collection, Item } from '@entropia/store'

export const IMPORTED_FILE_METADATA_KEY = '__entropia_file_metadata'

export type ImportedFileMetadata = {
  originalName?: string
  originalPath?: string
  importedAt?: string
  sizeBytes?: number
  readonly?: boolean
  isFile?: boolean
  isDirectory?: boolean
  createdAt?: number | null
  modifiedAt?: number | null
  accessedAt?: number | null
}

export type TechnicalMetadataEntry = {
  label: string
  value: string
}

type MetadataPersistItem = Pick<Item, 'id' | 'metadata'>

type DebouncedMetadataPersistorOptions = {
  delayMs?: number
  getItem: () => MetadataPersistItem | null
  updateItem: (id: string, patch: { metadata: string }) => Promise<unknown>
  onSavingChange: (saving: boolean) => void
  onError: (error: string) => void
}

export class DebouncedMetadataPersistor {
  private timer: ReturnType<typeof setTimeout> | null = null
  private readonly delayMs: number
  private readonly getItem: () => MetadataPersistItem | null
  private readonly updateItem: (id: string, patch: { metadata: string }) => Promise<unknown>
  private readonly onSavingChange: (saving: boolean) => void
  private readonly onError: (error: string) => void

  constructor({
    delayMs = 1000,
    getItem,
    updateItem,
    onSavingChange,
    onError,
  }: DebouncedMetadataPersistorOptions) {
    this.delayMs = delayMs
    this.getItem = getItem
    this.updateItem = updateItem
    this.onSavingChange = onSavingChange
    this.onError = onError
  }

  schedule(metadata: Record<string, string>) {
    this.cancel()
    this.timer = setTimeout(() => {
      void this.persist(metadata)
    }, this.delayMs)
  }

  cancel() {
    if (!this.timer) return
    clearTimeout(this.timer)
    this.timer = null
  }

  private async persist(metadata: Record<string, string>) {
    this.timer = null
    const item = this.getItem()
    if (!item) return

    try {
      this.onSavingChange(true)
      await this.updateItem(item.id, {
        metadata: JSON.stringify(mergeReservedMetadata(metadata, item.metadata)),
      })
    } catch (e) {
      this.onError(e instanceof Error ? e.message : 'Failed to save metadata')
    } finally {
      this.onSavingChange(false)
    }
  }
}

export function parseMetadataRecord(json: string): Record<string, string> {
  try {
    const obj = JSON.parse(json)
    const record: Record<string, string> = {}
    for (const [key, value] of Object.entries(obj)) {
      if (key === IMPORTED_FILE_METADATA_KEY) continue
      record[key] = String(value)
    }
    return record
  } catch {
    return {}
  }
}

export function parseImportedFileMetadata(json: string): ImportedFileMetadata | null {
  try {
    const obj = JSON.parse(json) as Record<string, unknown>
    const metadata = obj[IMPORTED_FILE_METADATA_KEY]
    return metadata && typeof metadata === 'object' ? (metadata as ImportedFileMetadata) : null
  } catch {
    return null
  }
}

export function mergeReservedMetadata(
  metadata: Record<string, string>,
  sourceMetadata?: string | null
): Record<string, unknown> {
  const reserved = sourceMetadata ? parseImportedFileMetadata(sourceMetadata) : null
  return reserved ? { ...metadata, [IMPORTED_FILE_METADATA_KEY]: reserved } : metadata
}

export function getAssetPathLabel(path: string) {
  const fileName = path.split(/[/\\]/).pop() ?? path
  return fileName.replace(/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}_/i, '')
}

export function buildExportDefaultName(path: string) {
  const fileName = getAssetPathLabel(path)
  const extension = getFileExtension(fileName)
  return extension ? fileName.slice(0, fileName.length - extension.length) : fileName
}

export function getAssetDisplayPath(path: string) {
  const fileName = path.split(/[/\\]/).pop() ?? path
  if (!fileName) return path

  return `${path.slice(0, path.length - fileName.length)}${getAssetPathLabel(fileName)}`
}

export function getAssetTypeLabel(assetType: string) {
  return assetType ? assetType.toUpperCase() : 'ASSET'
}

export function normalizeMetadataKey(key: string) {
  return key
    .trim()
    .toLowerCase()
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
}

function getFileExtension(fileName: string): string | null {
  const index = fileName.lastIndexOf('.')
  if (index <= 0 || index === fileName.length - 1) return null
  return fileName.slice(index).toLowerCase()
}

function formatBytes(size: number | null | undefined): string | null {
  if (size === null || size === undefined || !Number.isFinite(size) || size < 0) return null
  if (size < 1024) return `${size} B`

  const units = ['KB', 'MB', 'GB', 'TB']
  let value = size / 1024
  let unitIndex = 0

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024
    unitIndex++
  }

  const digits = value >= 10 ? 0 : 1
  return `${value.toFixed(digits)} ${units[unitIndex]}`
}

function formatTimestamp(timestamp: number | string | null | undefined): string | null {
  if (timestamp === null || timestamp === undefined) return null
  const millis = typeof timestamp === 'string' ? Date.parse(timestamp) : timestamp
  if (!Number.isFinite(millis)) return null
  return new Date(millis).toLocaleString()
}

function formatBoolean(value: boolean | null | undefined): string | null {
  if (value === null || value === undefined) return null
  return value ? 'Sí' : 'No'
}

function pushTechnicalMetadataEntry(
  entries: TechnicalMetadataEntry[],
  customMetadataKeys: Set<string>,
  label: string,
  value: string | null | undefined,
  aliases: string[] = []
) {
  if (!value) return

  const normalizedCandidates = [label, ...aliases].map((candidate) => normalizeMetadataKey(candidate))
  if (normalizedCandidates.some((candidate) => customMetadataKeys.has(candidate))) {
    return
  }

  entries.push({ label, value })
}

export function buildTechnicalMetadata({
  item,
  selectedAsset,
  collection,
  originalFileMetadata,
  customMetadataKeys,
}: {
  item: Item | null
  selectedAsset: Asset | null
  collection: Collection | null
  originalFileMetadata: ImportedFileMetadata | null
  customMetadataKeys: Set<string>
}): TechnicalMetadataEntry[] {
  const entries: TechnicalMetadataEntry[] = []
  const fileName = selectedAsset ? getAssetPathLabel(selectedAsset.path) : null
  const extension = fileName ? getFileExtension(fileName) : null

  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Nombre del archivo', fileName, [
    'archivo',
    'nombre archivo',
    'file name',
  ])
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Tipo de archivo',
    selectedAsset?.type ? getAssetTypeLabel(selectedAsset.type) : null,
    ['tipo', 'tipo archivo', 'file type', 'mime', 'mime type']
  )
  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Extensión', extension, ['extension', 'ext'])
  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Tamaño', formatBytes(selectedAsset?.size), [
    'tamano',
    'tamaño archivo',
    'file size',
    'size',
  ])
  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Documento ID', item?.id ?? null, [
    'documento id',
    'document id',
    'item id',
    'id',
  ])
  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Asset ID', selectedAsset?.id ?? null, [
    'asset id',
    'archivo id',
  ])
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Ruta interna',
    selectedAsset ? getAssetDisplayPath(selectedAsset.path) : null,
    ['ruta interna', 'internal path', 'path']
  )
  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Colección', collection?.name ?? null, [
    'coleccion',
    'collection',
    'project',
    'proyecto',
  ])

  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Nombre original', originalFileMetadata?.originalName, [
    'original name',
    'nombre fuente',
  ])
  pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Ruta original', originalFileMetadata?.originalPath, [
    'source path',
    'ruta fuente',
  ])
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Tamaño original',
    formatBytes(originalFileMetadata?.sizeBytes),
    ['original size', 'source size']
  )
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Importado el',
    formatTimestamp(originalFileMetadata?.importedAt),
    ['imported at', 'fecha importacion']
  )
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Creado en origen',
    formatTimestamp(originalFileMetadata?.createdAt),
    ['created at', 'fecha creacion origen']
  )
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Modificado en origen',
    formatTimestamp(originalFileMetadata?.modifiedAt),
    ['modified at', 'fecha modificacion origen']
  )
  pushTechnicalMetadataEntry(
    entries,
    customMetadataKeys,
    'Solo lectura',
    formatBoolean(originalFileMetadata?.readonly),
    ['readonly', 'read only']
  )

  return entries
}
