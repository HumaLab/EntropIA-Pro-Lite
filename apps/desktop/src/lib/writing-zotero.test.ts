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
    mockInvoke.mockResolvedValue({
      items: [GINZBURG],
      version: 1,
      total: 1,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries[0]?.csl_json).toBe(GINZBURG)
  })

  it('follows the pages the library says are there', async () => {
    mockInvoke
      .mockResolvedValueOnce({ items: [GINZBURG], version: 1, total: 150, has_more: true } as never)
      .mockResolvedValueOnce({ items: [DARNTON], version: 1, total: 150, has_more: false } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.loaded).toBe(2)
    expect(mockInvoke).toHaveBeenCalledTimes(2)
    expect(mockInvoke).toHaveBeenLastCalledWith('writing_zotero_items', {
      library: '0',
      start: 100,
      limit: 100,
    })
  })

  /**
   * The count Zotero sends is what makes the rest of the library plannable, so
   * the pages behind the first are asked for together rather than one after
   * another — a library of thousands was twenty round trips in a row.
   */
  it('asks for the pages behind the first one at the same time', async () => {
    const pending: Array<() => void> = []
    mockInvoke.mockImplementation(((_cmd: string, args: { start: number }) => {
      if (args.start === 0) {
        return Promise.resolve({ items: [GINZBURG], version: 1, total: 450, has_more: true })
      }
      return new Promise((resolve) => {
        pending.push(() => resolve({ items: [], version: 1, total: 450, has_more: false }))
      })
    }) as never)
    const store = new WritingZoteroStore()

    const reading = store.load()
    await vi.waitFor(() => expect(pending.length).toBeGreaterThan(1))
    expect(store.snapshot.loading).toBe(true)
    while (store.snapshot.loading) {
      for (const release of pending.splice(0)) release()
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
    await reading

    const starts = mockInvoke.mock.calls.map(([, args]) => (args as { start: number }).start)
    expect(starts.sort((a, b) => a - b)).toEqual([0, 100, 200, 300, 400])
  })

  /** A ceiling is how references went missing: the whole library is read. */
  it('reads a library larger than the old ceiling to the end', async () => {
    mockInvoke.mockImplementation(((_cmd: string, args: { start: number }) =>
      Promise.resolve({
        items: Array.from({ length: 100 }, (_, i) =>
          JSON.stringify({ id: `K${args.start + i}`, title: `Work ${args.start + i}` })
        ),
        version: 1,
        total: 2_500,
        has_more: args.start + 100 < 2_500,
      })) as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.loaded).toBe(2_500)
  })

  it('keeps the order the library gave, whatever order the pages arrive in', async () => {
    mockInvoke.mockImplementation(((_cmd: string, args: { start: number }) => {
      const first = args.start === 0
      // The later page answers first.
      return new Promise((resolve) =>
        setTimeout(
          () =>
            resolve({
              items: [first ? GINZBURG : DARNTON],
              version: 1,
              total: 150,
              has_more: first,
            }),
          first ? 0 : 5
        )
      )
    }) as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['ABCD1234', 'EFGH5678'])
  })

  /**
   * A citation key is the owner's to choose, and two works can end up with the
   * same one. Collapsing on it hid a real work: 2,805 in Zotero, 2,804 listed.
   */
  it('lists two different works that share a citation key', async () => {
    const post = (title: string) =>
      JSON.stringify({ id: 'karpathy-vibe-coding-2025', type: 'post-weblog', title })
    mockInvoke.mockResolvedValue({
      items: [post('Vibe Coding'), post("There's a new kind of coding")],
      version: 1,
      total: 2,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.loaded).toBe(2)
    expect(store.snapshot.entries).toHaveLength(2)
  })

  /** The same work on two pages is still one work to choose. */
  it('lists a work once even if two pages both carry it', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG],
      version: 1,
      total: 150,
      has_more: true,
    } as never)
    const store = new WritingZoteroStore()

    await store.load()

    expect(store.snapshot.entries).toHaveLength(1)
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

describe('opening the tab', () => {
  function zoteroWithOneWork() {
    mockInvoke.mockImplementation(((cmd: string) =>
      Promise.resolve(
        cmd === 'writing_zotero_probe'
          ? { state: 'available' }
          : { items: [GINZBURG], version: 1, total: 1, has_more: false }
      )) as never)
  }

  /** Nobody should have to press a button to be allowed to cite. */
  it('reads the library on its own once Zotero answers', async () => {
    zoteroWithOneWork()
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.loaded).toBe(1)
  })

  it('does not try to read a library Zotero refused to open', async () => {
    mockInvoke.mockResolvedValue({ state: 'api_disabled' } as never)
    const store = new WritingZoteroStore()

    await store.connect()

    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(store.snapshot.loaded).toBe(0)
  })

  /** Switching tabs back and forth must not read the library again each time. */
  it('reads the library once however often the tab is opened', async () => {
    zoteroWithOneWork()
    const store = new WritingZoteroStore()

    await Promise.all([store.connect(), store.connect()])
    await store.connect()

    const reads = mockInvoke.mock.calls.filter(([cmd]) => cmd === 'writing_zotero_items')
    expect(reads).toHaveLength(1)
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

    await store.searchLibrary(' Acha ')

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_search', {
      library: '0',
      query: 'Acha',
    })
  })

  /** An empty box is a request for the library, not a search for nothing. */
  it('reads the library rather than searching when the box is emptied', async () => {
    mockInvoke.mockResolvedValue({
      items: [GINZBURG],
      version: 1,
      total: 1,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()

    await store.searchLibrary('   ')

    const commands = mockInvoke.mock.calls.map(([cmd]) => cmd)
    expect(commands).toEqual(['writing_zotero_items'])
  })

  /**
   * Asking Zotero used to replace the library with the answer, so clearing the
   * box afterwards showed only what the last search found.
   */
  it('leaves the library it read in place', async () => {
    mockInvoke.mockResolvedValueOnce({
      items: [GINZBURG, DARNTON],
      version: 1,
      total: 2,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()
    await store.load()
    mockInvoke.mockResolvedValueOnce({
      items: [GINZBURG],
      version: 1,
      total: 1,
      has_more: false,
    } as never)

    await store.searchLibrary('formaggio')
    mockInvoke.mockClear()
    store.search('')

    expect(store.snapshot.entries).toHaveLength(2)
    expect(mockInvoke).not.toHaveBeenCalled()
  })

  /** Zotero also searches full text; what it finds there joins the list. */
  it('adds what only Zotero found to what the list already matched', async () => {
    const ACHA = JSON.stringify({ id: 'IJKL9012', type: 'book', title: 'Historia social' })
    mockInvoke.mockResolvedValueOnce({
      items: [GINZBURG, DARNTON],
      version: 1,
      total: 2,
      has_more: false,
    } as never)
    const store = new WritingZoteroStore()
    await store.load()
    mockInvoke.mockResolvedValueOnce({
      items: [GINZBURG, ACHA],
      version: 1,
      total: 2,
      has_more: false,
    } as never)

    await store.searchLibrary('Ginzburg')

    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['ABCD1234', 'IJKL9012'])
  })

  /** An answer that arrives after the box moved on answers nothing. */
  it('drops an answer for a search that is no longer in the box', async () => {
    let answer: (value: unknown) => void = () => {}
    mockInvoke.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve
      }) as never
    )
    const store = new WritingZoteroStore()

    const searching = store.searchLibrary('Ginzburg')
    store.search('Darnton')
    answer({ items: [GINZBURG], version: 1, total: 1, has_more: false })
    await searching

    expect(store.snapshot.query).toBe('Darnton')
    expect(store.snapshot.entries).toEqual([])
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
