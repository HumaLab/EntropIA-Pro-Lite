import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WritingZoteroStore } from './writing-zotero'

/**
 * The Zotero tab's state (plan-editor.md §11.3).
 *
 * What is asserted here is the honesty of what reaches the screen. The backend
 * already refuses to claim Zotero is closed or absent; this store must not
 * reintroduce either claim by flattening a failure into an empty list, which is
 * the easy mistake — an unreachable library and a library with no matches look
 * identical if all you keep is the results.
 */

const mockInvoke = vi.mocked(invoke)

const GINZBURG = JSON.stringify({
  id: 'ABCD1234',
  type: 'book',
  title: 'Il formaggio e i vermi',
  author: [{ family: 'Ginzburg', given: 'Carlo' }],
  issued: { 'date-parts': [[1976]] },
})

const DARNTON = JSON.stringify({
  id: 'EFGH5678',
  type: 'book',
  title: 'The Great Cat Massacre',
  author: [{ family: 'Darnton', given: 'Robert' }],
  issued: { 'date-parts': [[1984]] },
})

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('probing', () => {
  it('holds what the probe said and claims nothing more', async () => {
    mockInvoke.mockResolvedValue({ state: 'api_disabled' } as never)
    const store = new WritingZoteroStore()

    await store.probe()

    expect(store.snapshot.status).toEqual({ state: 'api_disabled' })
    expect(store.snapshot.probing).toBe(false)
  })

  /**
   * The probe failing is our problem, not a diagnosis of Zotero. Recording it
   * as a state would be exactly the false claim §11.3 forbids.
   */
  it('says nothing about the library when the probe itself fails', async () => {
    mockInvoke.mockRejectedValue(new Error('the command is not registered'))
    const store = new WritingZoteroStore()

    await store.probe()

    expect(store.snapshot.status).toBeNull()
    expect(store.snapshot.error).toContain('not registered')
  })
})

describe('reading the library', () => {
  it('reads a page and describes each work enough to choose it', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG, DARNTON],
      version: 140,
      total: 2,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries).toHaveLength(2)
    expect(store.snapshot.entries[0]).toMatchObject({
      key: 'ABCD1234',
      title: 'Il formaggio e i vermi',
      authors: 'Ginzburg',
      year: '1976',
    })
  })

  /** The CSL-JSON is what gets cited, so it must survive being listed. */
  it('keeps the untouched CSL-JSON beside what it read from it', async () => {
    mockInvoke.mockResolvedValue({ items: [GINZBURG], version: 1, total: 1, has_more: false } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries[0]?.csl_json).toBe(GINZBURG)
  })

  it('follows the pages the library says are there', async () => {
    mockInvoke
      .mockResolvedValueOnce({ items: [GINZBURG], version: 1, total: 2, has_more: true } as never)
      .mockResolvedValueOnce({ items: [DARNTON], version: 1, total: 2, has_more: false } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.loaded).toBe(2)
    expect(mockInvoke).toHaveBeenCalledTimes(2)
    expect(mockInvoke).toHaveBeenLastCalledWith('writing_zotero_items', {
      library: '0',
      start: 100,
      limit: 100,
      query: null,
    })
  })

  /**
   * The easy mistake, and the one that matters. Someone hunting a reference
   * must be able to tell "Zotero is switched off" from "you have not got that
   * book".
   */
  it('does not turn an unreachable library into an empty one', async () => {
    mockInvoke.mockRejectedValue({
      code: 'zotero_api_disabled',
      message: 'the local API is off',
    })
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries).toEqual([])
    expect(store.snapshot.error).toContain('the local API is off')
    expect(store.snapshot.loading).toBe(false)
  })

  /** A library item we cannot parse is the library's business, not ours. */
  it('skips an item it cannot read rather than showing a blank row', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG, 'no es json'],
      version: 1,
      total: 2,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries).toHaveLength(1)
  })
})

describe('searching what was read', () => {
  async function loaded() {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG, DARNTON],
      version: 1,
      total: 2,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()
    await store.load()
    mockInvoke.mockClear()
    return store
  }

  it('matches on title, author and year', async () => {
    const store = await loaded()

    store.search('formaggio')
    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['ABCD1234'])

    store.search('Darnton')
    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['EFGH5678'])

    store.search('1984')
    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['EFGH5678'])
  })

  it('is case-insensitive, like every other search in the app', async () => {
    const store = await loaded()

    store.search('GINZBURG')

    expect(store.snapshot.entries).toHaveLength(1)
  })

  /** Typing must not re-read the library; that is what holding it is for. */
  it('filters what was read instead of asking again', async () => {
    const store = await loaded()

    store.search('formaggio')

    expect(mockInvoke).not.toHaveBeenCalled()
  })

  it('shows everything again when the search is cleared', async () => {
    const store = await loaded()
    store.search('formaggio')

    store.search('')

    expect(store.snapshot.entries).toHaveLength(2)
  })
})

/**
 * Searching the library rather than the copy of it that happened to be read.
 *
 * This is the bug that made an author who *is* in the library impossible to
 * find: the list was filtered, the library was not asked, and whatever had not
 * been read could not be matched. Nothing on screen said so.
 */
describe('searching the library itself', () => {
  it('asks Zotero rather than filtering what was already read', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG],
      version: 1,
      total: 1,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.searchLibrary('Acha')

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_items', {
      library: '0',
      start: 0,
      limit: 100,
      query: 'Acha',
    })
  })

  it('sends no query at all when the box is emptied', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG],
      version: 1,
      total: 1,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.searchLibrary('   ')

    expect(mockInvoke).toHaveBeenCalledWith(
      'writing_zotero_items',
      expect.objectContaining({ query: null })
    )
  })

  it('keeps reporting what the library says it holds', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG],
      version: 1,
      total: 137,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.searchLibrary('Acha')

    expect(store.snapshot.total).toBe(137)
  })
})
