import { invoke } from '@tauri-apps/api/core'
import { describe, expect, it, vi } from 'vitest'
import {
  describeCapture,
  describeSource,
  formatLocalTime,
  navegadorDeleteSource,
  navegadorListSources,
  navegadorPdfFile,
  navegadorSourceDetail,
  parseSourceError,
  sourceOpenAction,
  type CaptureDetail,
  type SourceSummary,
} from './navegador-sources'

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'

function summary(overrides: Partial<SourceSummary> = {}): SourceSummary {
  return {
    id: 's1',
    title: 'A title',
    finalUrl: 'https://www.example.com/a/b',
    siteName: null,
    updatedAt: 1_790_000_000_000,
    captureCount: 3,
    kinds: ['page', 'selection'],
    ...overrides,
  }
}

function capture(overrides: Partial<CaptureDetail> = {}): CaptureDetail {
  return {
    id: 'c1',
    kind: 'page',
    mimeType: 'text/html',
    accessedAt: '2026-09-30T12:00:00Z',
    finalUrl: 'https://example.com/a',
    title: 'A title',
    sha256: SHA,
    hashOf: 'html',
    sizeBytes: 2048,
    textPreview: 'The readable text',
    textInFile: false,
    quotePrefix: null,
    quoteSuffix: null,
    filePresent: true,
    createdAt: 1,
    ...overrides,
  }
}

describe('source commands', () => {
  it('list with a trimmed query, or none, and the limit', async () => {
    vi.mocked(invoke).mockClear()
    vi.mocked(invoke).mockResolvedValue([])
    await navegadorListSources('  trabajo ')
    await navegadorListSources('   ', 20)
    await navegadorListSources()
    expect(invoke).toHaveBeenNthCalledWith(1, 'navegador_list_sources', {
      query: 'trabajo',
      limit: null,
    })
    expect(invoke).toHaveBeenNthCalledWith(2, 'navegador_list_sources', { query: null, limit: 20 })
    expect(invoke).toHaveBeenNthCalledWith(3, 'navegador_list_sources', {
      query: null,
      limit: null,
    })
  })

  it('name the source by id and never send a path', async () => {
    vi.mocked(invoke).mockClear()
    vi.mocked(invoke).mockResolvedValue(null)
    await navegadorSourceDetail('s1')
    vi.mocked(invoke).mockResolvedValue({ leftoverFiles: false })
    await navegadorDeleteSource('s1')
    expect(invoke).toHaveBeenNthCalledWith(1, 'navegador_source_detail', { sourceId: 's1' })
    expect(invoke).toHaveBeenNthCalledWith(2, 'navegador_delete_source', { sourceId: 's1' })
  })
})

describe('parseSourceError', () => {
  it('reads the code the backend printed and keeps the detail', () => {
    expect(parseSourceError('db_error: no such table: web_sources')).toEqual({
      code: 'db_error',
      detail: 'no such table: web_sources',
    })
    expect(parseSourceError(new Error('not_found: there is no such source'))).toEqual({
      code: 'not_found',
      detail: 'there is no such source',
    })
    expect(parseSourceError('invalid_id').code).toBe('invalid_id')
  })

  it('treats anything else as unknown and keeps its message', () => {
    expect(parseSourceError('Command not found')).toEqual({
      code: 'unknown',
      detail: 'Command not found',
    })
    expect(parseSourceError('../x: y').code).toBe('unknown')
  })
})

describe('formatLocalTime', () => {
  it('shows an instant in the requested time zone', () => {
    const text = formatLocalTime('2026-09-30T12:00:00Z', 'es', 'America/Argentina/Buenos_Aires')
    expect(text).toContain('09:00')
    expect(formatLocalTime('2026-09-30T12:00:00Z', 'en', 'UTC')).toContain('12:00')
  })

  it('hands back what it cannot read, as it was recorded', () => {
    expect(formatLocalTime('not a date', 'es')).toBe('not a date')
    expect(formatLocalTime('', 'es')).toBe('')
  })
})

describe('describeSource', () => {
  it('uses the title, the host without www, the count and the kinds', () => {
    const view = describeSource(summary())
    expect(view).toMatchObject({
      id: 's1',
      title: 'A title',
      host: 'example.com',
      url: 'https://www.example.com/a/b',
      captureCount: 3,
      kinds: ['page', 'selection'],
    })
  })

  it('falls back to the address when there is no title', () => {
    expect(describeSource(summary({ title: null })).title).toBe('https://www.example.com/a/b')
    expect(describeSource(summary({ title: '   ' })).title).toBe('https://www.example.com/a/b')
  })

  it('keeps the host empty for an address that is not parseable', () => {
    expect(describeSource(summary({ finalUrl: 'nonsense' })).host).toBe('')
  })
})

describe('describeCapture', () => {
  it('shows local time next to the recorded UTC, a short hash and the size', () => {
    const view = describeCapture(capture(), 'es', 'America/Argentina/Buenos_Aires')
    expect(view.accessedUtc).toBe('2026-09-30T12:00:00Z')
    expect(view.accessedLocal).toContain('09:00')
    expect(view.shortSha).toBe('ba7816bf8f01')
    expect(view.size).toBe('2.0 KB')
    expect(view.file).toBe('present')
    expect(view.preview).toBe('The readable text')
  })

  it('tells a missing file from a capture that never had one', () => {
    expect(describeCapture(capture({ filePresent: false })).file).toBe('missing')
    expect(describeCapture(capture({ filePresent: null })).file).toBe('none')
  })

  it('shows a selection as its quote with the text around it', () => {
    const view = describeCapture(
      capture({
        kind: 'selection',
        hashOf: 'quote',
        textPreview: 'the quote',
        quotePrefix: 'before ',
        quoteSuffix: ' after',
        filePresent: null,
      })
    )
    expect(view.quote).toEqual({ before: 'before ', quote: 'the quote', after: ' after' })
  })

  it('collapses a long preview to one bounded line and never treats it as markup', () => {
    const view = describeCapture(
      capture({ textPreview: `<script>alert(1)</script>\n\n${'x'.repeat(900)}` })
    )
    expect(view.preview.startsWith('<script>alert(1)</script> xxx')).toBe(true)
    expect(Array.from(view.preview).length).toBeLessThanOrEqual(501)
    expect(view.preview).not.toContain('\n')
  })

  it('has no preview when the text lives in a file', () => {
    const view = describeCapture(capture({ textPreview: null, textInFile: true }))
    expect(view.preview).toBe('')
    expect(view.textInFile).toBe(true)
  })

  it('keeps a hash it cannot recognise out of the short form', () => {
    expect(describeCapture(capture({ sha256: 'zz' })).shortSha).toBe('')
  })
})

describe('sourceOpenAction', () => {
  it('loads the page of origin for a source that only holds PDFs', () => {
    expect(sourceOpenAction([capture({ kind: 'pdf' })])).toBe('origin')
    expect(sourceOpenAction([capture({ kind: 'pdf' }), capture({ kind: 'pdf', id: 'c2' })])).toBe(
      'origin'
    )
  })

  it('opens the address in the browser for pages and selections', () => {
    expect(sourceOpenAction([capture({ kind: 'page' })])).toBe('browser')
    expect(sourceOpenAction([capture({ kind: 'selection' })])).toBe('browser')
  })

  it('a page capture next to a PDF is still just the page', () => {
    expect(sourceOpenAction([capture({ kind: 'pdf' }), capture({ kind: 'page', id: 'c2' })])).toBe(
      'browser'
    )
  })

  it('has no PDF to speak of for a source with no captures', () => {
    expect(sourceOpenAction([])).toBe('browser')
  })
})

describe('a saved PDF', () => {
  it('can be viewed only when the capture is a PDF whose file is on disk', () => {
    expect(describeCapture(capture({ kind: 'pdf', filePresent: true })).canViewPdf).toBe(true)
    expect(describeCapture(capture({ kind: 'pdf', filePresent: false })).canViewPdf).toBe(false)
    expect(describeCapture(capture({ kind: 'pdf', filePresent: null })).canViewPdf).toBe(false)
    expect(describeCapture(capture({ kind: 'page', filePresent: true })).canViewPdf).toBe(false)
  })

  it('is asked for by capture id, never by a path', async () => {
    vi.mocked(invoke).mockClear()
    vi.mocked(invoke).mockResolvedValue('C:/data/web-captures/s/c.pdf')
    await expect(navegadorPdfFile('c1')).resolves.toBe('C:/data/web-captures/s/c.pdf')
    expect(invoke).toHaveBeenCalledWith('navegador_pdf_file', { captureId: 'c1' })
  })

  it('reads the codes of a PDF that cannot be opened', () => {
    expect(parseSourceError('file_missing: the saved PDF is not on disk')).toEqual({
      code: 'file_missing',
      detail: 'the saved PDF is not on disk',
    })
    expect(parseSourceError('not_a_pdf').code).toBe('not_a_pdf')
  })
})
