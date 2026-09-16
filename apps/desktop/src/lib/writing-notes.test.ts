import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WritingNotesStore } from './writing-notes'

/**
 * The Notas tab's state (plan-editor.md §6.3, §13).
 *
 * The search itself belongs to `NoteRepo` and is tested there against the real
 * schema. What is asserted here is what this store adds: the manuscript's
 * collections as a scope, and reading a note back so a link can be checked
 * against the snapshot it holds.
 */

const HIT = {
  id: 'n1',
  itemId: 'it1',
  itemTitle: 'Acta del gremio',
  collectionId: 'col1',
  assetId: null,
  content: 'los obreros del filet',
  createdAt: 1,
  updatedAt: 1,
}

const search = vi.fn()
const findById = vi.fn()
const fakeStore = { notes: { search, findById } }

function makeStore() {
  return new WritingNotesStore(() => fakeStore as never)
}

beforeEach(() => {
  search.mockReset().mockResolvedValue([HIT])
  findById.mockReset().mockResolvedValue({ id: 'n1', content: 'los obreros del filet' })
})

describe('searching notes for a manuscript', () => {
  it('searches the whole corpus when the manuscript narrows nothing', async () => {
    const store = makeStore()

    await store.search('filet')

    expect(search).toHaveBeenCalledWith({ query: 'filet', collectionIds: undefined })
    expect(store.snapshot.results).toEqual([HIT])
  })

  /**
   * §13.1: the manuscript's associations "ayudarán a filtrar el selector". A
   * writer looking for evidence for *this* article is not looking through the
   * whole archive.
   */
  it('narrows to the manuscript collections once they are known', async () => {
    const store = makeStore()
    store.setScope(['col1', 'col2'])

    await store.search('filet')

    expect(search).toHaveBeenCalledWith({ query: 'filet', collectionIds: ['col1', 'col2'] })
  })

  /**
   * The opposite of what the repository does with an empty array, and
   * deliberately. There, an empty scope was asked for. Here, a manuscript with
   * no associated collection has simply not narrowed anything — showing it no
   * notes at all would be a worse answer than showing it the corpus.
   */
  it('treats no associated collection as the whole corpus, not as none', async () => {
    const store = makeStore()
    store.setScope([])

    await store.search('filet')

    expect(search).toHaveBeenCalledWith({ query: 'filet', collectionIds: undefined })
  })

  it('lists the scope when nothing was typed', async () => {
    const store = makeStore()

    await store.search('   ')

    expect(search).toHaveBeenCalledWith({ query: undefined, collectionIds: undefined })
  })

  it('ignores a slow search that finishes after a newer one', async () => {
    let releaseFirst: (value: unknown) => void = () => {}
    search
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            releaseFirst = resolve
          })
      )
      .mockResolvedValueOnce([{ ...HIT, id: 'nuevo' }])

    const store = makeStore()
    const slow = store.search('filet')
    await store.search('molino')
    releaseFirst([{ ...HIT, id: 'viejo' }])
    await slow

    expect(store.snapshot.results.map((note) => note.id)).toEqual(['nuevo'])
  })

  it('reports a failure instead of leaving the panel searching', async () => {
    search.mockRejectedValue(new Error('no such table: notes'))
    const store = makeStore()

    await store.search('filet')

    expect(store.snapshot.searching).toBe(false)
    expect(store.snapshot.error).toContain('no such table')
  })
})

describe('reading a note back', () => {
  it('reports the note as it stands now', async () => {
    const store = makeStore()

    expect(await store.readNote('n1')).toEqual({
      exists: true,
      content: 'los obreros del filet',
    })
  })

  it('reports a deleted note as gone', async () => {
    findById.mockResolvedValue(null)
    const store = makeStore()

    expect(await store.readNote('n1')).toEqual({ exists: false, content: null })
  })

  /**
   * A read that failed is not a deletion. Telling a writer their note is gone
   * when it is sitting there is a worse lie than admitting the check could not
   * be made, so this answers "present, unreadable" and `resolveNoteLink` turns
   * that into `unverifiable`.
   */
  it('does not report a failed read as a deletion', async () => {
    findById.mockRejectedValue(new Error('database is locked'))
    const store = makeStore()

    expect(await store.readNote('n1')).toEqual({ exists: true, content: null })
  })
})

describe('opening a note', () => {
  it('opens one from the results and closes back', async () => {
    const store = makeStore()
    await store.search('filet')

    store.openNote('n1')
    expect(store.snapshot.open?.id).toBe('n1')

    store.closeNote()
    expect(store.snapshot.open).toBeNull()
  })

  it('opens nothing for an id that is not in the results', async () => {
    const store = makeStore()
    await store.search('filet')

    store.openNote('n9')

    expect(store.snapshot.open).toBeNull()
  })
})
