import { describe, expect, it } from 'vitest'
import {
  describeCaptureDraft,
  describeDownload,
  formatBytes,
  parseCaptureError,
  previewText,
  shortHash,
  upsertDownload,
  type CaptureDraft,
  type DownloadDraft,
} from './navegador-capture'

const SHA = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'

function draft(overrides: Partial<CaptureDraft> = {}): CaptureDraft {
  return {
    kind: 'page',
    finalUrl: 'https://example.com/a',
    title: 'A title',
    canonicalUrl: null,
    siteName: null,
    lang: 'en',
    text: 'Some readable text',
    quote: null,
    quotePrefix: null,
    quoteSuffix: null,
    htmlBytes: 2048,
    hashOf: 'html',
    sha256: SHA,
    truncated: false,
    accessedAt: '2026-09-30T12:00:00Z',
    ...overrides,
  }
}

function download(overrides: Partial<DownloadDraft> = {}): DownloadDraft {
  return {
    id: 'id-1',
    url: 'https://example.com/paper.pdf',
    fileName: 'paper.pdf',
    size: 1_500_000,
    sha256: SHA,
    accessedAt: '2026-09-30T12:00:00Z',
    status: 'ready',
    reason: null,
    ...overrides,
  }
}

describe('shortHash', () => {
  it('keeps the first characters so two captures can be told apart', () => {
    expect(shortHash(SHA)).toBe('ba7816bf8f01')
    expect(shortHash(SHA, 8)).toBe('ba7816bf')
  })

  it('answers an empty string for anything that is not a hash', () => {
    expect(shortHash('')).toBe('')
    expect(shortHash(null)).toBe('')
    expect(shortHash('not a hash')).toBe('')
  })
})

describe('previewText', () => {
  it('collapses whitespace and trims', () => {
    expect(previewText('  one\n\n two\t three  ')).toBe('one two three')
  })

  it('cuts long text at the limit and says so', () => {
    const preview = previewText('a'.repeat(600), 500)
    expect(preview).toBe(`${'a'.repeat(500)}…`)
  })

  it('leaves text at the limit alone', () => {
    expect(previewText('a'.repeat(500), 500)).toBe('a'.repeat(500))
  })

  it('does not split a surrogate pair', () => {
    const preview = previewText('😀'.repeat(10), 3)
    expect(preview).toBe('😀😀😀…')
  })

  it('handles nothing', () => {
    expect(previewText('')).toBe('')
    expect(previewText(null)).toBe('')
  })
})

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [512, '512 B'],
    [1024, '1.0 KB'],
    [1536, '1.5 KB'],
    [1_500_000, '1.4 MB'],
    [104_857_600, '100.0 MB'],
    [3 * 1024 ** 3, '3.0 GB'],
  ])('formats %d as %s', (bytes, text) => {
    expect(formatBytes(bytes)).toBe(text)
  })

  it('does not print nonsense for bad input', () => {
    expect(formatBytes(-1)).toBe('0 B')
    expect(formatBytes(Number.NaN)).toBe('0 B')
  })
})

describe('parseCaptureError', () => {
  it('reads the stable code and the detail the backend sends', () => {
    expect(parseCaptureError('no_selection')).toEqual({ code: 'no_selection', detail: null })
    expect(parseCaptureError('script_failed: TypeError: x')).toEqual({
      code: 'script_failed',
      detail: 'TypeError: x',
    })
  })

  it('accepts an Error or a rejected string', () => {
    expect(parseCaptureError(new Error('timeout')).code).toBe('timeout')
    expect(parseCaptureError('not_open').code).toBe('not_open')
  })

  it('files anything it does not know under unknown, keeping the message', () => {
    expect(parseCaptureError('The embedded browser is not available in this build')).toEqual({
      code: 'unknown',
      detail: 'The embedded browser is not available in this build',
    })
    expect(parseCaptureError(42)).toEqual({ code: 'unknown', detail: '42' })
  })
})

describe('describeCaptureDraft', () => {
  it('describes a page: what the person is about to keep', () => {
    const view = describeCaptureDraft(draft())
    expect(view.kind).toBe('page')
    expect(view.title).toBe('A title')
    expect(view.finalUrl).toBe('https://example.com/a')
    expect(view.accessedAt).toBe('2026-09-30T12:00:00Z')
    expect(view.textLength).toBe('Some readable text'.length)
    expect(view.htmlBytes).toBe(2048)
    expect(view.shortSha).toBe('ba7816bf8f01')
    expect(view.hashOf).toBe('html')
    expect(view.preview).toBe('Some readable text')
    expect(view.truncated).toBe(false)
  })

  it('previews the quote for a selection, not the page text', () => {
    const view = describeCaptureDraft(
      draft({ kind: 'selection', text: 'the quote', quote: 'the quote', hashOf: 'quote' })
    )
    expect(view.preview).toBe('the quote')
    expect(view.kind).toBe('selection')
  })

  it('bounds the preview to 500 characters', () => {
    const view = describeCaptureDraft(draft({ text: 'w '.repeat(1000) }))
    expect(view.preview.length).toBeLessThanOrEqual(501)
    expect(view.preview.endsWith('…')).toBe(true)
  })

  it('falls back to the address when the page has no title', () => {
    expect(describeCaptureDraft(draft({ title: null })).title).toBe('https://example.com/a')
  })

  it('carries the truncated flag', () => {
    expect(describeCaptureDraft(draft({ truncated: true })).truncated).toBe(true)
  })
})

describe('describeDownload', () => {
  it('shows a verified pdf', () => {
    const view = describeDownload(download())
    expect(view.fileName).toBe('paper.pdf')
    expect(view.size).toBe('1.4 MB')
    expect(view.shortSha).toBe('ba7816bf8f01')
    expect(view.status).toBe('ready')
    expect(view.reason).toBeNull()
  })

  it('has no size or hash to show while it downloads', () => {
    const view = describeDownload(download({ status: 'downloading', size: null, sha256: null }))
    expect(view.size).toBe('')
    expect(view.shortSha).toBe('')
  })

  it('names the reason for a rejection', () => {
    const view = describeDownload(
      download({ status: 'rejected', reason: 'not_pdf', size: 10, sha256: null })
    )
    expect(view.status).toBe('rejected')
    expect(view.reason).toBe('not_pdf')
  })
})

describe('upsertDownload', () => {
  it('adds a new download at the top', () => {
    const list = upsertDownload([download({ id: 'a' })], download({ id: 'b' }))
    expect(list.map((d) => d.id)).toEqual(['b', 'a'])
  })

  it('replaces a download in place when its status changes', () => {
    const start = [download({ id: 'b' }), download({ id: 'a', status: 'downloading' })]
    const list = upsertDownload(start, download({ id: 'a', status: 'ready' }))
    expect(list.map((d) => [d.id, d.status])).toEqual([
      ['b', 'ready'],
      ['a', 'ready'],
    ])
  })

  it('keeps only the most recent few', () => {
    let list: DownloadDraft[] = []
    for (let i = 0; i < 8; i++) list = upsertDownload(list, download({ id: `d${i}` }), 5)
    expect(list.map((d) => d.id)).toEqual(['d7', 'd6', 'd5', 'd4', 'd3'])
  })

  it('does not mutate its input', () => {
    const start = [download({ id: 'a' })]
    upsertDownload(start, download({ id: 'b' }))
    expect(start).toHaveLength(1)
  })
})
