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
const create = vi.fn()
const searchGlobal = vi.fn()
const findByItem = vi.fn()
const fakeStore = {
  notes: { search, findById, create },
  items: { searchGlobal },
  assets: { findByItem },
}

function makeStore() {
  return new WritingNotesStore(() => fakeStore as never)
}

beforeEach(() => {
  search.mockReset().mockResolvedValue([HIT])
  findById.mockReset().mockResolvedValue({ id: 'n1', content: 'los obreros del filet' })
  create.mockReset()
  searchGlobal.mockReset().mockResolvedValue([])
  findByItem.mockReset().mockResolvedValue([])
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

/**
 * Creating a note from a passage of the manuscript (plan-editor.md §13.1).
 *
 * The constraint that shapes this: `notes.item_id` is NOT NULL and §13.1
 * forbids both relaxing it and minting a fictitious item to carry a
 * manuscript-only note. So the writer chooses a real destination, and these
 * assert that nothing here invents one.
 */
describe('creating a note from a selection', () => {
  it('writes the passage to the item that was chosen', async () => {
    create.mockResolvedValue({ id: 'n9', itemId: 'it1', content: 'el pasaje' })
    const store = makeStore()

    const note = await store.createFromSelection({ text: '  el pasaje  ', itemId: 'it1' })

    expect(create).toHaveBeenCalledWith({ itemId: 'it1', assetId: null, content: 'el pasaje' })
    expect(note?.id).toBe('n9')
  })

  /**
   * A note with no asset is an item-level note, and `findByAsset` returns those
   * for **every** asset of the item by design. A passage filed against a
   * forty-page scan therefore appeared forty times, once under each page.
   */
  it('files the passage against one asset so it is not repeated under every page', async () => {
    findByItem.mockResolvedValue([{ id: 'as1' }, { id: 'as2' }, { id: 'as3' }])
    create.mockResolvedValue({ id: 'n9' })
    const store = makeStore()

    await store.createFromSelection({ text: 'el pasaje', itemId: 'it1' })

    expect(create).toHaveBeenCalledWith({ itemId: 'it1', assetId: 'as1', content: 'el pasaje' })
  })

  /** The repository orders by path, so "first" is the same asset every time. */
  it('takes the first asset the repository reports, not whichever came back', async () => {
    findByItem.mockResolvedValue([{ id: 'as-a' }, { id: 'as-b' }])
    create.mockResolvedValue({ id: 'n9' })
    const store = makeStore()

    await store.createFromSelection({ text: 'el pasaje', itemId: 'it1' })

    expect(create).toHaveBeenCalledWith(expect.objectContaining({ assetId: 'as-a' }))
  })

  /**
   * Not a fallback but the accurate answer: with no assets there is nothing for
   * the note to be repeated across.
   */
  it('leaves the note at item level when the item has no assets', async () => {
    findByItem.mockResolvedValue([])
    create.mockResolvedValue({ id: 'n9' })
    const store = makeStore()

    await store.createFromSelection({ text: 'el pasaje', itemId: 'it1' })

    expect(create).toHaveBeenCalledWith(expect.objectContaining({ assetId: null }))
  })

  /** A caller that names the asset is not second-guessed. */
  it('keeps an asset the caller chose', async () => {
    findByItem.mockResolvedValue([{ id: 'as1' }])
    create.mockResolvedValue({ id: 'n9' })
    const store = makeStore()

    await store.createFromSelection({ text: 'el pasaje', itemId: 'it1', assetId: 'as7' })

    expect(create).toHaveBeenCalledWith(expect.objectContaining({ assetId: 'as7' }))
    expect(findByItem).not.toHaveBeenCalled()
  })

  /**
   * The note matters more than where it is filed: refusing to write it because
   * the assets could not be read would lose the passage just chosen.
   */
  it('still writes the note when the assets cannot be read', async () => {
    findByItem.mockRejectedValue(new Error('DB locked'))
    create.mockResolvedValue({ id: 'n9' })
    const store = makeStore()

    const note = await store.createFromSelection({ text: 'el pasaje', itemId: 'it1' })

    expect(note?.id).toBe('n9')
    expect(create).toHaveBeenCalledWith(expect.objectContaining({ assetId: null }))
  })

  it('refuses to invent a destination', async () => {
    const store = makeStore()

    expect(await store.createFromSelection({ text: 'el pasaje', itemId: '' })).toBeNull()
    expect(create).not.toHaveBeenCalled()
  })

  it('refuses to write an empty note', async () => {
    const store = makeStore()

    expect(await store.createFromSelection({ text: '   ', itemId: 'it1' })).toBeNull()
    expect(create).not.toHaveBeenCalled()
  })

  it('reports a failure rather than pretending the note exists', async () => {
    create.mockRejectedValue(new Error('FOREIGN KEY constraint failed'))
    const store = makeStore()

    expect(await store.createFromSelection({ text: 'el pasaje', itemId: 'it9' })).toBeNull()
    expect(store.snapshot.error).toContain('FOREIGN KEY')
  })
})

describe('choosing where a new note goes', () => {
  it('offers the items a search found', async () => {
    searchGlobal.mockResolvedValue([{ id: 'it1', title: 'Acta', collectionId: 'col1' }])
    const store = makeStore()

    await store.findTargets('acta')

    expect(store.snapshot.targets.map((item) => item.id)).toEqual(['it1'])
  })

  /**
   * §13.1: the manuscript's collections "ayudarán a filtrar el selector, pero
   * no reemplazarán la identidad del item".
   */
  it('narrows the offer to the manuscript collections without changing the items', async () => {
    searchGlobal.mockResolvedValue([
      { id: 'it1', title: 'Acta', collectionId: 'col1' },
      { id: 'it2', title: 'Padron', collectionId: 'col9' },
    ])
    const store = makeStore()
    store.setScope(['col1'])

    await store.findTargets('a')

    expect(store.snapshot.targets.map((item) => item.id)).toEqual(['it1'])
  })

  it('offers nothing until something is typed', async () => {
    const store = makeStore()

    await store.findTargets('   ')

    expect(searchGlobal).not.toHaveBeenCalled()
    expect(store.snapshot.targets).toEqual([])
  })
})
