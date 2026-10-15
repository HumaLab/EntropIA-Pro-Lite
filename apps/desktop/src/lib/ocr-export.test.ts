import { strFromU8, unzipSync } from 'fflate'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'

import { exportOcrText, generateOcrExportBytes, prepareOcrExport } from './ocr-export'

const prepared = {
  markdown: '# Título\n',
  html: '<h1>Título</h1><img src="data:image/png;base64,AAAA" alt="crop" />',
}

const renderInput = {
  source:
    '# Título\r\n\r\n<div align="center">HTML</div>\r\n\r\n![](page=0,bbox=[1,2,3,4])\r\n\r\n![](page=0,bbox=[1,2,3,4])',
  assetUrl: 'asset://source',
  sourceType: 'image' as const,
  referenceWidth: 100,
  referenceHeight: 100,
}

const input = {
  source: '# Título\n',
  assetUrl: 'asset://source',
  sourceType: 'image' as const,
  referenceWidth: 100,
  referenceHeight: 100,
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
  vi.mocked(save).mockReset()
  vi.mocked(writeFile).mockReset()
})

describe('prepareOcrExport', () => {
  it('preserves source Markdown/HTML and embeds every valid OCR region', async () => {
    const result = await prepareOcrExport(renderInput, async (reference) => {
      return reference.token === 'region-0'
        ? 'data:image/png;base64,AAAA'
        : 'data:image/png;base64,BBBB'
    })

    expect(result.markdown).toContain('# Título')
    expect(result.markdown).toContain('<div align="center">HTML</div>')
    expect(result.markdown).toContain('![OCR region from page 1](data:image/png;base64,AAAA)')
    expect(result.markdown).toContain('![OCR region from page 1](data:image/png;base64,BBBB)')
    expect(result.markdown).not.toContain('\r')
    expect(result.html).toContain('<h1>Título</h1>')
    expect(result.html).toContain('<img src="data:image/png;base64,AAAA"')
    expect(result.html).toContain('<img src="data:image/png;base64,BBBB"')
  })

  it('uses a readable marker for a rejected region without dropping surrounding content', async () => {
    const result = await prepareOcrExport(
      { ...renderInput, source: 'antes ![](page=4,bbox=[1,2,3,4]) después' },
      async () => {
        throw new Error('source unavailable')
      }
    )

    expect(result.markdown).toContain('*[Imagen OCR no disponible]*')
    expect(result.html).toContain('Imagen OCR no disponible')
    expect(result.html).toContain('antes')
    expect(result.html).toContain('después')
  })
})

describe('OCR export adapters', () => {
  /**
   * D-01: the DOCX generator used to load `html-docx-js` by injecting a
   * `<script>`. The package builds the file in-process now; a script element
   * appearing here would be the regression.
   */
  it('writes a real DOCX without injecting a script', async () => {
    const appendChild = vi
      .spyOn(document.head, 'appendChild')
      .mockImplementation(((node: Node) => node) as typeof document.head.appendChild)

    const bytes = await generateOcrExportBytes('docx', prepared)

    expect(appendChild).not.toHaveBeenCalled()
    expect(document.querySelector('script')).toBeNull()

    const files = unzipSync(bytes)
    expect(Object.keys(files)).toContain('word/document.xml')
    expect(strFromU8(files['word/document.xml']!)).not.toContain('altChunk')
  })

  it('routes PDF and DOCX through the same prepared HTML', async () => {
    const pdf = vi.fn(async (html: string) => {
      expect(html).toContain('<h1>Título</h1>')
      expect(html).toContain('data:image/png;base64,AAAA')
      return Uint8Array.from([1, 2])
    })
    const docx = vi.fn(async (html: string) => {
      expect(html).toContain('<h1>Título</h1>')
      expect(html).toContain('data:image/png;base64,AAAA')
      return Uint8Array.from([3, 4])
    })

    await expect(generateOcrExportBytes('pdf', prepared, { pdf, docx })).resolves.toEqual(
      Uint8Array.from([1, 2])
    )
    await expect(generateOcrExportBytes('docx', prepared, { pdf, docx })).resolves.toEqual(
      Uint8Array.from([3, 4])
    )

    expect(pdf).toHaveBeenCalledTimes(1)
    expect(docx).toHaveBeenCalledTimes(1)
    expect(pdf.mock.calls[0]![0]).toBe(docx.mock.calls[0]![0])

    expect(pdf.mock.calls[0]![0]).toContain('<div class="ocr-export-document">')
    expect(pdf.mock.calls[0]![0]).toContain('.ocr-export-document')
    expect(pdf.mock.calls[0]![0]).not.toContain(':root')
    expect(pdf.mock.calls[0]![0]).toContain('.ocr-export-document p {')
  })

  it('encodes Markdown bytes as UTF-8', async () => {
    const bytes = await generateOcrExportBytes('markdown', prepared)

    expect(bytes).toEqual(new TextEncoder().encode('# Título\n'))
  })

  it.each([
    {
      format: 'markdown' as const,
      defaultName: 'scan-ocr.md',
      savedPath: '/exports/scan-ocr.md',
      filterName: 'Markdown',
      extension: 'md',
      bytes: new TextEncoder().encode('# Título\n'),
    },
    {
      format: 'pdf' as const,
      defaultName: 'scan-ocr.pdf',
      savedPath: '/exports/scan-ocr.pdf',
      filterName: 'PDF',
      extension: 'pdf',
      bytes: Uint8Array.from([1, 2]),
    },
    {
      format: 'docx' as const,
      defaultName: 'scan-ocr.docx',
      savedPath: '/exports/scan-ocr.docx',
      filterName: 'Microsoft Word',
      extension: 'docx',
      bytes: Uint8Array.from([3, 4]),
    },
  ])(
    'writes $format bytes after choosing a path',
    async ({ format, defaultName, savedPath, filterName, extension, bytes }) => {
      vi.mocked(save).mockResolvedValue(savedPath)
      vi.mocked(writeFile).mockResolvedValue(undefined)

      const pdf = vi.fn(async () => bytes)
      const docx = vi.fn(async () => bytes)

      await expect(
        exportOcrText(input, format, defaultName, { generators: { pdf, docx } })
      ).resolves.toBe(savedPath)

      expect(save).toHaveBeenCalledWith({
        defaultPath: defaultName,
        filters: [{ name: filterName, extensions: [extension] }],
      })
      expect(writeFile).toHaveBeenCalledWith(savedPath, bytes)
    }
  )

  it('does not write when the save dialog is cancelled', async () => {
    vi.mocked(save).mockResolvedValue(null)

    const pdf = vi.fn(async () => Uint8Array.from([1, 2]))
    const docx = vi.fn(async () => Uint8Array.from([3, 4]))

    await expect(
      exportOcrText(input, 'pdf', 'scan-ocr.pdf', { generators: { pdf, docx } })
    ).resolves.toBeNull()

    expect(writeFile).not.toHaveBeenCalled()
    expect(pdf).not.toHaveBeenCalled()
    expect(docx).not.toHaveBeenCalled()
  })
})
