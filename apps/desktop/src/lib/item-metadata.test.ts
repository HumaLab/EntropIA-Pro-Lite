import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  DebouncedMetadataPersistor,
  IMPORTED_FILE_METADATA_KEY,
  buildTechnicalMetadata,
  getAssetDisplayPath,
  getAssetPathLabel,
  getAssetTypeLabel,
  mergeReservedMetadata,
  normalizeMetadataKey,
  parseImportedFileMetadata,
  parseMetadataRecord,
} from './item-metadata'
import type { Asset, Collection, Item } from '@entropia/store'

describe('item metadata helpers', () => {
  const item = {
    id: 'item-1',
    title: 'Documento de prueba',
  } as Item

  const asset = {
    id: 'asset-1',
    type: 'pdf',
    path: 'uploads/archivo.pdf',
    size: 1536,
  } as Asset

  const collection = {
    id: 'collection-1',
    name: 'Archivo histórico',
  } as Collection

  it('parses custom metadata without exposing reserved imported-file metadata', () => {
    const metadata = parseMetadataRecord(
      JSON.stringify({
        autor: 'Mariano Moreno',
        pages: 3,
        [IMPORTED_FILE_METADATA_KEY]: { originalPath: 'C:/privado/archivo.pdf' },
      })
    )

    expect(metadata).toEqual({ autor: 'Mariano Moreno', pages: '3' })
  })

  it('preserves reserved imported-file metadata when custom metadata is saved', () => {
    const source = JSON.stringify({
      [IMPORTED_FILE_METADATA_KEY]: {
        originalName: 'fuente.pdf',
        originalPath: 'C:/fuente/fuente.pdf',
      },
    })

    expect(mergeReservedMetadata({ autor: 'Belgrano' }, source)).toEqual({
      autor: 'Belgrano',
      [IMPORTED_FILE_METADATA_KEY]: {
        originalName: 'fuente.pdf',
        originalPath: 'C:/fuente/fuente.pdf',
      },
    })
  })

  it('extracts imported file metadata from the reserved key', () => {
    expect(
      parseImportedFileMetadata(
        JSON.stringify({ [IMPORTED_FILE_METADATA_KEY]: { originalName: 'acta.pdf' } })
      )
    ).toEqual({ originalName: 'acta.pdf' })
  })

  it('builds technical metadata and avoids duplicating custom metadata aliases', () => {
    const metadata = buildTechnicalMetadata({
      item,
      selectedAsset: asset,
      collection,
      originalFileMetadata: {
        originalName: 'acta-original.pdf',
        sizeBytes: 2048,
        readonly: true,
      },
      customMetadataKeys: new Set(['ruta interna'].map(normalizeMetadataKey)),
    })

    expect(metadata).toEqual(
      expect.arrayContaining([
        { label: 'Nombre del archivo', value: 'archivo.pdf' },
        { label: 'Tipo de archivo', value: 'PDF' },
        { label: 'Extensión', value: '.pdf' },
        { label: 'Tamaño', value: '1.5 KB' },
        { label: 'Documento ID', value: 'item-1' },
        { label: 'Asset ID', value: 'asset-1' },
        { label: 'Colección', value: 'Archivo histórico' },
        { label: 'Nombre original', value: 'acta-original.pdf' },
        { label: 'Tamaño original', value: '2.0 KB' },
        { label: 'Solo lectura', value: 'Sí' },
      ])
    )
    expect(metadata.some((entry) => entry.label === 'Ruta interna')).toBe(false)
  })

  it('formats asset labels consistently', () => {
    expect(getAssetPathLabel('C:\\documentos\\imagen.png')).toBe('imagen.png')
    expect(
      getAssetPathLabel(
        'C:\\documentos\\7c08cc56-3abc-45a5-89f3-5e05d1a99572_DSC01129_v6.JPG'
      )
    ).toBe('DSC01129_v6.JPG')
    expect(
      getAssetPathLabel('/documentos/11111111-1111-4111-8111-111111111111_entrevista.mp3')
    ).toBe('entrevista.mp3')
    expect(getAssetPathLabel('/documentos/AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE_acta.pdf')).toBe(
      'acta.pdf'
    )
    expect(getAssetPathLabel('/documentos/archivo_2026.pdf')).toBe('archivo_2026.pdf')
    expect(
      getAssetDisplayPath('/documentos/11111111-1111-4111-8111-111111111111_entrevista.mp3')
    ).toBe('/documentos/entrevista.mp3')
    expect(getAssetTypeLabel('image')).toBe('IMAGE')
    expect(getAssetTypeLabel('')).toBe('ASSET')
  })
})

describe('DebouncedMetadataPersistor', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  function createPersistor({
    item = { id: 'item-1', metadata: '{}' },
    updateItem = vi.fn().mockResolvedValue(undefined),
    onSavingChange = vi.fn(),
    onError = vi.fn(),
  }: {
    item?: Pick<Item, 'id' | 'metadata'> | null
    updateItem?: (id: string, patch: { metadata: string }) => Promise<unknown>
    onSavingChange?: (saving: boolean) => void
    onError?: (error: string) => void
  } = {}) {
    let currentItem = item
    const persistor = new DebouncedMetadataPersistor({
      delayMs: 1000,
      getItem: () => currentItem,
      updateItem,
      onSavingChange,
      onError,
    })

    return {
      persistor,
      updateItem,
      onSavingChange,
      onError,
      setItem: (nextItem: Pick<Item, 'id' | 'metadata'> | null) => {
        currentItem = nextItem
      },
    }
  }

  it('coalesces multiple scheduled saves into the latest metadata update', async () => {
    const { persistor, updateItem } = createPersistor()

    persistor.schedule({ autor: 'Moreno' })
    await vi.advanceTimersByTimeAsync(999)
    persistor.schedule({ autor: 'Belgrano' })
    await vi.advanceTimersByTimeAsync(1000)

    expect(updateItem).toHaveBeenCalledTimes(1)
    expect(updateItem).toHaveBeenCalledWith('item-1', {
      metadata: JSON.stringify({ autor: 'Belgrano' }),
    })
  })

  it('preserves reserved imported-file metadata when saving', async () => {
    const { persistor, updateItem } = createPersistor({
      item: {
        id: 'item-1',
        metadata: JSON.stringify({
          [IMPORTED_FILE_METADATA_KEY]: { originalName: 'fuente.pdf' },
        }),
      },
    })

    persistor.schedule({ autor: 'Belgrano' })
    await vi.advanceTimersByTimeAsync(1000)

    expect(updateItem).toHaveBeenCalledWith('item-1', {
      metadata: JSON.stringify({
        autor: 'Belgrano',
        [IMPORTED_FILE_METADATA_KEY]: { originalName: 'fuente.pdf' },
      }),
    })
  })

  it('reports saving state around the debounced update', async () => {
    const { persistor, onSavingChange } = createPersistor()

    persistor.schedule({ autor: 'Moreno' })
    expect(onSavingChange).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1000)

    expect(onSavingChange).toHaveBeenNthCalledWith(1, true)
    expect(onSavingChange).toHaveBeenNthCalledWith(2, false)
  })

  it('reports update errors with the existing fallback message', async () => {
    const { persistor, onError } = createPersistor({
      updateItem: vi.fn().mockRejectedValue('boom'),
    })

    persistor.schedule({ autor: 'Moreno' })
    await vi.advanceTimersByTimeAsync(1000)

    expect(onError).toHaveBeenCalledWith('Failed to save metadata')
  })

  it('cancels pending metadata saves', async () => {
    const { persistor, updateItem } = createPersistor()

    persistor.schedule({ autor: 'Moreno' })
    persistor.cancel()
    await vi.advanceTimersByTimeAsync(1000)

    expect(updateItem).not.toHaveBeenCalled()
  })

  it('does not save when no item is available at execution time', async () => {
    const { persistor, updateItem, setItem } = createPersistor()

    persistor.schedule({ autor: 'Moreno' })
    setItem(null)
    await vi.advanceTimersByTimeAsync(1000)

    expect(updateItem).not.toHaveBeenCalled()
  })
})
