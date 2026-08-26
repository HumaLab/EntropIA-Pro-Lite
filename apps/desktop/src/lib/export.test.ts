import { describe, it, expect, vi, beforeEach } from 'vitest'
import {
  exportCollectionToJson,
  buildCollectionExportData,
  exportCollectionById,
  estimateCollectionEmbeddedAssets,
  planExportBatches,
  bytesToBase64,
  loadExportAiConfig,
  WEB_UPLOAD_LIMIT_BYTES,
} from './export'
import { DEFAULT_OPENROUTER_MODEL } from './settings'

type ExportStoreMock = {
  collections: { findById: ReturnType<typeof vi.fn> }
  items: { findByCollection: ReturnType<typeof vi.fn> }
  assets: { findByItem: ReturnType<typeof vi.fn> }
  notes: { findByItem: ReturnType<typeof vi.fn> }
  extractions: { findByAsset: ReturnType<typeof vi.fn> }
  annotations: { findByAsset: ReturnType<typeof vi.fn> }
  transcriptions: { findByAsset: ReturnType<typeof vi.fn> }
}

type ExportStoreInput = Parameters<typeof exportCollectionById>[0]

function asExportStore(store: ExportStoreMock): ExportStoreInput {
  return store as unknown as ExportStoreInput
}

describe('exportCollectionToJson', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns null when user cancels save dialog', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    vi.mocked(save).mockResolvedValue(null)

    const result = await exportCollectionToJson({ name: 'Test' }, 'test.json')
    expect(result).toBeNull()
  })

  it('writes JSON file and returns path when user selects location', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/exports/test.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const data = { name: 'My Collection', items: [{ id: '1' }] }
    const result = await exportCollectionToJson(data, 'my-collection.json')

    expect(result).toBe('/exports/test.json')
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        defaultPath: 'my-collection.json',
      })
    )
    expect(writeFile).toHaveBeenCalledWith('/exports/test.json', expect.any(Uint8Array))
  })

  it('serializes data as pretty-printed JSON', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/out/data.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const data = { hello: 'world' }
    await exportCollectionToJson(data, 'data.json')

    const writtenBytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    const writtenStr = new TextDecoder().decode(writtenBytes)
    expect(JSON.parse(writtenStr)).toEqual(data)
    // Pretty-printed = has newlines
    expect(writtenStr).toContain('\n')
  })
})

describe('buildCollectionExportData', () => {
  it('builds versioned payload with relative asset paths', () => {
    const collection = {
      id: 'c1',
      name: 'Archivo Municipal',
      description: 'docs',
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'Acta 1', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]

    const payload = buildCollectionExportData(
      collection,
      items,
      {
        i1: [
          {
            id: 'a1',
            itemId: 'i1',
            path: '/mock/app-data/assets/c1/i1/file.pdf',
            type: 'pdf',
            sortIndex: 0,
            size: 123,
            createdAt: 5,
          },
        ],
      },
      {
        i1: [
          {
            id: 'n1',
            itemId: 'i1',
            assetId: null,
            content: 'nota',
            createdAt: 6,
            updatedAt: 7,
          },
        ],
      },
      // extractionsByAssetId
      {
        a1: {
          id: 'e1',
          assetId: 'a1',
          textContent: 'Texto extraído por OCR',
          method: 'ocr',
          confidence: 0.95,
          createdAt: 8,
        },
      },
      // annotationsByAssetId
      {
        a1: [
          {
            id: 'ann1',
            assetId: 'a1',
            page: 1,
            kind: 'rectangle',
            color: '#ff0000',
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
            createdAt: 9,
            updatedAt: 10,
          },
        ],
      }
    )

    expect(payload.version).toBe(3)
    expect(Array.isArray(payload.items)).toBe(true)
    expect(payload.items).toHaveLength(1)
    expect(payload.items[0]?.assets[0]?.path).toBe('assets/c1/i1/file.pdf')
    expect(payload.items[0]?.notes[0]?.content).toBe('nota')
    expect(payload.items[0]?.assets[0]?.text).toBe('Texto extraído por OCR')
    expect(payload.items[0]?.assets[0]?.bboxes).toEqual([
      { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
    ])
  })

  it('sets text to null and bboxes to empty array when no extraction or annotations exist', () => {
    const collection = {
      id: 'c1',
      name: 'Empty',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      {
        id: 'i1',
        title: 'No data',
        collectionId: 'c1',
        metadata: null,
        createdAt: 3,
        updatedAt: 4,
      },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/img.png',
      type: 'image',
      sortIndex: 0,
      size: 50,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {}, // no extractions
      {} // no annotations
    )

    expect(payload.items[0]!.assets[0]!.text).toBeNull()
    expect(payload.items[0]!.assets[0]!.bboxes).toEqual([])
  })

  it('ignores underline annotations for bboxes — only rectangles are collected', () => {
    const collection = {
      id: 'c1',
      name: 'UnderlineOnly',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'UL', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/img.png',
      type: 'image',
      sortIndex: 0,
      size: 50,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {},
      {
        a1: [
          {
            id: 'ann1',
            assetId: 'a1',
            page: 1,
            kind: 'underline',
            color: '#0000ff',
            x: 0.1,
            y: 0.5,
            width: 0.4,
            height: 0.02,
            createdAt: 9,
            updatedAt: 10,
          },
        ],
      }
    )

    expect(payload.items[0]!.assets[0]!.bboxes).toEqual([])
  })

  it('collects ALL rectangle bboxes when multiple annotations exist', () => {
    const collection = {
      id: 'c1',
      name: 'Multi',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'Multi', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/img.png',
      type: 'image',
      sortIndex: 0,
      size: 50,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {},
      {
        a1: [
          {
            id: 'ann1',
            assetId: 'a1',
            page: 1,
            kind: 'underline',
            color: '#0000ff',
            x: 0.1,
            y: 0.5,
            width: 0.4,
            height: 0.02,
            createdAt: 9,
            updatedAt: 10,
          },
          {
            id: 'ann2',
            assetId: 'a1',
            page: 1,
            kind: 'rectangle',
            color: '#ff0000',
            x: 0.2,
            y: 0.3,
            width: 0.5,
            height: 0.6,
            createdAt: 11,
            updatedAt: 12,
          },
          {
            id: 'ann3',
            assetId: 'a1',
            page: 1,
            kind: 'rectangle',
            color: '#00ff00',
            x: 0.7,
            y: 0.8,
            width: 0.1,
            height: 0.1,
            createdAt: 13,
            updatedAt: 14,
          },
        ],
      }
    )

    // ALL rectangles, in order — underline is filtered out
    expect(payload.items[0]!.assets[0]!.bboxes).toEqual([
      { x: 0.2, y: 0.3, width: 0.5, height: 0.6 },
      { x: 0.7, y: 0.8, width: 0.1, height: 0.1 },
    ])
  })

  it('includes text from extraction when asset has OCR data', () => {
    const collection = {
      id: 'c1',
      name: 'WithOCR',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'OCR', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/doc.pdf',
      type: 'pdf',
      sortIndex: 0,
      size: 100,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {
        a1: {
          id: 'e1',
          assetId: 'a1',
          textContent: 'Contenido nativo del PDF',
          method: 'native',
          confidence: null,
          createdAt: 8,
        },
      },
      {}
    )

    expect(payload.items[0]!.assets[0]!.text).toBe('Contenido nativo del PDF')
    expect(payload.items[0]!.assets[0]!.bboxes).toEqual([])
  })

  it('includes transcription text for audio assets when no extraction exists', () => {
    const collection = {
      id: 'c1',
      name: 'WithTranscription',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'Audio', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/recording.wav',
      type: 'audio',
      sortIndex: 0,
      size: 5000,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {}, // no extractions (audio assets have none)
      {}, // no annotations
      {
        a1: {
          id: 't1',
          assetId: 'a1',
          textContent: 'Transcripción del audio grabado',
          language: 'es',
          durationMs: 120000,
          model: 'base',
          segments: null,
          confidence: 0.92,
          createdAt: 8,
        },
      }
    )

    expect(payload.items[0]!.assets[0]!.text).toBe('Transcripción del audio grabado')
  })

  it('prefers extraction text over transcription when both exist', () => {
    const collection = {
      id: 'c1',
      name: 'Both',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'Both', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/doc.pdf',
      type: 'pdf',
      sortIndex: 0,
      size: 100,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {
        a1: {
          id: 'e1',
          assetId: 'a1',
          textContent: 'OCR text',
          method: 'ocr',
          confidence: null,
          createdAt: 8,
        },
      },
      {},
      {
        a1: {
          id: 't1',
          assetId: 'a1',
          textContent: 'Transcription text',
          language: 'es',
          durationMs: 60000,
          model: 'base',
          segments: null,
          confidence: null,
          createdAt: 9,
        },
      }
    )

    expect(payload.items[0]!.assets[0]!.text).toBe('OCR text')
  })

  it('audio asset with no extraction and no transcription has text null', () => {
    const collection = {
      id: 'c1',
      name: 'NoText',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const items = [
      { id: 'i1', title: 'Empty', collectionId: 'c1', metadata: null, createdAt: 3, updatedAt: 4 },
    ]
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/audio.mp3',
      type: 'audio',
      sortIndex: 0,
      size: 2000,
      createdAt: 5,
    }

    const payload = buildCollectionExportData(
      collection,
      items,
      { i1: [asset] },
      { i1: [] },
      {}, // no extractions
      {}, // no annotations
      {} // no transcriptions
    )

    expect(payload.items[0]!.assets[0]!.text).toBeNull()
  })
})

describe('exportCollectionById', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns null when collection does not exist', async () => {
    const store: ExportStoreMock = {
      collections: { findById: vi.fn().mockResolvedValue(null) },
      items: { findByCollection: vi.fn() },
      assets: { findByItem: vi.fn() },
      notes: { findByItem: vi.fn() },
      extractions: { findByAsset: vi.fn() },
      annotations: { findByAsset: vi.fn() },
      transcriptions: { findByAsset: vi.fn() },
    }

    const result = await exportCollectionById(asExportStore(store), 'missing')
    expect(result).toBeNull()
  })

  it('exports full collection payload with default filename', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/exports/Archivo Municipal.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const collection = {
      id: 'c1',
      name: 'Archivo Municipal',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const item = {
      id: 'i1',
      title: 'Acta',
      collectionId: 'c1',
      metadata: null,
      createdAt: 3,
      updatedAt: 4,
    }
    const asset = {
      id: 'a1',
      itemId: 'i1',
      path: '/mock/app-data/assets/c1/i1/acta.pdf',
      type: 'pdf',
      sortIndex: 0,
      size: 10,
      createdAt: 5,
    }
    const extraction = {
      id: 'e1',
      assetId: 'a1',
      textContent: 'Texto OCR',
      method: 'ocr',
      confidence: 0.9,
      createdAt: 8,
    }
    const annotation = {
      id: 'ann1',
      assetId: 'a1',
      page: 1,
      kind: 'rectangle' as const,
      color: '#ff0000',
      x: 0.1,
      y: 0.2,
      width: 0.3,
      height: 0.4,
      createdAt: 9,
      updatedAt: 10,
    }

    const store: ExportStoreMock = {
      collections: { findById: vi.fn().mockResolvedValue(collection) },
      items: { findByCollection: vi.fn().mockResolvedValue([item]) },
      assets: { findByItem: vi.fn().mockResolvedValue([asset]) },
      notes: {
        findByItem: vi
          .fn()
          .mockResolvedValue([
            { id: 'n1', itemId: 'i1', assetId: null, content: 'nota', createdAt: 6, updatedAt: 7 },
          ]),
      },
      extractions: { findByAsset: vi.fn().mockResolvedValue(extraction) },
      annotations: { findByAsset: vi.fn().mockResolvedValue([annotation]) },
      transcriptions: { findByAsset: vi.fn().mockResolvedValue(null) },
    }

    const result = await exportCollectionById(asExportStore(store), 'c1')

    expect(result?.paths).toEqual(['/exports/Archivo Municipal.json'])
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({
        defaultPath: 'Archivo Municipal.json',
      })
    )
    expect(writeFile).toHaveBeenCalledTimes(1)

    // Verify the written JSON contains the new fields
    const writtenBytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    const writtenStr = new TextDecoder().decode(writtenBytes)
    const parsed = JSON.parse(writtenStr)

    expect(parsed.version).toBe(3)
    expect(parsed.items[0].assets[0].text).toBe('Texto OCR')
    expect(parsed.items[0].assets[0].bboxes).toEqual([{ x: 0.1, y: 0.2, width: 0.3, height: 0.4 }])
  })

  it('queries extractions and annotations for each asset', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/out/export.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const collection = {
      id: 'c1',
      name: 'Test',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const item = {
      id: 'i1',
      title: 'Doc',
      collectionId: 'c1',
      metadata: null,
      createdAt: 3,
      updatedAt: 4,
    }
    const asset1 = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/img1.png',
      type: 'image',
      sortIndex: 0,
      size: 50,
      createdAt: 5,
    }
    const asset2 = {
      id: 'a2',
      itemId: 'i1',
      path: '/app/assets/c1/i1/img2.png',
      type: 'image',
      sortIndex: 1,
      size: 60,
      createdAt: 6,
    }

    const store: ExportStoreMock = {
      collections: { findById: vi.fn().mockResolvedValue(collection) },
      items: { findByCollection: vi.fn().mockResolvedValue([item]) },
      assets: { findByItem: vi.fn().mockResolvedValue([asset1, asset2]) },
      notes: { findByItem: vi.fn().mockResolvedValue([]) },
      extractions: {
        findByAsset: vi
          .fn()
          .mockResolvedValueOnce({
            id: 'e1',
            assetId: 'a1',
            textContent: 'OCR 1',
            method: 'ocr',
            confidence: 0.8,
            createdAt: 7,
          })
          .mockResolvedValueOnce(null),
      },
      annotations: {
        findByAsset: vi
          .fn()
          .mockResolvedValueOnce([])
          .mockResolvedValueOnce([
            {
              id: 'ann2',
              assetId: 'a2',
              page: 1,
              kind: 'rectangle',
              color: '#00ff00',
              x: 0.5,
              y: 0.5,
              width: 0.2,
              height: 0.2,
              createdAt: 9,
              updatedAt: 10,
            },
          ]),
      },
      transcriptions: {
        findByAsset: vi.fn().mockResolvedValue(null),
      },
    }

    await exportCollectionById(asExportStore(store), 'c1')

    expect(store.extractions.findByAsset).toHaveBeenCalledWith('a1')
    expect(store.extractions.findByAsset).toHaveBeenCalledWith('a2')
    expect(store.annotations.findByAsset).toHaveBeenCalledWith('a1')
    expect(store.annotations.findByAsset).toHaveBeenCalledWith('a2')
    expect(store.transcriptions.findByAsset).toHaveBeenCalledWith('a1')
    expect(store.transcriptions.findByAsset).toHaveBeenCalledWith('a2')

    const writtenBytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    const parsed = JSON.parse(new TextDecoder().decode(writtenBytes))

    // asset1 has text but no bboxes
    expect(parsed.items[0].assets[0].text).toBe('OCR 1')
    expect(parsed.items[0].assets[0].bboxes).toEqual([])
    // asset2 has bboxes but no text
    expect(parsed.items[0].assets[1].text).toBeNull()
    expect(parsed.items[0].assets[1].bboxes).toEqual([{ x: 0.5, y: 0.5, width: 0.2, height: 0.2 }])
  })

  it('exports audio asset with transcription text', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/out/audio-export.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const collection = {
      id: 'c1',
      name: 'Entrevistas',
      description: null,
      createdAt: 1,
      updatedAt: 2,
    }
    const item = {
      id: 'i1',
      title: 'Entrevista 1',
      collectionId: 'c1',
      metadata: null,
      createdAt: 3,
      updatedAt: 4,
    }
    const audioAsset = {
      id: 'a1',
      itemId: 'i1',
      path: '/app/assets/c1/i1/interview.wav',
      type: 'audio',
      sortIndex: 0,
      size: 5000,
      createdAt: 5,
    }

    const store: ExportStoreMock = {
      collections: { findById: vi.fn().mockResolvedValue(collection) },
      items: { findByCollection: vi.fn().mockResolvedValue([item]) },
      assets: { findByItem: vi.fn().mockResolvedValue([audioAsset]) },
      notes: { findByItem: vi.fn().mockResolvedValue([]) },
      extractions: { findByAsset: vi.fn().mockResolvedValue(null) },
      annotations: { findByAsset: vi.fn().mockResolvedValue([]) },
      transcriptions: {
        findByAsset: vi.fn().mockResolvedValue({
          id: 't1',
          assetId: 'a1',
          textContent: 'Buenos días, hoy vamos a hablar sobre la historia colonial',
          language: 'es',
          durationMs: 120000,
          model: 'base',
          segments: null,
          confidence: 0.91,
          createdAt: 8,
        }),
      },
    }

    await exportCollectionById(asExportStore(store), 'c1')

    const writtenBytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    const parsed = JSON.parse(new TextDecoder().decode(writtenBytes))

    expect(parsed.items[0].assets[0].type).toBe('audio')
    expect(parsed.items[0].assets[0].text).toBe(
      'Buenos días, hoy vamos a hablar sobre la historia colonial'
    )
    expect(store.transcriptions.findByAsset).toHaveBeenCalledWith('a1')
  })
})

// ---------------------------------------------------------------------------
// Embedded asset bytes (the images that cross the bridge to the web app)
// ---------------------------------------------------------------------------

const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x01, 0x02])

type EmbedAsset = {
  id: string
  itemId: string
  path: string
  type: string
  sortIndex: number
  size: number | null
  createdAt: number
  parentAssetId?: string | null
}

function buildEmbedStore(assets: EmbedAsset[]): ExportStoreMock {
  return {
    collections: {
      findById: vi.fn().mockResolvedValue({
        id: 'c1',
        name: 'Archivo',
        description: null,
        createdAt: 1,
        updatedAt: 2,
      }),
    },
    items: {
      findByCollection: vi.fn().mockResolvedValue([
        {
          id: 'i1',
          title: 'Acta',
          collectionId: 'c1',
          metadata: null,
          createdAt: 3,
          updatedAt: 4,
        },
      ]),
    },
    assets: { findByItem: vi.fn().mockResolvedValue(assets) },
    notes: { findByItem: vi.fn().mockResolvedValue([]) },
    extractions: { findByAsset: vi.fn().mockResolvedValue(null) },
    annotations: { findByAsset: vi.fn().mockResolvedValue([]) },
    transcriptions: { findByAsset: vi.fn().mockResolvedValue(null) },
  }
}

async function writtenExportPayload(): Promise<Record<string, any>> {
  const { writeFile } = await import('@tauri-apps/plugin-fs')
  const bytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
  return JSON.parse(new TextDecoder().decode(bytes))
}

const imageAsset: EmbedAsset = {
  id: 'a1',
  itemId: 'i1',
  path: '/mock/app-data/assets/c1/i1/carta-1913-p1.jpg',
  type: 'image',
  sortIndex: 0,
  size: 10,
  createdAt: 5,
}

describe('exportCollectionById — embedded assets', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile, readFile, stat } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/exports/Archivo.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)
    vi.mocked(readFile).mockResolvedValue(PNG_BYTES)
    vi.mocked(stat).mockResolvedValue({ size: 10 } as never)
  })

  it('leaves assets without data when the option is off', async () => {
    const store = buildEmbedStore([imageAsset])

    const result = await exportCollectionById(asExportStore(store), 'c1')

    const payload = await writtenExportPayload()
    expect(payload.items[0].assets[0]).not.toHaveProperty('data')
    expect(result?.embeddedAssetCount).toBe(0)
  })

  it('leaves out the part number when the whole collection fits in one file', async () => {
    const store = buildEmbedStore([imageAsset])

    await exportCollectionById(asExportStore(store), 'c1', { includeAssetData: true })

    const payload = await writtenExportPayload()
    expect(payload).not.toHaveProperty('part')
  })

  it('embeds the file bytes as base64 when the option is on', async () => {
    const store = buildEmbedStore([imageAsset])

    const result = await exportCollectionById(asExportStore(store), 'c1', {
      includeAssetData: true,
    })

    const payload = await writtenExportPayload()
    const exported = payload.items[0].assets[0]
    expect(exported.filename).toBe('carta-1913-p1.jpg')
    expect(exported.data).toBe(bytesToBase64(PNG_BYTES))
    expect(exported.data.startsWith('data:')).toBe(false)
    expect(result?.embeddedAssetCount).toBe(1)
  })

  it('never embeds audio — its transcription already travels in text', async () => {
    const store = buildEmbedStore([
      imageAsset,
      {
        id: 'a2',
        itemId: 'i1',
        path: '/mock/app-data/assets/c1/i1/entrevista.wav',
        type: 'audio',
        sortIndex: 1,
        size: 900,
        createdAt: 6,
      },
    ])

    await exportCollectionById(asExportStore(store), 'c1', { includeAssetData: true })

    const payload = await writtenExportPayload()
    const audio = payload.items[0].assets.find((a: { type: string }) => a.type === 'audio')
    expect(audio).not.toHaveProperty('data')
  })

  it('skips the parent PDF when its pages carry the same bytes', async () => {
    const store = buildEmbedStore([
      {
        id: 'parent',
        itemId: 'i1',
        path: '/mock/app-data/assets/c1/i1/expediente.pdf',
        type: 'pdf',
        sortIndex: 0,
        size: 100,
        createdAt: 5,
      },
      {
        id: 'page1',
        itemId: 'i1',
        path: '/mock/app-data/assets/c1/i1/expediente-p1.pdf',
        type: 'pdf',
        sortIndex: 0,
        size: 50,
        createdAt: 6,
        parentAssetId: 'parent',
      },
    ])

    const result = await exportCollectionById(asExportStore(store), 'c1', {
      includeAssetData: true,
    })

    const payload = await writtenExportPayload()
    const byId = Object.fromEntries(
      payload.items[0].assets.map((asset: { id: string }) => [asset.id, asset])
    )
    expect(byId.parent).not.toHaveProperty('data')
    expect(byId.page1.data).toBe(bytesToBase64(PNG_BYTES))
    expect(result?.embeddedAssetCount).toBe(1)
  })

  it('exports the rest of the collection when one file cannot be read', async () => {
    const { readFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(readFile).mockRejectedValueOnce(new Error('file missing'))
    const store = buildEmbedStore([imageAsset])

    const result = await exportCollectionById(asExportStore(store), 'c1', {
      includeAssetData: true,
    })

    expect(result?.paths).toEqual(['/exports/Archivo.json'])
    expect(result?.skippedAssetIds).toEqual(['a1'])
    const payload = await writtenExportPayload()
    expect(payload.items[0].assets[0]).not.toHaveProperty('data')
  })

  it('recovers a missing filename extension from the bytes', async () => {
    const store = buildEmbedStore([{ ...imageAsset, path: '/mock/app-data/assets/c1/i1/escaneo' }])

    await exportCollectionById(asExportStore(store), 'c1', { includeAssetData: true })

    const payload = await writtenExportPayload()
    expect(payload.items[0].assets[0].filename).toBe('escaneo.png')
  })

  it('splits into numbered files when the collection busts the ceiling', async () => {
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    const halfLimit = Math.round((WEB_UPLOAD_LIMIT_BYTES * 0.6) / (4 / 3))
    const store = buildEmbedStore([imageAsset])
    // Three documents, each ~60% of the ceiling: two files, then one.
    store.items.findByCollection.mockResolvedValue(
      ['i1', 'i2', 'i3'].map((id) => ({
        id,
        title: `Acta ${id}`,
        collectionId: 'c1',
        metadata: null,
        createdAt: 3,
        updatedAt: 4,
      }))
    )
    store.assets.findByItem.mockImplementation((itemId: string) =>
      Promise.resolve([{ ...imageAsset, id: `a-${itemId}`, itemId, size: halfLimit }])
    )

    const result = await exportCollectionById(asExportStore(store), 'c1', {
      includeAssetData: true,
    })

    expect(result?.paths).toEqual([
      '/exports/Archivo (1 de 3).json',
      '/exports/Archivo (2 de 3).json',
      '/exports/Archivo (3 de 3).json',
    ])
    expect(writeFile).toHaveBeenCalledTimes(3)
    // Every document travels exactly once, and each file stands on its own.
    const written = await Promise.all(
      [0, 1, 2].map(async (index) => {
        const bytes = vi.mocked(writeFile).mock.calls[index]![1] as Uint8Array
        return JSON.parse(new TextDecoder().decode(bytes))
      })
    )
    expect(written.flatMap((file) => file.items.map((item: { id: string }) => item.id))).toEqual([
      'i1',
      'i2',
      'i3',
    ])
    // Numbered so the web app can tell the three files belong together.
    expect(written.map((file) => file.part)).toEqual([
      { index: 1, total: 3 },
      { index: 2, total: 3 },
      { index: 3, total: 3 },
    ])
    expect(written.every((file) => file.collection.id === 'c1')).toBe(true)
  })

  it('writes nothing when the user cancels the save dialog', async () => {
    const { save, writeFile } = await import('@tauri-apps/plugin-dialog').then(async (dialog) => ({
      save: dialog.save,
      writeFile: (await import('@tauri-apps/plugin-fs')).writeFile,
    }))
    vi.mocked(save).mockResolvedValue(null)
    const store = buildEmbedStore([imageAsset])

    const result = await exportCollectionById(asExportStore(store), 'c1')

    expect(result?.paths).toEqual([])
    expect(writeFile).not.toHaveBeenCalled()
  })
})

describe('planExportBatches', () => {
  it('packs documents together until the next one would bust the ceiling', () => {
    const batches = planExportBatches(['a', 'b', 'c', 'd'], { a: 40, b: 40, c: 30, d: 10 }, 100)

    expect(batches.map((batch) => batch.itemIds)).toEqual([
      ['a', 'b'],
      ['c', 'd'],
    ])
    expect(batches.every((batch) => batch.encodedBytes <= 100)).toBe(true)
  })

  it('gives a document heavier than the ceiling its own flagged file', () => {
    const batches = planExportBatches(['a', 'huge', 'b'], { a: 10, huge: 500, b: 10 }, 100)

    expect(batches).toEqual([
      { itemIds: ['a'], encodedBytes: 10, exceedsWebLimit: false },
      { itemIds: ['huge'], encodedBytes: 500, exceedsWebLimit: true },
      { itemIds: ['b'], encodedBytes: 10, exceedsWebLimit: false },
    ])
  })

  it('always returns one batch, even for an empty collection', () => {
    expect(planExportBatches([], {}, 100)).toEqual([
      { itemIds: [], encodedBytes: 0, exceedsWebLimit: false },
    ])
  })
})

describe('estimateCollectionEmbeddedAssets', () => {
  beforeEach(async () => {
    vi.clearAllMocks()
    const { stat } = await import('@tauri-apps/plugin-fs')
    vi.mocked(stat).mockResolvedValue({ size: 0 } as never)
  })

  it('counts only the assets that would carry bytes and flags the web limit', async () => {
    const store = buildEmbedStore([
      {
        id: 'a1',
        itemId: 'i1',
        path: '/mock/a.jpg',
        type: 'image',
        sortIndex: 0,
        size: 60 * 1024 * 1024,
        createdAt: 1,
      },
      {
        id: 'a2',
        itemId: 'i1',
        path: '/mock/b.wav',
        type: 'audio',
        sortIndex: 1,
        size: 90 * 1024 * 1024,
        createdAt: 2,
      },
    ])

    const estimate = await estimateCollectionEmbeddedAssets(asExportStore(store), 'c1')

    expect(estimate.assetCount).toBe(1)
    expect(estimate.bytes).toBe(60 * 1024 * 1024)
    expect(estimate.encodedBytes).toBeGreaterThan(estimate.bytes)
    // 60 MB of pictures encode to ~80 MB, past the 64 MB ceiling, and the whole
    // weight sits on one document — it cannot be split.
    expect(estimate.oversizedItemIds).toEqual(['i1'])
  })

  it('falls back to the file on disk when the stored size is missing', async () => {
    const { stat } = await import('@tauri-apps/plugin-fs')
    vi.mocked(stat).mockResolvedValue({ size: 4096 } as never)
    const store = buildEmbedStore([
      {
        id: 'a1',
        itemId: 'i1',
        path: '/mock/a.jpg',
        type: 'image',
        sortIndex: 0,
        size: null,
        createdAt: 1,
      },
    ])

    const estimate = await estimateCollectionEmbeddedAssets(asExportStore(store), 'c1')

    expect(stat).toHaveBeenCalledWith('/mock/a.jpg')
    expect(estimate.bytes).toBe(4096)
  })
})

// ---------------------------------------------------------------------------
// AI credentials (the key that saves setting the web app up again)
// ---------------------------------------------------------------------------

describe('loadExportAiConfig', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('returns null when no key is configured, so nothing can carry one', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(invoke).mockResolvedValue(null)

    expect(await loadExportAiConfig('Mi llave')).toBeNull()
  })

  it('returns null for a key that is only whitespace', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(invoke).mockResolvedValue('   ')

    expect(await loadExportAiConfig('Mi llave')).toBeNull()
  })

  it('spells the provider and base address exactly as the web app expects', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
      if (command !== 'settings_get') return Promise.resolve(null)
      if ((args as Record<string, unknown> | undefined)?.key === 'openrouter_api_key')
        return Promise.resolve('sk-or-v1-secreta')
      if ((args as Record<string, unknown> | undefined)?.key === 'openrouter_model')
        return Promise.resolve('google/gemma-4-26b-a4b-it')
      return Promise.resolve(null)
    })

    expect(await loadExportAiConfig('Mi llave')).toEqual({
      provider: 'openrouter',
      apiKey: 'sk-or-v1-secreta',
      model: 'google/gemma-4-26b-a4b-it',
      baseUrl: 'https://openrouter.ai/api/v1',
      name: 'Mi llave',
    })
  })

  it('falls back to the default model when none was chosen', async () => {
    const { invoke } = await import('@tauri-apps/api/core')
    vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
      if (command !== 'settings_get') return Promise.resolve(null)
      if ((args as Record<string, unknown> | undefined)?.key === 'openrouter_api_key')
        return Promise.resolve('sk-or-v1-secreta')
      return Promise.resolve('')
    })

    const config = await loadExportAiConfig('Mi llave')
    expect(config?.model).toBe(DEFAULT_OPENROUTER_MODEL)
  })
})

describe('exportCollectionById — AI credentials', () => {
  const aiConfig = {
    provider: 'openrouter',
    apiKey: 'sk-or-v1-secreta',
    model: 'google/gemma-4-26b-a4b-it',
    baseUrl: 'https://openrouter.ai/api/v1',
    name: 'Mi llave de OpenRouter',
  }

  beforeEach(async () => {
    vi.clearAllMocks()
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue('/exports/Archivo.json')
    vi.mocked(writeFile).mockResolvedValue(undefined)
  })

  it('leaves the key out unless it is handed in for this export', async () => {
    const store = buildEmbedStore([imageAsset])

    await exportCollectionById(asExportStore(store), 'c1')

    const payload = await writtenExportPayload()
    expect(payload).not.toHaveProperty('aiConfig')
  })

  it('writes the key block and warns in the filename when asked', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const store = buildEmbedStore([imageAsset])

    await exportCollectionById(asExportStore(store), 'c1', {
      aiConfig,
      aiConfigFilenameMarker: 'con llave',
    })

    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ defaultPath: 'Archivo (con llave).json' })
    )
    const payload = await writtenExportPayload()
    expect(payload.aiConfig).toEqual(aiConfig)
  })

  it('keeps the key in every file when the export is split', async () => {
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    const { readFile, stat } = await import('@tauri-apps/plugin-fs')
    vi.mocked(readFile).mockResolvedValue(PNG_BYTES)
    vi.mocked(stat).mockResolvedValue({ size: 10 } as never)
    const halfLimit = Math.round((WEB_UPLOAD_LIMIT_BYTES * 0.6) / (4 / 3))
    const store = buildEmbedStore([imageAsset])
    store.items.findByCollection.mockResolvedValue(
      ['i1', 'i2'].map((id) => ({
        id,
        title: `Acta ${id}`,
        collectionId: 'c1',
        metadata: null,
        createdAt: 3,
        updatedAt: 4,
      }))
    )
    store.assets.findByItem.mockImplementation((itemId: string) =>
      Promise.resolve([{ ...imageAsset, id: `a-${itemId}`, itemId, size: halfLimit }])
    )

    await exportCollectionById(asExportStore(store), 'c1', {
      includeAssetData: true,
      aiConfig,
      aiConfigFilenameMarker: 'con llave',
    })

    expect(writeFile).toHaveBeenCalledTimes(2)
    for (const index of [0, 1]) {
      const bytes = vi.mocked(writeFile).mock.calls[index]![1] as Uint8Array
      const file = JSON.parse(new TextDecoder().decode(bytes))
      expect(file.aiConfig.apiKey).toBe('sk-or-v1-secreta')
    }
  })
})
