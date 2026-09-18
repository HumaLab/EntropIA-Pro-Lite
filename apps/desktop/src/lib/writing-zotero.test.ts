import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WritingZoteroStore } from './writing-zotero'

/**
 * The Zotero tab's state (plan-editor.md §11.2, §11.3).
 *
 * Two things are asserted here. The first is the honesty of what reaches the
 * screen: the backend refuses to claim Zotero is closed or absent, and this
 * store must not reintroduce either claim by flattening a failure into an
 * empty list. The second is speed without loss: the library is listed from
 * the copy kept on disk at once, and Zotero is only asked what changed.
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

type Answers = Record<string, unknown | ((args: Record<string, unknown>) => unknown)>

/** Answers each command as Zotero and the copy on disk would. */
function answer(answers: Answers) {
  mockInvoke.mockImplementation(((cmd: string, args: Record<string, unknown>) => {
    if (!(cmd in answers)) return Promise.reject(new Error(`unexpected ${cmd}`))
    const reply = answers[cmd]
    return Promise.resolve(typeof reply === 'function' ? reply(args) : reply)
  }) as never)
}

const AVAILABLE = { state: 'available' }
const NO_COPY = { items: [], version: null }
const unchanged = { items: null, version: 1, fetched: 0, removed: 0 }
const synced = (items: string[]) => ({ items, version: 2, fetched: items.length, removed: 0 })
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([name]) => name === cmd)

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

describe('opening the tab', () => {
  /** The copy on disk is what makes the panel instant; Zotero comes after. */
  it('lists the copy kept on disk before Zotero answers', async () => {
    let probed: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_cached') {
        return Promise.resolve({ items: [GINZBURG, DARNTON], version: 1 })
      }
      return new Promise((resolve) => {
        probed = resolve
      })
    }) as never)
    const store = new WritingZoteroStore()

    const opening = store.connect()
    await vi.waitFor(() => expect(store.snapshot.loaded).toBe(2))
    expect(store.snapshot.status).toBeNull()

    probed({ state: 'api_disabled' })
    await opening
  })

  it('still lists the copy when Zotero cannot be reached', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['ABCD1234'])
    expect(calls('writing_zotero_sync')).toHaveLength(0)
  })

  /** Nobody should have to press a button to be allowed to cite. */
  it('brings the copy up to date once Zotero answers', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG], version: 1 },
      writing_zotero_probe: AVAILABLE,
      writing_zotero_sync: synced([GINZBURG, DARNTON]),
    })
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.loaded).toBe(2)
    expect(calls('writing_zotero_sync')[0]?.[1]).toEqual({ library: '0' })
  })

  it('keeps the list as it is when the library did not change', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG, DARNTON], version: 1 },
      writing_zotero_probe: AVAILABLE,
      writing_zotero_sync: unchanged,
    })
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.loaded).toBe(2)
    expect(store.snapshot.loading).toBe(false)
  })

  /** Switching tabs back and forth must not read the copy from disk each time. */
  it('reads the copy from disk once however often the tab is opened', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG], version: 1 },
      writing_zotero_probe: AVAILABLE,
      writing_zotero_sync: unchanged,
    })
    const store = new WritingZoteroStore()

    await store.connect()
    await store.connect()

    expect(calls('writing_zotero_cached')).toHaveLength(1)
  })

  it('does not sync twice at once', async () => {
    answer({
      writing_zotero_cached: NO_COPY,
      writing_zotero_probe: AVAILABLE,
      writing_zotero_sync: synced([GINZBURG]),
    })
    const store = new WritingZoteroStore()

    await Promise.all([store.connect(), store.connect()])

    expect(calls('writing_zotero_sync')).toHaveLength(1)
  })
})

describe('reading what Zotero sent', () => {
  async function listed(items: string[]) {
    answer({
      writing_zotero_cached: NO_COPY,
      writing_zotero_probe: AVAILABLE,
      writing_zotero_sync: synced(items),
    })
    const store = new WritingZoteroStore()
    await store.connect()
    return store
  }

  it('describes each work enough to choose it', async () => {
    const store = await listed([GINZBURG, DARNTON])

    expect(store.snapshot.entries[0]).toMatchObject({
      key: 'ABCD1234',
      title: 'Il formaggio e i vermi',
      authors: 'Ginzburg',
      year: '1976',
    })
  })

  /** The CSL-JSON is what gets cited, so it must survive being listed. */
  it('keeps the untouched CSL-JSON beside what it read from it', async () => {
    const store = await listed([GINZBURG])

    expect(store.snapshot.entries[0]?.csl_json).toBe(GINZBURG)
  })

  /** A library item we cannot parse is the library's business, not ours. */
  it('skips an item it cannot read rather than showing a blank row', async () => {
    const store = await listed([GINZBURG, 'no es json'])

    expect(store.snapshot.entries).toHaveLength(1)
  })

  /**
   * A citation key is the owner's to choose, and two works can end up with the
   * same one. Collapsing on it hid a real work: 2,805 in Zotero, 2,804 listed.
   */
  it('lists two different works that share a citation key', async () => {
    const post = (title: string) =>
      JSON.stringify({ id: 'karpathy-vibe-coding-2025', type: 'post-weblog', title })

    const store = await listed([post('Vibe Coding'), post("There's a new kind of coding")])

    expect(store.snapshot.entries).toHaveLength(2)
  })
})

describe('when the sync fails', () => {
  /** The copy on disk is still the library as it was; hiding it helps no one. */
  it('keeps listing the copy and says why it could not be updated', async () => {
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'writing_zotero_sync'
        ? Promise.reject({ code: 'zotero_timeout', message: 'the library took too long' })
        : Promise.resolve(
            cmd === 'writing_zotero_cached' ? { items: [GINZBURG], version: 1 } : AVAILABLE
          )) as never)
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.loaded).toBe(1)
    expect(store.snapshot.error).toContain('took too long')
    expect(store.snapshot.loading).toBe(false)
  })

  /**
   * The easy mistake, and the one that matters. Someone hunting a reference
   * must be able to tell "Zotero is switched off" from "you have not got that
   * book".
   */
  it('does not turn an unreadable library into an empty one', async () => {
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'writing_zotero_sync'
        ? Promise.reject({ code: 'zotero_api_disabled', message: 'the local API is off' })
        : Promise.resolve(cmd === 'writing_zotero_cached' ? NO_COPY : AVAILABLE)) as never)
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.entries).toEqual([])
    expect(store.snapshot.error).toContain('the local API is off')
  })
})

describe('searching what was read', () => {
  async function loaded() {
    answer({
      writing_zotero_cached: { items: [GINZBURG, DARNTON], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()
    await store.connect()
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

  /** Typing must not ask anything; that is what holding the library is for. */
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
 * Searching Zotero itself, for what the list cannot see: full text and notes.
 */
describe('searching Zotero', () => {
  async function loaded(found: string[], total = found.length) {
    answer({
      writing_zotero_cached: { items: [GINZBURG, DARNTON], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
      writing_zotero_search: { items: found, version: 1, total, has_more: false },
    })
    const store = new WritingZoteroStore()
    await store.connect()
    mockInvoke.mockClear()
    return store
  }

  it('asks Zotero with the query as typed, trimmed', async () => {
    const store = await loaded([])

    await store.searchLibrary(' Acha ')

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_search', {
      library: '0',
      query: 'Acha',
    })
  })

  /** An empty box is the whole list, which is already held. */
  it('does not ask Zotero for an empty box', async () => {
    const store = await loaded([])
    store.search('formaggio')

    await store.searchLibrary('   ')

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(store.snapshot.entries).toHaveLength(2)
  })

  /** A search is not a new library: clearing the box shows all of it again. */
  it('leaves the library it read in place', async () => {
    const store = await loaded([GINZBURG])

    await store.searchLibrary('formaggio')
    store.search('')

    expect(store.snapshot.entries).toHaveLength(2)
  })

  /** Zotero also searches full text; what it finds there joins the list. */
  it('adds what only Zotero found to what the list already matched', async () => {
    const ACHA = JSON.stringify({ id: 'IJKL9012', type: 'book', title: 'Historia social' })
    const store = await loaded([GINZBURG, ACHA])

    await store.searchLibrary('Ginzburg')

    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['ABCD1234', 'IJKL9012'])
  })

  /** An answer that arrives after the box moved on answers nothing. */
  it('drops an answer for a search that is no longer in the box', async () => {
    const store = await loaded([])
    let reply: (value: unknown) => void = () => {}
    mockInvoke.mockReturnValueOnce(
      new Promise((resolve) => {
        reply = resolve
      }) as never
    )

    const searching = store.searchLibrary('Ginzburg')
    store.search('Darnton')
    reply({ items: [GINZBURG], version: 1, total: 1, has_more: false })
    await searching

    expect(store.snapshot.query).toBe('Darnton')
    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['EFGH5678'])
  })

  it('keeps reporting what Zotero says it found', async () => {
    const store = await loaded([GINZBURG], 137)

    await store.searchLibrary('Acha')

    expect(store.snapshot.total).toBe(137)
  })
})
