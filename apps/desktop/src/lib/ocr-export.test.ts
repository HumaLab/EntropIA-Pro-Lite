import { describe, expect, it } from 'vitest'
import { prepareOcrExport } from './ocr-export'

const input = {
  source: '# Título\r\n\r\n<div align="center">HTML</div>\r\n\r\n![](page=0,bbox=[1,2,3,4])\r\n\r\n![](page=0,bbox=[1,2,3,4])',
  assetUrl: 'asset://source',
  sourceType: 'image' as const,
  referenceWidth: 100,
  referenceHeight: 100,
}

describe('prepareOcrExport', () => {
  it('preserves source Markdown/HTML and embeds every valid OCR region', async () => {
    const result = await prepareOcrExport(input, async (reference) => {
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
      { ...input, source: 'antes ![](page=4,bbox=[1,2,3,4]) después' },
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
