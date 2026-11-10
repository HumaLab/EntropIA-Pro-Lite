import { strFromU8, unzipSync } from 'fflate'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import type { StoreApi } from '@entropia/store'

import {
  CollectionOcrExportEmptyError,
  buildCollectionOcrExportBytes,
  exportCollectionOcrText,
  loadCollectionOcrDocuments,
  type CollectionOcrDocument,
} from './collection-ocr-export'

vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: vi.fn() }))

// ignoreBOM keeps a leading byte order mark visible, so the CSV test can pin it.
const decode = (bytes: Uint8Array) => new TextDecoder('utf-8', { ignoreBOM: true }).decode(bytes)

const DOCUMENTS: CollectionOcrDocument[] = [
  {
    itemTitle: 'Carta de 1890',
    fileName: 'carta.pdf',
    native: false,
    source: '# Encabezado\n\nPrimera línea; con "comillas"\n\n![](page=1,bbox=[1,2,3,4])',
  },
  { itemTitle: 'Informe', fileName: 'informe.pdf', native: true, source: 'Texto nativo.' },
]

interface FakeData {
  collection?: { id: string; name: string }
  items: Array<{ id: string; title: string }>
  assets: Record<string, Array<{ id: string; path: string; type: string }>>
  extractions: Record<string, { textContent: string; method: string } | null>
}

function fakeStore(data: FakeData): StoreApi {
  return {
    collections: { findById: vi.fn(async () => data.collection ?? null) },
    items: { findByCollection: vi.fn(async () => data.items) },
    assets: { findByItem: vi.fn(async (itemId: string) => data.assets[itemId] ?? []) },
    extractions: {
      findByAsset: vi.fn(async (assetId: string) => data.extractions[assetId] ?? null),
    },
  } as unknown as StoreApi
}

const COLLECTION_DATA: FakeData = {
  collection: { id: 'c1', name: 'Archivo/Municipal' },
  items: [
    { id: 'i1', title: 'Carta de 1890' },
    { id: 'i2', title: 'Sin texto' },
  ],
  assets: {
    i1: [
      { id: 'a1', path: 'C:\\data\\0a1b2c3d-1111-2222-3333-444455556666_carta.pdf', type: 'pdf' },
      { id: 'a2', path: 'C:\\data\\entrevista.mp3', type: 'audio' },
      { id: 'a3', path: 'C:\\data\\informe.pdf', type: 'pdf' },
    ],
    i2: [{ id: 'a4', path: 'C:\\data\\vacio.pdf', type: 'pdf' }],
  },
  extractions: {
    a1: { textContent: 'Texto OCR', method: 'glm_ocr' },
    a2: { textContent: 'audio', method: 'ocr' },
    a3: { textContent: 'Texto nativo', method: 'native' },
    a4: { textContent: '   ', method: 'ocr' },
  },
}

afterEach(() => {
  vi.mocked(save).mockReset()
  vi.mocked(writeFile).mockReset()
})

describe('loadCollectionOcrDocuments', () => {
  it('lists each non-audio asset with text in collection order, marking native text', async () => {
    const documents = await loadCollectionOcrDocuments(fakeStore(COLLECTION_DATA), 'c1')

    expect(documents).toEqual([
      { itemTitle: 'Carta de 1890', fileName: 'carta.pdf', native: false, source: 'Texto OCR' },
      { itemTitle: 'Carta de 1890', fileName: 'informe.pdf', native: true, source: 'Texto nativo' },
    ])
  })
})

describe('buildCollectionOcrExportBytes', () => {
  it('builds Markdown with the collection title, one section per document and image placeholders', async () => {
    const text = decode(await buildCollectionOcrExportBytes('Archivo', DOCUMENTS, 'markdown'))

    expect(text).toBe(
      [
        '# Archivo',
        '',
        '## Carta de 1890',
        '',
        '*carta.pdf*',
        '',
        '# Encabezado\n\nPrimera línea; con "comillas"\n\n[Imagen, página 2]',
        '',
        '## Informe',
        '',
        '*informe.pdf (texto nativo del PDF)*',
        '',
        'Texto nativo.',
        '',
      ].join('\n')
    )
  })

  it('builds plain text with a separator per document', async () => {
    const text = decode(await buildCollectionOcrExportBytes('Archivo', DOCUMENTS, 'txt'))

    expect(text).toBe(
      [
        'Archivo',
        '',
        '==================================================',
        'Carta de 1890 · carta.pdf',
        '==================================================',
        '',
        '# Encabezado\n\nPrimera línea; con "comillas"\n\n[Imagen, página 2]',
        '',
        '==================================================',
        'Informe · informe.pdf (texto nativo del PDF)',
        '==================================================',
        '',
        'Texto nativo.',
        '',
      ].join('\n')
    )
  })

  it('builds an Excel-compatible CSV with one row per document', async () => {
    const text = decode(await buildCollectionOcrExportBytes('Archivo', DOCUMENTS, 'csv'))

    expect(text).toBe(
      '\uFEFFdocumento;archivo;origen;texto\r\n' +
        'Carta de 1890;carta.pdf;ocr;"# Encabezado\n\nPrimera línea; con ""comillas""\n\n[Imagen, página 2]"\r\n' +
        'Informe;informe.pdf;nativo;Texto nativo.\r\n'
    )
  })

  it('renders PDF and DOCX from one escaped HTML document', async () => {
    const pdf = vi.fn(async (_html: string) => new Uint8Array([1]))
    const docx = vi.fn(async (_html: string) => new Uint8Array([2]))
    const titled = [{ ...DOCUMENTS[0]!, itemTitle: 'A <b>' }]

    expect(await buildCollectionOcrExportBytes('C & D', titled, 'pdf', { pdf, docx })).toEqual(
      new Uint8Array([1])
    )
    expect(await buildCollectionOcrExportBytes('C & D', titled, 'docx', { pdf, docx })).toEqual(
      new Uint8Array([2])
    )

    const html = pdf.mock.calls[0]![0]
    expect(docx.mock.calls[0]![0]).toBe(html)
    expect(html).toContain('<h1>C &amp; D</h1>')
    expect(html).toContain('<h2>A &lt;b&gt;</h2>')
    expect(html).toContain('<p><em>carta.pdf</em></p>')
    expect(html).toContain('[Imagen, página 2]')
    expect(html).not.toContain('page=1,bbox')
  })

  it('produces a real DOCX archive with the default generator', async () => {
    const bytes = await buildCollectionOcrExportBytes('Archivo', DOCUMENTS, 'docx')
    const documentXml = strFromU8(unzipSync(bytes)['word/document.xml']!)

    expect(documentXml).toContain('Carta de 1890')
    expect(documentXml).toContain('Texto nativo.')
  })
})

describe('exportCollectionOcrText', () => {
  it('asks where to save with the format filter and writes the file', async () => {
    vi.mocked(save).mockResolvedValue('C:\\out\\archivo.md')

    const path = await exportCollectionOcrText(fakeStore(COLLECTION_DATA), 'c1', 'markdown')

    expect(path).toBe('C:\\out\\archivo.md')
    expect(save).toHaveBeenCalledWith({
      defaultPath: 'Archivo-Municipal-texto.md',
      filters: [{ name: 'Markdown', extensions: ['md'] }],
    })
    const [writtenPath, bytes] = vi.mocked(writeFile).mock.calls[0]!
    expect(writtenPath).toBe('C:\\out\\archivo.md')
    expect(decode(bytes as Uint8Array)).toContain('## Carta de 1890')
  })

  it('writes nothing when the save dialog is cancelled', async () => {
    vi.mocked(save).mockResolvedValue(null)

    expect(await exportCollectionOcrText(fakeStore(COLLECTION_DATA), 'c1', 'txt')).toBeNull()
    expect(writeFile).not.toHaveBeenCalled()
  })

  it('refuses a collection without any extracted text before opening the dialog', async () => {
    const store = fakeStore({ ...COLLECTION_DATA, extractions: {} })

    await expect(exportCollectionOcrText(store, 'c1', 'pdf')).rejects.toBeInstanceOf(
      CollectionOcrExportEmptyError
    )
    expect(save).not.toHaveBeenCalled()
    expect(writeFile).not.toHaveBeenCalled()
  })

  it('returns null for an unknown collection', async () => {
    const store = fakeStore({ ...COLLECTION_DATA, collection: undefined })

    expect(await exportCollectionOcrText(store, 'missing', 'csv')).toBeNull()
    expect(save).not.toHaveBeenCalled()
  })
})
