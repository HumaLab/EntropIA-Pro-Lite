import { describe, expect, it } from 'vitest'
import type { Content, ContentImage, ContentUnorderedList } from 'pdfmake/interfaces'

import { buildOcrPdfDefinition, generateNativeOcrPdfBytes } from './ocr-pdf'

const ONE_PIXEL_PNG =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='

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

  it('maps captions, headers, row spans, and column spans into a rectangular table', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<table><caption>Resumen anual</caption><thead>' +
          '<tr><th rowspan="2">Nombre</th><th colspan="2">Valores</th></tr>' +
          '<tr><th>Uno</th><th>Dos</th></tr></thead><tbody>' +
          '<tr><td>Registro</td><td>10</td><td>20</td></tr>' +
          '</tbody></table>'
      )
    )
    const [tableStack] = contentArray(definition.content)

    expect(tableStack).toMatchObject({
      stack: [
        { text: 'Resumen anual', style: 'tableCaption' },
        {
          table: {
            headerRows: 2,
            widths: ['*', '*', '*'],
            body: [
              [
                expect.objectContaining({ rowSpan: 2, style: 'tableHeader' }),
                expect.objectContaining({ colSpan: 2, style: 'tableHeader' }),
                {},
              ],
              [{}, expect.objectContaining({ style: 'tableHeader' }), expect.any(Object)],
              [
                expect.objectContaining({ style: 'tableCell' }),
                expect.any(Object),
                expect.any(Object),
              ],
            ],
          },
        },
      ],
    })
  })

  it('degrades a malformed table to readable native text', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml('<table><caption>Tabla incompleta</caption></table>')
    )

    expect(contentArray(definition.content)).toEqual([
      { text: 'Tabla incompleta', style: 'paragraph' },
    ])
  })

  it('degrades a table with rows but no usable cells to readable native text', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml('<table><caption>Tabla</caption><tr></tr></table>')
    )

    expect(contentArray(definition.content)).toEqual([{ text: 'Tabla', style: 'paragraph' }])
  })

  it('degrades overlapping row and column spans to readable native text', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<table><tr><td>A</td><td rowspan="2">B</td></tr>' +
          '<tr><td colspan="2">C</td></tr></table>'
      )
    )

    expect(contentArray(definition.content)).toEqual([{ text: 'ABC', style: 'paragraph' }])
  })

  it('generates native extractable text while preserving an embedded OCR image', async () => {
    const bytes = await generateNativeOcrPdfBytes(
      documentHtml(
        '<h1>Radiografía nativa</h1>' +
          '<p>Texto español con <strong>énfasis</strong> y ' +
          '<a href="https://example.test/">enlace verificable</a>.</p>' +
          '<ul><li>Elemento seleccionable</li></ul>' +
          '<table><thead><tr><th>Columna</th></tr></thead>' +
          '<tbody><tr><td>Celda extraíble</td></tr></tbody></table>' +
          `<img src="${ONE_PIXEL_PNG}" alt="OCR region from page 1">`
      )
    )

    expect(Array.from(bytes.slice(0, 5))).toEqual([37, 80, 68, 70, 45])

    // @ts-expect-error -- pdfjs-dist does not publish types for the worker bundle.
    const pdfjsWorker = await import('pdfjs-dist/legacy/build/pdf.worker.min.mjs')
    Object.assign(globalThis, { pdfjsWorker })
    const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs')
    pdfjs.GlobalWorkerOptions.workerSrc = new URL(
      'pdfjs-dist/legacy/build/pdf.worker.min.mjs',
      import.meta.url
    ).href
    const loadingTask = pdfjs.getDocument({ data: bytes.slice() })
    const pdf = await loadingTask.promise
    const page = await pdf.getPage(1)
    const textContent = await page.getTextContent()
    const extracted = textContent.items
      .map((item) => ('str' in item ? item.str : ''))
      .join(' ')
      .replace(/\s+/g, ' ')
      .replace(/\s+([.,;:!?])/g, '$1')
      .trim()

    expect(extracted).toContain('Radiografía nativa')
    expect(extracted).toContain('Texto español con énfasis y enlace verificable.')
    expect(extracted).toContain('Elemento seleccionable')
    expect(extracted).toContain('Columna')
    expect(extracted).toContain('Celda extraíble')
    expect(extracted).not.toContain('OCR region from page 1')

    await pdf.destroy()
    // Alone this takes about a second: it renders a real PDF and then parses it
    // back with pdfjs and its worker. Sharing a machine with the rest of the
    // suite pushed it past the default timeout often enough to look like a
    // broken test, so the budget is stated rather than inherited.
  }, 30_000)
})
