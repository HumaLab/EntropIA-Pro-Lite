import { describe, it, expect, vi, beforeEach } from 'vitest'
import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import {
  exportCollectionToJson,
  exportCollectionToCsv,
  buildCollectionExportData,
  exportCollectionById,
} from './export'

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

describe('exportCollectionToCsv', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('does not write a file when the user cancels the save dialog', async () => {
    vi.mocked(save).mockResolvedValue(null)

    const result = await exportCollectionToCsv([{ id: '1' }], ['id'], 'documents.csv')

    expect(result).toBeNull()
    expect(writeFile).not.toHaveBeenCalled()
  })

  it('writes an Excel-compatible semicolon CSV with schema-ordered columns', async () => {
    vi.mocked(save).mockResolvedValue('/exports/documents.csv')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const result = await exportCollectionToCsv(
      [
        {
          name: 'Álvarez',
          note: 'Uno; "dos"\r\ntres',
          details: { page: 2 },
          tags: ['acta', '2026'],
          missing: null,
          active: true,
          count: 42,
        },
      ],
      ['name', 'note', 'details', 'tags', 'missing', 'active', 'count'],
      'documents.csv'
    )

    expect(result).toBe('/exports/documents.csv')
    expect(save).toHaveBeenCalledWith({
      defaultPath: 'documents.csv',
      filters: [{ name: 'CSV', extensions: ['csv'] }],
    })

    const writtenBytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    expect([...writtenBytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf])
    expect(new TextDecoder().decode(writtenBytes.slice(3))).toBe(
      'name;note;details;tags;missing;active;count\r\n' +
        'Álvarez;"Uno; ""dos""\r\ntres";"{""page"":2}";"[""acta"",""2026""]";;true;42\r\n'
    )
  })

  it('writes a header-only CSV when the table has no rows', async () => {
    vi.mocked(save).mockResolvedValue('/exports/empty.csv')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    await exportCollectionToCsv([], ['second', 'first'], 'empty.csv')

    const writtenBytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    expect(new TextDecoder().decode(writtenBytes.slice(3))).toBe('second;first\r\n')
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

    const path = await exportCollectionById(asExportStore(store), 'c1')

    expect(path).toBe('/exports/Archivo Municipal.json')
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
