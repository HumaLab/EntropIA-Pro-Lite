import { beforeEach, describe, expect, it } from 'vitest'
import { locale } from './i18n'
import type { RagSource } from './rag'
import {
  bibliographyNoticeKey,
  isBibliographySource,
  citedPageNumber,
  libraryChoiceKey,
  locationText,
  locatorOf,
  passageHeading,
  passageMarks,
  passageWindow,
  selectedLibrariesAfterToggle,
  sourceScopeKey,
  withNoticeDetail,
  workLine,
} from './rag-scope'

function corpus(): RagSource {
  return {
    index: 1,
    assetId: 'asset-1',
    itemId: 'item-1',
    itemTitle: 'Entrevista 12',
    collectionId: 'col-1',
    collectionName: 'Historia oral',
    snippet: 'texto',
    score: 0.9,
    startSeconds: null,
    endSeconds: null,
    provenance: null,
  }
}

function biblio(location: { kind: 'pages' | 'paragraphs'; from: number; to: number } | null) {
  return {
    ...corpus(),
    index: 2,
    assetId: '',
    bibliography: {
      chunkId: 'chunk-1',
      itemKey: 'ABCD1234',
      libraryName: 'Mi biblioteca',
      libraryType: 'user',
      libraryNativeId: '0',
      authors: 'Bloch, Febvre',
      year: 1949,
      location,
    },
  } satisfies RagSource
}

beforeEach(() => {
  locale.set('es')
})

describe('source scope', () => {
  it('reads a source without the bibliography field as a corpus source', () => {
    expect(isBibliographySource(corpus())).toBe(false)
    expect(sourceScopeKey(corpus())).toBe('ragChat.scopeCorpus')
    expect(isBibliographySource({ ...corpus(), bibliography: null })).toBe(false)
  })

  it('reads a source with the bibliography field as Biblioteca', () => {
    const source = biblio({ kind: 'pages', from: 3, to: 3 })
    expect(isBibliographySource(source)).toBe(true)
    expect(sourceScopeKey(source)).toBe('ragChat.scopeBiblioteca')
  })
})

describe('locationText', () => {
  it('writes PDF pages as p. N and ranges as pp. a–b', () => {
    expect(locationText({ kind: 'pages', from: 3, to: 3 })).toBe('p. 3')
    expect(locationText({ kind: 'pages', from: 3, to: 4 })).toBe('pp. 3–4')
  })

  it('writes HTML snapshots as a paragraph range and never as a page', () => {
    expect(locationText({ kind: 'paragraphs', from: 2, to: 2 })).toBe('párr. 2')
    expect(locationText({ kind: 'paragraphs', from: 2, to: 3 })).toBe('párr. 2–3')
    expect(locationText({ kind: 'paragraphs', from: 1, to: 1 })).not.toMatch(/^p\. /)
  })

  it('follows the language', () => {
    locale.set('en')
    expect(locationText({ kind: 'paragraphs', from: 2, to: 3 })).toBe('paras. 2–3')
    expect(locationText({ kind: 'pages', from: 7, to: 7 })).toBe('p. 7')
  })

  it('says nothing when the location is unknown', () => {
    expect(locationText(null)).toBe('')
    expect(locationText(undefined)).toBe('')
  })
})

describe('workLine', () => {
  it('joins authors and year, skipping what is missing', () => {
    expect(workLine({ authors: 'Bloch, Febvre', year: 1949 })).toBe('Bloch, Febvre · 1949')
    expect(workLine({ authors: '', year: 1949 })).toBe('1949')
    expect(workLine({ authors: 'Bloch', year: null })).toBe('Bloch')
    expect(workLine({ authors: ' ', year: null })).toBe('')
  })
})

describe('bibliographyNoticeKey', () => {
  it('maps every backend code to its own message and ignores the unknown', () => {
    expect(bibliographyNoticeKey('no_library_synced')).toBe('ragChat.biblioNotice.noLibrarySynced')
    expect(bibliographyNoticeKey('no_embeddings')).toBe('ragChat.biblioNotice.noEmbeddings')
    expect(bibliographyNoticeKey('embedding_unavailable')).toBe(
      'ragChat.biblioNotice.embeddingUnavailable'
    )
    expect(bibliographyNoticeKey('failed')).toBe('ragChat.biblioNotice.failed')
    expect(bibliographyNoticeKey('whatever')).toBe('ragChat.biblioNotice.failed')
    expect(bibliographyNoticeKey(null)).toBeNull()
    expect(bibliographyNoticeKey(undefined)).toBeNull()
  })
})

describe('withNoticeDetail', () => {
  it('appends the cause only where something broke', () => {
    expect(withNoticeDetail('Falló.', 'failed', 'sql_error: tabla')).toBe(
      'Falló. (sql_error: tabla)'
    )
    expect(withNoticeDetail('Sin vector.', 'embedding_unavailable', ' sin clave ')).toBe(
      'Sin vector. (sin clave)'
    )
    expect(withNoticeDetail('Falló.', 'whatever', 'x')).toBe('Falló. (x)')
  })

  it('leaves self-explanatory notices and empty details as worded', () => {
    expect(withNoticeDetail('A.', 'no_embeddings', 'x')).toBe('A.')
    expect(withNoticeDetail('B.', 'no_library_synced', 'x')).toBe('B.')
    expect(withNoticeDetail('C.', 'failed', null)).toBe('C.')
    expect(withNoticeDetail('D.', 'failed', '  ')).toBe('D.')
    expect(withNoticeDetail('E.', null, 'x')).toBe('E.')
  })
})

describe('library selection', () => {
  const all = [
    { libraryType: 'user', libraryId: '0' },
    { libraryType: 'group', libraryId: '77' },
  ]

  it('keys a library by type and id, because user 0 and group 0 differ', () => {
    expect(libraryChoiceKey({ libraryType: 'user', libraryId: '0' })).not.toBe(
      libraryChoiceKey({ libraryType: 'group', libraryId: '0' })
    )
  })

  it('unchecking one library from "all" selects the others explicitly', () => {
    expect(selectedLibrariesAfterToggle(null, all, all[0]!)).toEqual([all[1]])
  })

  it('checking back every library returns to "all" (null)', () => {
    expect(selectedLibrariesAfterToggle([all[1]!], all, all[0]!)).toBeNull()
  })

  it('never leaves an empty selection: the last library stays checked', () => {
    expect(selectedLibrariesAfterToggle([all[1]!], all, all[1]!)).toEqual([all[1]])
  })
})

describe('passageWindow', () => {
  it('marks the cited range inside a bounded window of context', () => {
    const text = `${'a'.repeat(100)}CITADO${'b'.repeat(100)}`
    const view = passageWindow(text, [[100, 106]], 10)
    expect(view.segments).toEqual([
      { text: 'a'.repeat(10), marked: false },
      { text: 'CITADO', marked: true },
      { text: 'b'.repeat(10), marked: false },
    ])
    expect(view.truncatedBefore).toBe(true)
    expect(view.truncatedAfter).toBe(true)
  })

  it('does not truncate when everything fits', () => {
    const view = passageWindow('uno dos tres', [[4, 7]], 50)
    expect(view.segments).toEqual([
      { text: 'uno ', marked: false },
      { text: 'dos', marked: true },
      { text: ' tres', marked: false },
    ])
    expect(view.truncatedBefore).toBe(false)
    expect(view.truncatedAfter).toBe(false)
  })

  it('counts Unicode scalars like the backend, not UTF-16 units', () => {
    // The emoji is one scalar but two UTF-16 units: offsets 2..6 are "dos!".
    const view = passageWindow('😀 dos! cola', [[2, 6]], 50)
    expect(view.segments.filter((segment) => segment.marked)).toEqual([
      { text: 'dos!', marked: true },
    ])
  })

  it('shows plain text when no range is given', () => {
    const view = passageWindow('texto sin rango', [], 5)
    expect(view.segments).toEqual([{ text: 'texto', marked: false }])
    expect(view.truncatedAfter).toBe(true)
  })

  it('merges overlapping ranges instead of marking twice', () => {
    const view = passageWindow(
      'abcdefghij',
      [
        [1, 5],
        [3, 8],
      ],
      50
    )
    expect(view.segments.filter((segment) => segment.marked)).toEqual([
      { text: 'bcdefgh', marked: true },
    ])
  })
})

describe('locatorOf', () => {
  it('cites a PDF by page and an HTML snapshot by paragraph', () => {
    expect(locatorOf({ kind: 'pages', from: 3, to: 3 })).toEqual({
      locator: '3',
      locatorType: 'page',
    })
    expect(locatorOf({ kind: 'pages', from: 3, to: 4 })).toEqual({
      locator: '3-4',
      locatorType: 'page',
    })
    expect(locatorOf({ kind: 'paragraphs', from: 2, to: 3 })).toEqual({
      locator: '2-3',
      locatorType: 'paragraph',
    })
  })

  it('adds no locator when the location is unknown', () => {
    expect(locatorOf(null)).toBeNull()
    expect(locatorOf(undefined)).toBeNull()
  })
})

describe('passageHeading', () => {
  it('names the work, where in it and the library, skipping what is missing', () => {
    expect(
      passageHeading(
        { authors: 'Bloch', year: 1949, libraryName: 'Mi biblioteca' },
        { kind: 'pages', from: 3, to: 4 }
      )
    ).toBe('Bloch · 1949 · pp. 3–4 · Mi biblioteca')
    expect(passageHeading({ authors: '', year: null, libraryName: 'Mi biblioteca' }, null)).toBe(
      'Mi biblioteca'
    )
  })
})

describe('passageMarks', () => {
  const page = (pageNumber: number, text: string, highlights: Array<[number, number]> = []) => ({
    pageNumber,
    text,
    highlights,
  })

  it('keeps the stored ranges as the only marks when the catalog has them', () => {
    const pages = [page(2, 'alfa beta gamma', [[5, 9]]), page(3, 'beta aparece aqui tambien')]
    expect(passageMarks(pages, 'beta')).toEqual([[[5, 9]], []])
  })

  it('falls back to an exact search of the passage on the page when no range is stored', () => {
    // The emoji is two UTF-16 units and one scalar: offsets count scalars.
    const pages = [page(4, '😀 antes. EL TEXTO CITADO. despues')]
    expect(passageMarks(pages, 'EL TEXTO CITADO.')).toEqual([[[9, 25]]])
  })

  it('never marks text it cannot find exactly', () => {
    const pages = [page(4, 'una pagina sin la frase')]
    expect(passageMarks(pages, 'texto que no esta')).toEqual([[]])
    expect(passageMarks(pages, '   ')).toEqual([[]])
  })

  it('does not search the fallback when any page carries stored ranges', () => {
    const pages = [page(1, 'cita aqui', [[0, 4]]), page(2, 'cita aqui')]
    expect(passageMarks(pages, 'cita')).toEqual([[[0, 4]], []])
  })
})

describe('citedPageNumber', () => {
  it('is the first page that carries the cited range', () => {
    expect(
      citedPageNumber([
        { pageNumber: 4, text: 'a', highlights: [] },
        { pageNumber: 5, text: 'b', highlights: [[0, 1]] },
      ])
    ).toBe(5)
  })

  it('falls back to the first page, then to page 1', () => {
    expect(citedPageNumber([{ pageNumber: 7, text: 'a', highlights: [] }])).toBe(7)
    expect(citedPageNumber([])).toBe(1)
  })
})
