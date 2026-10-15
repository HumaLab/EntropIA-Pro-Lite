import { strFromU8, unzipSync } from 'fflate'
import { describe, expect, it } from 'vitest'
import { buildPrintableHtml, generateDocxBytes } from './ocr-export'

/**
 * The OCR HTML as a real DOCX (audit D-01).
 *
 * # Why the tests open the zip
 *
 * The exporter this replaced (`html-docx-js`) wrapped an MHTML blob in an
 * `altChunk`: what a reader saw was produced by the application opening the
 * file, not by the file. So the assertions here are about the parts inside —
 * a table is a `w:tbl`, a heading is a heading style, an image has its bytes
 * in `word/media/` — exactly as for the manuscript export.
 */

/** A 1×1 IHDR-only PNG, big enough for `imageSize` to read its declared size. */
function pngBytes(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(33)
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0)
  const view = new DataView(bytes.buffer)
  view.setUint32(8, 13)
  bytes.set([0x49, 0x48, 0x44, 0x52], 12)
  view.setUint32(16, width)
  view.setUint32(20, height)
  return bytes
}

function dataUrl(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return `data:image/png;base64,${btoa(binary)}`
}

const PNG = dataUrl(pngBytes(640, 320))

/** Every construct the OCR vocabulary admits, in one sanitized body. */
const OCR_BODY = [
  '<h1>Informe OCR</h1>',
  '<h2>Contexto</h2>',
  '<h3>Detalle</h3>',
  '<p>Texto con <strong>negrita</strong>, <em>cursiva</em>, <u>subrayado</u>, <code>código</code> y <a href="https://archivo.example.org">enlace</a>.</p>',
  '<ul><li>uno<ul><li>anidado</li></ul></li><li>dos</li></ul>',
  '<ol><li>primero</li><li>segundo</li></ol>',
  '<blockquote><p>La ciudad quedó detenida.</p></blockquote>',
  '<pre>linea uno\nlinea dos</pre>',
  '<table><caption>Tabla 1</caption><thead><tr><th>Año</th><th>Hecho</th></tr></thead>',
  '<tbody><tr><td colspan="2">1919</td></tr><tr><td rowspan="2">Marzo</td><td>Paro</td></tr>',
  '<tr><td>Acuerdo</td></tr></tbody></table>',
  '<div align="center"><span>texto suelto</span></div>',
  `<p>antes <img src="${PNG}" alt="crop" /> después</p>`,
].join('\n')

async function parts(html: string) {
  const bytes = await generateDocxBytes(html)
  const files = unzipSync(bytes)
  const read = (path: string) => (files[path] ? strFromU8(files[path]!) : null)
  return { bytes, names: Object.keys(files), read }
}

describe('the OCR HTML as a DOCX', () => {
  it('is a real package, with no altChunk, title or bibliography', async () => {
    const { bytes, names, read } = await parts(buildPrintableHtml(OCR_BODY))
    const body = read('word/document.xml') ?? ''

    expect(bytes).toBeInstanceOf(Uint8Array)
    expect(names).toContain('word/document.xml')
    expect(body).not.toContain('altChunk')
    // Only the document's own H1: no title paragraph above it, no bibliography
    // heading below everything.
    expect(body.match(/<w:pStyle w:val="Heading1"\/>/g) ?? []).toHaveLength(1)
    expect(body).not.toContain('<w:pStyle w:val="Title"/>')
    expect(body).not.toContain('Bibliografía')
  })

  it('parses a bare fragment too, without the printable wrapper', async () => {
    const { read } = await parts('<p>hola</p>')

    expect(read('word/document.xml')).toContain('hola')
  })

  it('writes each heading at its own level', async () => {
    const body = (await parts(buildPrintableHtml(OCR_BODY))).read('word/document.xml') ?? ''

    expect(body).toContain('<w:pStyle w:val="Heading1"/>')
    expect(body).toContain('<w:pStyle w:val="Heading2"/>')
    expect(body).toContain('<w:pStyle w:val="Heading3"/>')
  })

  it('writes the inline marks and a live hyperlink', async () => {
    const { names, read } = await parts(buildPrintableHtml(OCR_BODY))
    const body = read('word/document.xml') ?? ''

    expect(body).toContain('<w:b/>')
    expect(body).toContain('<w:i/>')
    expect(body).toContain('<w:u ')
    expect(body).toContain('w:ascii="Consolas"')
    expect(body).toContain('w:hyperlink')
    expect(body).toContain('<w:rStyle w:val="Hyperlink"/>')
    expect(names).toContain('word/_rels/document.xml.rels')
    expect(read('word/_rels/document.xml.rels')).toContain('https://archivo.example.org')
  })

  it('writes lists with numbering, including the nested level', async () => {
    const body = (await parts(buildPrintableHtml(OCR_BODY))).read('word/document.xml') ?? ''

    expect(body).toContain('w:numPr')
    expect(body).toContain('<w:ilvl w:val="1"/>')
    expect(body).toContain('anidado')
  })

  it('writes the caption before a real table, with colspan as a grid span and rowspan as a merge', async () => {
    const body = (await parts(buildPrintableHtml(OCR_BODY))).read('word/document.xml') ?? ''

    expect(body).toContain('<w:tbl>')
    expect(body).toContain('<w:gridSpan w:val="2"/>')
    expect(body).toContain('w:vMerge w:val="restart"')
    expect(body).toContain('w:vMerge w:val="continue"')
    const caption = body.indexOf('Tabla 1')
    expect(caption).toBeGreaterThan(-1)
    expect(caption).toBeLessThan(body.indexOf('<w:tbl>'))
  })

  it('writes a blockquote as a bordered paragraph and a code block in a monospaced font', async () => {
    const body = (await parts(buildPrintableHtml(OCR_BODY))).read('word/document.xml') ?? ''

    expect(body).toContain('<w:pBdr>')
    expect(body).toContain('La ciudad quedó detenida.')
    expect(body).toContain('linea uno')
    expect(body).toContain('w:ascii="Consolas"')
  })

  it('embeds the image bytes and draws them between the words around them', async () => {
    const { names, read } = await parts(buildPrintableHtml(OCR_BODY))
    const body = read('word/document.xml') ?? ''

    expect(names.some((name) => name.startsWith('word/media/'))).toBe(true)
    expect(body).toContain('<w:drawing>')
    expect(body.indexOf('antes')).toBeLessThan(body.indexOf('<w:drawing>'))
    expect(body.indexOf('<w:drawing>')).toBeLessThan(body.indexOf('después'))
    expect(body).toContain('texto suelto')
  })

  it('keeps the half-inch page margins the printable HTML used', async () => {
    const body = (await parts(buildPrintableHtml(OCR_BODY))).read('word/document.xml') ?? ''

    expect(body).toMatch(/<w:pgMar [^>]*w:top="720"[^>]*w:right="720"/)
    expect(body).toContain('w:bottom="720"')
    expect(body).toContain('w:left="720"')
  })
})
