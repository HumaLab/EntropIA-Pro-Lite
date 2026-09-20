import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  downloadInvestigationReport,
  investigationReportBytes,
  reportFileName,
  reportMarkdownWithQuery,
} from './investigation-export'

const REPORT = {
  markdown: '# Roberto Crocitto y el SOIP\n\n## Hechos\n\nEl plenario dispuso un paro.',
  question: '¿Quién fue Roberto Crocitto?',
  queryHeading: 'Consulta',
}

function decode(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes)
}

afterEach(() => {
  vi.mocked(save).mockReset()
  vi.mocked(writeFile).mockReset()
})

describe('the query heads the downloaded report', () => {
  it('puts it under the title, not above it', () => {
    const markdown = reportMarkdownWithQuery(REPORT)

    expect(markdown).toBe(
      '# Roberto Crocitto y el SOIP\n\n## Consulta\n\n¿Quién fue Roberto Crocitto?\n\n## Hechos\n\nEl plenario dispuso un paro.'
    )
  })

  it('heads the file when the report carries no title', () => {
    const markdown = reportMarkdownWithQuery({ ...REPORT, markdown: 'Sin encabezado.' })

    expect(markdown).toBe('## Consulta\n\n¿Quién fue Roberto Crocitto?\n\nSin encabezado.')
  })

  // A newer engine may write the question into the report itself. Prepending it
  // anyway would print it twice, two lines apart.
  it('does not repeat a question the report already states', () => {
    const already = `# Informe\n\n## Consulta\n\n${REPORT.question}\n\n## Hechos\n\nTexto.`

    expect(reportMarkdownWithQuery({ ...REPORT, markdown: already })).toBe(already)
  })

  it('leaves the report alone when there is no question to add', () => {
    expect(reportMarkdownWithQuery({ ...REPORT, question: '   ' })).toBe(REPORT.markdown)
  })
})

describe('each format carries the same document', () => {
  it('writes the markdown as text', async () => {
    const bytes = await investigationReportBytes('markdown', REPORT)

    expect(decode(bytes)).toContain('## Consulta')
  })

  it('wraps the rendered markdown as a printable HTML document', async () => {
    const html = decode(await investigationReportBytes('html', REPORT))

    expect(html.startsWith('<!doctype html>')).toBe(true)
    // The same print hook ocr-pdf looks for, so a report prints like the rest.
    expect(html).toContain('class="ocr-export-document"')
    expect(html).toContain('<h1>Roberto Crocitto y el SOIP</h1>')
    expect(html).toContain('¿Quién fue Roberto Crocitto?')
  })

  it('hands Word the very HTML it would have written to disk', async () => {
    const docx = vi.fn(async () => new Uint8Array([1, 2, 3]))

    const bytes = await investigationReportBytes('docx', REPORT, { docx })

    expect(bytes).toEqual(new Uint8Array([1, 2, 3]))
    expect(docx).toHaveBeenCalledWith(decode(await investigationReportBytes('html', REPORT)))
  })
})

describe('the download reports what actually happened', () => {
  it('writes to the chosen path with the format extension offered', async () => {
    vi.mocked(save).mockResolvedValue('C:/tmp/informe.md')

    const outcome = await downloadInvestigationReport(REPORT, 'markdown', 'informe')

    expect(outcome).toEqual({ kind: 'saved', path: 'C:/tmp/informe.md' })
    expect(vi.mocked(save).mock.calls[0]?.[0]).toMatchObject({
      defaultPath: 'informe.md',
      filters: [{ name: 'Markdown', extensions: ['md'] }],
    })
    expect(vi.mocked(writeFile)).toHaveBeenCalledOnce()
  })

  // A closed dialog is not a failure, and must not write or report one.
  it('says cancelled and writes nothing when nobody chose a file', async () => {
    vi.mocked(save).mockResolvedValue(null)

    expect(await downloadInvestigationReport(REPORT, 'docx', 'informe')).toEqual({
      kind: 'cancelled',
    })
    expect(vi.mocked(writeFile)).not.toHaveBeenCalled()
  })

  it('reports a write that threw instead of claiming a file exists', async () => {
    vi.mocked(save).mockResolvedValue('C:/tmp/informe.md')
    vi.mocked(writeFile).mockRejectedValue(new Error('disco lleno'))

    expect(await downloadInvestigationReport(REPORT, 'markdown', 'informe')).toEqual({
      kind: 'failed',
      message: 'disco lleno',
    })
  })
})

describe('the default filename survives the save dialog', () => {
  it('drops the characters Windows rejects in a path', () => {
    expect(reportFileName('¿Quién fue Crocitto? SOIP: 1965/66')).toBe(
      '¿Quién fue Crocitto SOIP 196566'
    )
  })

  it('falls back to a name when the title had nothing usable', () => {
    expect(reportFileName('  ///  ')).toBe('informe')
  })
})
