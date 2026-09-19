import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WritingCorpusStore, corpusPageLabel } from './writing-corpus'
import { SearchPreferences } from './search-preferences'

/**
 * Searching the corpus from inside Escritura (plan-editor.md §10.1).
 *
 * The plan is explicit that no second retrieval engine may be built: this
 * drives `store.fts`, the same index the item view searches. What is asserted
 * here is the part that is genuinely this store's own — that a result is
 * hydrated into something citable, and that a slow search cannot overwrite a
 * newer one.
 */

const ITEM = { id: 'it1', title: 'Molinos y molineros', collectionId: 'col1' }
const ASSETS = [
  { id: 'as1', itemId: 'it1', pageNumber: 1, type: 'pdf', path: '/a/1.png' },
  { id: 'as2', itemId: 'it1', pageNumber: 2, type: 'pdf', path: '/a/2.png' },
]

const search = vi.fn()
const findById = vi.fn()
const findByItem = vi.fn()
const findByAsset = vi.fn()
const findTranscription = vi.fn()

const fakeStore = {
  fts: { search },
  items: { findById },
  assets: { findByItem },
  extractions: { findByAsset },
  transcriptions: { findByAsset: findTranscription },
}

let fuzzySetting: string | null = null
const prefs = {
  get: vi.fn(async () => fuzzySetting),
  set: vi.fn(async (_key: string, value: string) => {
    fuzzySetting = value
  }),
}

function makeStore() {
  return new WritingCorpusStore(() => fakeStore as never, new SearchPreferences(prefs))
}

beforeEach(() => {
  fuzzySetting = null
  prefs.set.mockClear()
  search.mockReset().mockResolvedValue([{ itemId: 'it1', rank: -1.2 }])
  findById.mockReset().mockResolvedValue(ITEM)
  findByItem.mockReset().mockResolvedValue(ASSETS)
  findByAsset.mockReset().mockResolvedValue({ id: 'e1', assetId: 'as1', textContent: 'el molino' })
  findTranscription.mockReset().mockResolvedValue(null)
})

describe('searching', () => {
  it('hydrates each hit into an item someone could cite', async () => {
    const store = makeStore()

    await store.search('molino')

    // A document nothing was read from has no text to quote, so the corpus
    // search never offers it, however well its title matches.
    expect(search).toHaveBeenCalledWith('molino', expect.any(Number), {
      withTextOnly: true,
      fuzzy: true,
    })
    expect(store.snapshot.results).toEqual([
      { itemId: 'it1', title: 'Molinos y molineros', collectionId: 'col1', rank: -1.2 },
    ])
    expect(store.snapshot.searching).toBe(false)
  })

  it('carries the words an approximate hit was found as', async () => {
    search.mockResolvedValue([
      { itemId: 'it1', rank: -1.2, approximate: true, variants: ['molinso'] },
    ])
    const store = makeStore()

    await store.search('molinos')

    expect(store.snapshot.results[0]?.foundAs).toEqual(['molinso'])
  })

  it('searches exactly when approximate search was turned off', async () => {
    fuzzySetting = 'off'
    const store = makeStore()

    await store.search('molino')

    expect(search).toHaveBeenCalledWith('molino', expect.any(Number), {
      withTextOnly: true,
      fuzzy: false,
    })
    expect(store.snapshot.fuzzy).toBe(false)
  })

  it('remembers the switch and searches again with it', async () => {
    const store = makeStore()
    await store.search('molino')

    await store.setFuzzy(false)

    expect(prefs.set).toHaveBeenCalledWith('search_fuzzy', 'off')
    expect(search).toHaveBeenLastCalledWith('molino', expect.any(Number), {
      withTextOnly: true,
      fuzzy: false,
    })
    expect(store.snapshot.fuzzy).toBe(false)
  })

  /** A hit whose item is gone is dropped, not rendered as a blank row. */
  it('drops a hit whose item no longer exists', async () => {
    findById.mockResolvedValue(null)
    const store = makeStore()

    await store.search('molino')

    expect(store.snapshot.results).toEqual([])
  })

  it('clears the results for an empty query without asking the index', async () => {
    const store = makeStore()
    await store.search('molino')

    await store.search('   ')

    expect(store.snapshot.results).toEqual([])
    expect(search).toHaveBeenCalledTimes(1)
  })

  /**
   * Typing fast starts several searches. Without a guard the slowest one wins,
   * so the panel ends up showing results for a query the writer has already
   * moved on from.
   */
  it('ignores a slow search that finishes after a newer one', async () => {
    let releaseFirst: (value: unknown) => void = () => {}
    search
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            releaseFirst = resolve
          })
      )
      .mockResolvedValueOnce([{ itemId: 'nuevo', rank: -2 }])
    // Each id hydrates to its own title, so the assertion can actually tell
    // which search produced what. A blanket mock would pass either way.
    findById.mockImplementation(async (id: string) => ({ ...ITEM, id, title: `titulo de ${id}` }))

    const store = makeStore()
    const slow = store.search('molino')
    await store.search('molinero')
    releaseFirst([{ itemId: 'viejo', rank: -9 }])
    await slow

    expect(store.snapshot.results.map((row) => row.title)).toEqual(['titulo de nuevo'])
  })

  it('reports a failure instead of leaving the panel spinning', async () => {
    search.mockRejectedValue(new Error('fts is not ready'))
    const store = makeStore()

    await store.search('molino')

    expect(store.snapshot.searching).toBe(false)
    expect(store.snapshot.error).toContain('fts is not ready')
  })
})

describe('opening an item', () => {
  it('lists its pages in order, because a page is what gets cited', async () => {
    const store = makeStore()

    await store.openItem('it1')

    expect(store.snapshot.openItem?.itemId).toBe('it1')
    expect(store.snapshot.pages.map((page) => page.assetId)).toEqual(['as1', 'as2'])
    expect(store.snapshot.pages.map((page) => page.pageNumber)).toEqual([1, 2])
  })

  it('reads the extracted text of a page, which is what a citation quotes', async () => {
    const store = makeStore()
    await store.openItem('it1')

    await store.openPage('as1')

    expect(findByAsset).toHaveBeenCalledWith('as1')
    expect(store.snapshot.pageText).toBe('el molino')
  })

  it('reads the transcription of an audio, which the search already found words in', async () => {
    findByAsset.mockResolvedValue(null)
    findTranscription.mockResolvedValue({ id: 't1', assetId: 'as1', textContent: 'Crosito' })
    const store = makeStore()
    await store.openItem('it1')

    await store.openPage('as1')

    expect(store.snapshot.pageText).toBe('Crosito')
  })

  /** A page with no extraction is not an error; there is simply nothing to quote. */
  it('says nothing rather than failing when a page has no text', async () => {
    findByAsset.mockResolvedValue(null)
    const store = makeStore()
    await store.openItem('it1')

    await store.openPage('as1')

    expect(store.snapshot.pageText).toBe('')
    expect(store.snapshot.error).toBeNull()
  })

  it('closes back to the results', async () => {
    const store = makeStore()
    await store.search('molino')
    await store.openItem('it1')

    store.closeItem()

    expect(store.snapshot.openItem).toBeNull()
    expect(store.snapshot.pages).toEqual([])
    expect(store.snapshot.results).toHaveLength(1)
  })
})

describe('naming a page', () => {
  const t = (key: string, params?: Record<string, string>) =>
    params ? `${key}(${Object.values(params).join(',')})` : key

  it('numbers a page that has a number', () => {
    expect(corpusPageLabel({ assetId: 'a', pageNumber: 3, type: 'pdf' }, t)).toBe(
      'writing.corpusPage(3)'
    )
  })

  it('calls an audio what it offers to quote: its transcription', () => {
    expect(corpusPageLabel({ assetId: 'a', pageNumber: null, type: 'audio' }, t)).toBe(
      'writing.corpusPageAudio'
    )
  })

  it('says so when an image has no number', () => {
    expect(corpusPageLabel({ assetId: 'a', pageNumber: null, type: 'image' }, t)).toBe(
      'writing.corpusPageUnnumbered'
    )
  })
})
