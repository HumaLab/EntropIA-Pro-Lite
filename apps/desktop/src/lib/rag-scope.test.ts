import { beforeEach, describe, expect, it } from 'vitest'
import { locale } from './i18n'
import type { RagSource } from './rag'
import {
  bibliographyNoticeKey,
  isBibliographySource,
  libraryChoiceKey,
  locationText,
  passageWindow,
  selectedLibrariesAfterToggle,
  sourceScopeKey,
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
