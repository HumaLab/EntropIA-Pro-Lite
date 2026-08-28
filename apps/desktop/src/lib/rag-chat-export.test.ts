import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { RagConversation, RagSource } from './rag'
import { downloadDir, join } from '@tauri-apps/api/path'
import { writeFile } from '@tauri-apps/plugin-fs'

const { getConversationMock, generatePdfMock } = vi.hoisted(() => ({
  getConversationMock: vi.fn(),
  generatePdfMock: vi.fn(),
}))

vi.mock('./rag', () => ({ ragGetConversation: getConversationMock }))
vi.mock('./ocr-pdf', () => ({ generateNativeOcrPdfBytes: generatePdfMock }))
vi.mock('@tauri-apps/api/path', () => ({
  downloadDir: vi.fn(),
  join: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: vi.fn() }))

const conversation: RagConversation = {
  id: 'conv-42',
  title: 'Acta <sindical>',
  messages: [
    { id: 'm1', role: 'user', content: 'Pregunta\ncon dos líneas', sources: [], createdAt: 1 },
    { id: 'm2', role: 'assistant', content: '**Respuesta** con `formato`', sources: [], createdAt: 2 },
  ],
}

beforeEach(() => {
  vi.resetModules()
  vi.mocked(getConversationMock).mockReset()
  vi.mocked(generatePdfMock).mockReset()
  vi.mocked(downloadDir).mockReset()
  vi.mocked(join).mockReset()
  vi.mocked(writeFile).mockReset()
})

describe('buildRagConversationPdfHtml', () => {
  it('escapes title and questions, renders answers, and preserves message order', async () => {
    const { buildRagConversationPdfHtml } = await import('./rag-chat-export')
    const html = buildRagConversationPdfHtml(conversation)

    expect(html).toContain('<h1>Acta &lt;sindical&gt;</h1>')
    expect(html).toContain('Pregunta<br>con dos líneas')
    expect(html).toContain('<strong>Respuesta</strong>')
    expect(html).toContain('<code>formato</code>')
    expect(html.indexOf('Pregunta')).toBeLessThan(html.indexOf('Respuesta'))
    expect(html).not.toContain('<sindical>')
    expect(html).not.toContain('<h2>Fuentes</h2>')
  })

  it('renders assistant source titles after answers without source metadata', async () => {
    const { buildRagConversationPdfHtml } = await import('./rag-chat-export')
    const userSource: RagSource = {
      index: 99,
      assetId: 'user-asset-sentinel',
      itemId: 'user-item-sentinel',
      itemTitle: 'Usuario source title sentinel',
      collectionId: 'user-collection-id-sentinel',
      collectionName: 'user-collection-sentinel',
      snippet: 'user-snippet-sentinel',
      score: 0.9999,
      startSeconds: 9991,
      endSeconds: 9992,
      provenance: {
        retrievalUnit: 'user-retrieval-sentinel',
        sourceKind: 'user-source-kind-sentinel',
        sourceId: 'user-source-id-sentinel',
        chunkIds: ['user-chunk-sentinel'],
        startChar: 9993,
        endChar: 9994,
      },
    }
    const assistantSources: RagSource[] = [
      {
        index: 1,
        assetId: 'asset-sentinel-one',
        itemId: 'item-sentinel-one',
        itemTitle: 'Archivo <uno>',
        collectionId: 'collection-id-sentinel-one',
        collectionName: 'collection-sentinel-one',
        snippet: 'snippet-sentinel-one',
        score: 0.1111,
        startSeconds: 1111,
        endSeconds: 1112,
        provenance: {
          retrievalUnit: 'retrieval-sentinel-one',
          sourceKind: 'source-kind-sentinel-one',
          sourceId: 'source-id-sentinel-one',
          chunkIds: ['chunk-sentinel-one'],
          startChar: 1113,
          endChar: 1114,
        },
      },
      {
        index: 2,
        assetId: 'asset-sentinel-two',
        itemId: 'item-sentinel-two',
        itemTitle: 'Archivo & dos',
        collectionId: 'collection-id-sentinel-two',
        collectionName: 'collection-sentinel-two',
        snippet: 'snippet-sentinel-two',
        score: 0.2222,
        startSeconds: 2221,
        endSeconds: 2222,
        provenance: {
          retrievalUnit: 'retrieval-sentinel-two',
          sourceKind: 'source-kind-sentinel-two',
          sourceId: 'source-id-sentinel-two',
          chunkIds: ['chunk-sentinel-two'],
          startChar: 2223,
          endChar: 2224,
        },
      },
    ]
    const html = buildRagConversationPdfHtml({
      ...conversation,
      messages: [
        { ...conversation.messages[0]!, sources: [userSource] },
        { ...conversation.messages[1]!, sources: assistantSources },
      ],
    })

    expect(html).toContain('<h2>Fuentes</h2>')
    expect(html).toContain('<li>[1] Archivo &lt;uno&gt;</li>')
    expect(html).toContain('<li>[2] Archivo &amp; dos</li>')
    expect(html.indexOf('<h2>Fuentes</h2>')).toBeGreaterThan(html.indexOf('<code>formato</code>'))
    expect(html.indexOf('<li>[1]')).toBeLessThan(html.indexOf('<li>[2]'))

    for (const sentinel of [
      'user-asset-sentinel',
      'user-item-sentinel',
      'Usuario source title sentinel',
      'user-collection-id-sentinel',
      'user-collection-sentinel',
      'user-snippet-sentinel',
      '0.9999',
      '9991',
      '9992',
      'user-retrieval-sentinel',
      'user-source-kind-sentinel',
      'user-source-id-sentinel',
      'user-chunk-sentinel',
      '9993',
      '9994',
      'asset-sentinel-one',
      'item-sentinel-one',
      'collection-id-sentinel-one',
      'collection-sentinel-one',
      'snippet-sentinel-one',
      '0.1111',
      '1111',
      '1112',
      'retrieval-sentinel-one',
      'source-kind-sentinel-one',
      'source-id-sentinel-one',
      'chunk-sentinel-one',
      '1113',
      '1114',
      'asset-sentinel-two',
      'item-sentinel-two',
      'collection-id-sentinel-two',
      'collection-sentinel-two',
      'snippet-sentinel-two',
      '0.2222',
      '2221',
      '2222',
      'retrieval-sentinel-two',
      'source-kind-sentinel-two',
      'source-id-sentinel-two',
      'chunk-sentinel-two',
      '2223',
      '2224',
    ]) {
      expect(html).not.toContain(sentinel)
    }
  })
})

describe('downloadRagConversationPdf', () => {
  it('loads the requested id and writes generated bytes to Downloads', async () => {
    const bytes = Uint8Array.from([1, 2, 3])
    getConversationMock.mockResolvedValue(conversation)
    generatePdfMock.mockResolvedValue(bytes)
    vi.mocked(downloadDir).mockResolvedValue('C:/Users/test/Downloads')
    vi.mocked(join).mockResolvedValue('C:/Users/test/Downloads/Acta sindical - conv-42.pdf')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const { downloadRagConversationPdf } = await import('./rag-chat-export')
    await expect(downloadRagConversationPdf('conv-42')).resolves.toBe(
      'C:/Users/test/Downloads/Acta sindical - conv-42.pdf'
    )

    expect(getConversationMock).toHaveBeenCalledWith('conv-42')
    expect(generatePdfMock).toHaveBeenCalledWith(expect.stringContaining('Acta &lt;sindical&gt;'))
    expect(join).toHaveBeenCalledWith(
      'C:/Users/test/Downloads',
      'Acta sindical - conv-42.pdf'
    )
    expect(writeFile).toHaveBeenCalledWith(
      'C:/Users/test/Downloads/Acta sindical - conv-42.pdf',
      bytes
    )
  })

  it('sanitizes unsafe conversation ids while keeping the export in Downloads', async () => {
    const unsafeConversation: RagConversation = {
      ...conversation,
      id: '../../\\:*?"<>',
    }
    const bytes = Uint8Array.from([4, 5, 6])
    getConversationMock.mockResolvedValue(unsafeConversation)
    generatePdfMock.mockResolvedValue(bytes)
    vi.mocked(downloadDir).mockResolvedValue('C:/Users/test/Downloads')
    vi.mocked(join).mockImplementation(async (...parts) => parts.join('/'))
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const { downloadRagConversationPdf } = await import('./rag-chat-export')
    const path = await downloadRagConversationPdf(unsafeConversation.id)

    const [directory, filename] = vi.mocked(join).mock.calls[0]!
    expect(directory).toBe('C:/Users/test/Downloads')
    expect(filename).toMatch(/^Acta sindical - [A-Za-z0-9_-]+\.pdf$/)
    expect(path).toBe(`C:/Users/test/Downloads/${filename}`)
    expect(path.startsWith(`${directory}/`)).toBe(true)
    expect(writeFile).toHaveBeenCalledWith(path, bytes)
  })

  it('strips Windows control characters from export filenames', async () => {
    const unsafeConversation: RagConversation = {
      ...conversation,
      title: 'Acta\u0000\u001f final',
    }
    const bytes = Uint8Array.from([7, 8, 9])
    getConversationMock.mockResolvedValue(unsafeConversation)
    generatePdfMock.mockResolvedValue(bytes)
    vi.mocked(downloadDir).mockResolvedValue('C:/Users/test/Downloads')
    vi.mocked(join).mockImplementation(async (...parts) => parts.join('/'))
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const { downloadRagConversationPdf } = await import('./rag-chat-export')
    await downloadRagConversationPdf(unsafeConversation.id)

    expect(join).toHaveBeenCalledWith(
      'C:/Users/test/Downloads',
      'Acta final - conv-42.pdf'
    )
  })

  it('does not write when PDF generation fails', async () => {
    getConversationMock.mockResolvedValue(conversation)
    generatePdfMock.mockRejectedValue(new Error('pdf failed'))

    const { downloadRagConversationPdf } = await import('./rag-chat-export')
    await expect(downloadRagConversationPdf('conv-42')).rejects.toThrow('pdf failed')
    expect(writeFile).not.toHaveBeenCalled()
  })
})
