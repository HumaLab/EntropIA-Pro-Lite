import { invoke } from '@tauri-apps/api/core'
import { unzipSync, strFromU8 } from 'fflate'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Node } from './export-document'
import { exportDocument, isExportFailure, type ExportSettings } from './writing-export'

/**
 * The pattern document (plan-editor.md §17.4).
 *
 * §17.4 asks each exporter for fidelity tests against *one* document holding
 * every admitted node. That is what this is: one manuscript, three formats, and
 * an assertion per obligatory element of §17.1 in each. Per-construct tests
 * live beside each exporter and say what broke; this says whether the whole
 * thing survives together — which is a different question, and the one a writer
 * actually asks.
 */

const mockInvoke = vi.mocked(invoke)

const text = (value: string, marks?: { type: string; attrs?: Record<string, unknown> }[]) => ({
  type: 'text',
  text: value,
  ...(marks ? { marks } : {}),
})
const p = (...content: Node[]) => ({ type: 'paragraph', content })
const cell = (value: string) => ({ type: 'tableCell', content: [p(text(value))] })
const head = (value: string) => ({ type: 'tableHeader', content: [p(text(value))] })

/** Every node the schema admits, in one manuscript. */
const PATTERN: Node = {
  type: 'doc',
  content: [
    { type: 'heading', attrs: { level: 1 }, content: [text('La huelga de 1919')] },
    { type: 'heading', attrs: { level: 2 }, content: [text('El conflicto')] },
    p(
      text('Los talleres pararon en '),
      text('enero', [{ type: 'bold' }]),
      text(', de modo '),
      text('sostenido', [{ type: 'italic' }]),
      text(' y '),
      text('documentado', [{ type: 'underline' }]),
      text('.'),
      { type: 'footnoteReference', attrs: { 'data-id': 'f1' } }
    ),
    p(
      text('Ver el '),
      text('registro', [{ type: 'link', attrs: { href: 'https://archivo.example.org' } }]),
      text(' y la cita '),
      { type: 'zoteroCitation', attrs: { citationNodeId: 'z1', items: [{ metadataSnapshot: '{"id":"acha2015"}' }] } },
      text(' junto al fragmento '),
      {
        type: 'documentCitation',
        attrs: {
          quotedText: 'los obreros declararon la huelga',
          pageNumber: 112,
          metadataSnapshot: { title: 'Acta del gremio', pageNumber: 112 },
        },
      },
      text(' y la nota '),
      { type: 'noteLink', attrs: { contentSnapshot: 'lo anotado en su momento' } }
    ),
    {
      type: 'bulletList',
      content: [
        { type: 'listItem', content: [p(text('los metalúrgicos'))] },
        { type: 'listItem', content: [p(text('los gráficos'))] },
      ],
    },
    {
      type: 'orderedList',
      content: [
        { type: 'listItem', content: [p(text('primero el paro'))] },
        { type: 'listItem', content: [p(text('después la represión'))] },
      ],
    },
    { type: 'blockquote', content: [p(text('La ciudad quedó detenida.'))] },
    { type: 'codeBlock', attrs: { language: 'text' }, content: [text('cifras del padrón')] },
    { type: 'horizontalRule' },
    {
      type: 'table',
      content: [
        { type: 'tableRow', content: [head('Año'), head('Hecho')] },
        { type: 'tableRow', content: [cell('1919'), cell('Semana Trágica')] },
      ],
    },
    p(text('Una línea'), { type: 'hardBreak' }, text('y la siguiente.')),
    {
      type: 'footnotes',
      content: [
        {
          type: 'footnote',
          attrs: { 'data-id': 'f1' },
          content: [p(text('Según el parte policial del día 9.'))],
        },
      ],
    },
  ],
}

const settings: ExportSettings = {
  format: 'markdown',
  citations: 'footnote',
  bibliography: true,
  style: { kind: 'bundled', name: 'apa' },
  title: 'La huelga de 1919',
  bibliographyHeading: 'Bibliografía',
}

beforeEach(() => {
  mockInvoke.mockReset()
  mockInvoke.mockImplementation(async (command: string) => {
    if (command === 'writing_csl_render_document') {
      return [{ text: '(Acha, 2015)', author_suppressed: false }] as never
    }
    if (command === 'writing_csl_bibliography') {
      return ['Acha, O. (2015). Un libro. Editorial.'] as never
    }
    throw new Error(`unexpected command: ${command}`)
  })
})

async function exported(format: ExportSettings['format'], extra: Partial<ExportSettings> = {}) {
  const out = await exportDocument(PATTERN, { ...settings, format, ...extra })
  if (isExportFailure(out)) throw new Error(`refused: ${out.elements.join(', ')}`)
  return out
}

const asText = (bytes: Uint8Array) => new TextDecoder().decode(bytes)

/**
 * The artifacts §17.4 asks a human to open.
 *
 * *"Deben registrarse aplicaciones de lectura y versiones verificadas."* No
 * test can do that — whether a footnote renumbers in Word is a fact about Word.
 * So the three files are written out on every run, which is the only way they
 * cannot drift from the exporters they came from, and a person checks them
 * against `docs/escritura-export-pattern/LEEME.md`.
 */
describe('the artifacts for human verification', () => {
  it('writes the pattern document in all three formats', async () => {
    const { mkdirSync, statSync, writeFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const into = resolve(import.meta.dirname, '../../../../docs/escritura-export-pattern')
    mkdirSync(into, { recursive: true })

    const written: string[] = []
    for (const [format, name] of [
      ['markdown', 'patron.md'],
      ['html', 'patron.html'],
      ['docx', 'patron.docx'],
    ] as const) {
      const path = resolve(into, name)
      writeFileSync(path, (await exported(format)).bytes)
      written.push(path)
    }

    // A file that exists and is empty is the failure worth catching here: the
    // reviewer opens it, sees nothing, and cannot tell whether the exporter or
    // the reader is at fault.
    expect(written.map((path) => statSync(path).size > 200)).toEqual([true, true, true])
  })
})

describe('the pattern document in markdown', () => {
  it('keeps every obligatory element of §17.1', async () => {
    const out = asText((await exported('markdown')).bytes)

    expect(out).toContain('# La huelga de 1919')
    expect(out).toContain('## El conflicto')
    expect(out).toContain('**enero**')
    expect(out).toContain('*sostenido*')
    expect(out).toContain('- los metalúrgicos')
    expect(out).toContain('1. primero el paro')
    expect(out).toContain('> La ciudad quedó detenida')
    expect(out).toContain('[registro](https://archivo.example.org)')
    expect(out).toContain('| Año | Hecho |')
    // Escaped, like every other piece of prose that leaves as Markdown.
    expect(out).toContain('[^1]: Según el parte policial del día 9\\.')
    expect(out).toContain('[^2]: Acta del gremio, p\\. 112\\.')
    expect(out).toContain('\\(Acha, 2015\\)')
    expect(out).toContain('## Bibliografía')
    expect(out).toContain('Acha, O\\. \\(2015\\)')
  })

  /** The one thing Markdown cannot do natively, reported rather than dropped. */
  it('reports the underline it had to stand in for', async () => {
    const out = await exported('markdown')

    expect(out.warnings).toContainEqual({
      element: 'underline',
      kind: 'mark',
      support: 'fallback',
      count: 1,
    })
  })
})

describe('the pattern document in html', () => {
  it('keeps every obligatory element of §17.1', async () => {
    const out = asText((await exported('html')).bytes)

    expect(out).toContain('<h1>La huelga de 1919</h1>')
    expect(out).toContain('<h2>El conflicto</h2>')
    expect(out).toContain('<strong>enero</strong>')
    expect(out).toContain('<u>documentado</u>')
    expect(out).toContain('<ul>')
    expect(out).toContain('<ol>')
    expect(out).toContain('<blockquote>')
    expect(out).toContain('href="https://archivo.example.org"')
    expect(out).toContain('<th><p>Año</p></th>')
    expect(out).toContain('Según el parte policial del día 9.')
    expect(out).toContain('Acta del gremio, p. 112')
    expect(out).toContain('(Acha, 2015)')
    expect(out).toContain('Acha, O. (2015). Un libro. Editorial.')
  })

  it('has nothing to warn about', async () => {
    expect((await exported('html')).warnings.filter((w) => w.element !== 'noteLink')).toEqual([])
  })
})

describe('the pattern document in docx', () => {
  /**
   * Read out of the zip, because that is the check that would have caught the
   * incumbent: S4 found `html-docx-js` producing a file Word renders and that
   * contains no document model at all.
   */
  it('keeps every obligatory element of §17.1 as real OOXML', async () => {
    const files = unzipSync((await exported('docx')).bytes)
    const read = (path: string) => (files[path] ? strFromU8(files[path]!) : '')
    const body = read('word/document.xml')

    expect(Object.keys(files)).toContain('word/footnotes.xml')
    expect(body).not.toContain('altChunk')
    expect(body).toContain('Heading1')
    expect(body).toContain('Heading2')
    expect(body).toContain('<w:b/>')
    expect(body).toContain('<w:i/>')
    expect(body).toContain('<w:u ')
    expect(body).toContain('w:numPr')
    expect(body).toContain('<w:tbl>')
    expect(body).toContain('w:hyperlink')
    // Live *and* visibly a link: S4 verified this element as blue and
    // underlined, and a link nobody can see is clickable is not one.
    expect(body).toContain('<w:rStyle w:val="Hyperlink"/>')
    expect(body).toContain('w:footnoteReference')
    expect(body).toContain('w:hanging')
    expect(body).toContain('(Acha, 2015)')
    expect(read('word/footnotes.xml')).toContain('Según el parte policial del día 9.')
    expect(read('word/footnotes.xml')).toContain('Acta del gremio, p. 112')
    expect(read('word/_rels/document.xml.rels')).toContain('https://archivo.example.org')
  })

  /**
   * §17.4: only the note link, which is a fallback everywhere by design and is
   * not on the obligatory list. Anything else here would be a regression.
   */
  it('warns about nothing but the note link', async () => {
    const out = await exported('docx')

    expect(out.warnings.map((warning) => warning.element)).toEqual(['noteLink'])
  })
})

describe('the line §17.4 draws', () => {
  /**
   * *"Una advertencia no permite declarar cumplido un elemento obligatorio que
   * DOCX deba conservar."* A comment is not admitted in Markdown, which is a
   * limitation of the format §17.1 allows — but if a required element were ever
   * downgraded in DOCX, no file would be written at all.
   */
  it('does not write a DOCX that would lose an obligatory element', async () => {
    const { losesRequiredElement } = await import('./export-fidelity')
    const lost = losesRequiredElement(
      [{ element: 'footnote', kind: 'node', support: 'fallback', count: 1 }],
      'docx'
    )

    expect(lost).toEqual(['footnote'])
  })

  /** And nothing here alters the manuscript (§17.2's closing line). */
  it('leaves the document exactly as it found it', async () => {
    const before = JSON.stringify(PATTERN)

    await exported('docx')
    await exported('markdown', { citations: 'inline' })
    await exported('html', { citations: 'quote_with_note' })

    expect(JSON.stringify(PATTERN)).toBe(before)
  })
})

describe('when the citation engine cannot answer', () => {
  /**
   * §11.3's rule — a citation that cannot be rendered must not stop the work —
   * applies at least as strongly to exporting. The file is produced, with
   * whatever each node cached, and the trouble is reported.
   */
  it('still produces the file and says what happened', async () => {
    mockInvoke.mockRejectedValue({ code: 'style_invalid', message: 'el estilo no se pudo leer' })

    const out = await exported('markdown')

    expect(out.citationTrouble).toContain('el estilo')
    expect(asText(out.bytes)).toContain('La huelga de 1919')
  })

  it('does not ask the engine about a manuscript that cites nothing', async () => {
    const plain = { type: 'doc', content: [p(text('sin citas'))] }

    await exportDocument(plain, settings)

    expect(mockInvoke).not.toHaveBeenCalled()
  })
})
