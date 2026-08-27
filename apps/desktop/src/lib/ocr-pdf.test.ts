import { describe, expect, it } from 'vitest'
import type { Content, ContentImage, ContentUnorderedList } from 'pdfmake/interfaces'

import { buildOcrPdfDefinition } from './ocr-pdf'

const ONE_PIXEL_PNG =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+X1S8AAAAAElFTkSuQmCC'

function documentHtml(content: string): string {
  return `<!doctype html><html><body><div class="ocr-export-document">${content}</div></body></html>`
}

function contentArray(content: Content): Content[] {
  return Array.isArray(content) ? content : [content]
}

describe('buildOcrPdfDefinition', () => {
  it('maps headings and inline formatting to native text runs', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<h1>Título ñandú</h1>' +
          '<p>Texto <strong>negrita</strong> <em>cursiva</em> ' +
          '<u>subrayado</u><br><a href="https://example.test/">enlace</a>.</p>'
      )
    )
    const content = contentArray(definition.content)

    expect(content[0]).toMatchObject({
      style: 'h1',
      headlineLevel: 1,
      text: [{ text: 'Título ñandú' }],
    })
    expect(content[1]).toMatchObject({
      style: 'paragraph',
      text: [
        { text: 'Texto ' },
        { text: 'negrita', bold: true },
        { text: ' ' },
        { text: 'cursiva', italics: true },
        { text: ' ' },
        { text: 'subrayado', decoration: 'underline' },
        { text: '\n' },
        { text: 'enlace', link: 'https://example.test/' },
        { text: '.' },
      ],
    })
    expect(definition.pageSize).toBe('A4')
    expect(definition.pageOrientation).toBe('portrait')
    expect(definition.pageMargins).toEqual([34.02, 34.02, 34.02, 34.02])
    expect(definition.defaultStyle).toMatchObject({ font: 'Roboto', fontSize: 11 })
  })

  it('maps nested blocks, lists, code, images, and unknown wrappers without losing text', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<article><blockquote><p>Cita histórica</p></blockquote>' +
          '<ul><li>Primero<ol><li>Interior</li></ol></li><li>Segundo</li></ul>' +
          '<pre>const x = 1;\n  x++</pre>' +
          `<p>Antes<img src="${ONE_PIXEL_PNG}" alt="crop">Después</p>` +
          '</article>'
      )
    )
    const content = contentArray(definition.content)
    const list = content.find(
      (item): item is ContentUnorderedList =>
        typeof item === 'object' && item !== null && !Array.isArray(item) && 'ul' in item
    )
    const image = content.find(
      (item): item is ContentImage =>
        typeof item === 'object' && item !== null && !Array.isArray(item) && 'image' in item
    )

    expect(content).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          table: expect.objectContaining({
            body: [[expect.objectContaining({ stack: [expect.any(Object)] })]],
          }),
        }),
        expect.objectContaining({
          text: 'const x = 1;\n  x++',
          style: 'codeBlock',
          preserveLeadingSpaces: true,
          preserveTrailingSpaces: true,
        }),
        expect.objectContaining({ text: [{ text: 'Antes' }], style: 'paragraph' }),
        expect.objectContaining({ text: [{ text: 'Después' }], style: 'paragraph' }),
      ])
    )
    expect(list).toMatchObject({
      style: 'list',
      ul: [
        {
          stack: [
            { text: [{ text: 'Primero' }], style: 'listItem' },
            { ol: [{ text: [{ text: 'Interior' }], style: 'listItem' }], style: 'list' },
          ],
        },
        { text: [{ text: 'Segundo' }], style: 'listItem' },
      ],
    })
    expect(image).toMatchObject({ image: ONE_PIXEL_PNG, fit: [527.24, 700] })
  })

  it('unwraps unrecognized wrappers while preserving nested block order', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        `<ocr-wrapper><p>Antes</p><img src="${ONE_PIXEL_PNG}" alt="crop"><p>Después</p></ocr-wrapper>`
      )
    )

    expect(contentArray(definition.content)).toEqual([
      { text: [{ text: 'Antes' }], style: 'paragraph' },
      {
        image: ONE_PIXEL_PNG,
        fit: [527.24, 700],
        margin: [0, 0, 0, 9],
      },
      { text: [{ text: 'Después' }], style: 'paragraph' },
    ])
  })
})
