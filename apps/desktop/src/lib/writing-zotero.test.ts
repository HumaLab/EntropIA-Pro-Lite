import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { processingSyncBibliographyLibrary } from './batch-processing'
import {
  bibliographyDerivedProgress,
  formatEtaMs,
  WritingZoteroStore,
  type BibliographySyncStatus,
} from './writing-zotero'

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

const MOORE = JSON.stringify({
  id: 'moore1973',
  type: 'book',
  title: 'Los orígenes',
  author: [{ family: 'Moore', given: 'Barrington' }],
  issued: { 'date-parts': [[1973]] },
})

const MOORE_ITEM = {
  key: '37C8RJP8',
  itemVersion: 9756,
  libraryType: 'user',
  libraryId: '0',
  cslJson: MOORE,
}

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

describe('E2b-4 bibliography synchronization IPC', () => {
  it('forwards the generated request and selected library and returns admission details', async () => {
    const response = {
      batchId: 'batch-bibliography',
      taskId: 'task-bibliography',
      created: true,
      requeued: false,
    }
    mockInvoke.mockResolvedValue(response as never)

    await expect(
      processingSyncBibliographyLibrary('request-42', 'group', '6680944')
    ).resolves.toEqual(response)
    expect(mockInvoke).toHaveBeenCalledWith('processing_sync_bibliography_library', {
      requestId: 'request-42',
      libraryType: 'group',
      libraryId: '6680944',
    })
  })
})

describe('E2b-4 bibliography synchronization request state', () => {
  const requested = {
    batchId: 'batch-bibliography',
    taskId: 'task-bibliography',
    created: true,
    requeued: false,
  }

  it('requests the selected library once and reports admission without claiming completion', async () => {
    let release: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd !== 'processing_sync_bibliography_library') {
        return Promise.reject(new Error(`unexpected ${cmd}`))
      }
      return new Promise((resolve) => {
        release = resolve
      })
    }) as never)
    const store = new WritingZoteroStore()
    store.select('group', '6680944')

    const first = store.requestBibliographySync()
    const joined = store.requestBibliographySync()

    expect(joined).toBe(first)
    expect(calls('processing_sync_bibliography_library')).toHaveLength(1)
    expect(calls('processing_sync_bibliography_library')[0]?.[1]).toEqual({
      requestId: expect.stringMatching(/\S/),
      libraryType: 'group',
      libraryId: '6680944',
    })
    expect(store.snapshot.bibliographySync).toEqual({
      loading: true,
      error: null,
      requested: null,
    })

    release(requested)
    await first

    expect(store.snapshot.bibliographySync).toEqual({
      loading: false,
      error: null,
      requested,
    })
  })

  const status = (overrides: Record<string, unknown> = {}) => ({
    state: 'pending',
    errorCode: null,
    errorMessage: null,
    progressDone: 0,
    progressTotal: null,
    itemsSeen: null,
    remoteTotal: null,
    newProfiles: 0,
    newExtractions: 0,
    ...overrides,
  })

  /** Answers the request, then each status poll with the next scripted status. */
  function scheduler(statuses: unknown[]) {
    const polled: unknown[] = [...statuses]
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'processing_sync_bibliography_library') return Promise.resolve(requested)
      if (cmd === 'processing_bibliography_sync_status') {
        return Promise.resolve(polled.length > 1 ? polled.shift() : polled[0])
      }
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
  }

  it('follows the scheduler task until it finishes and reports its real result', async () => {
    vi.useFakeTimers()
    try {
      scheduler([
        status({ state: 'running', progressDone: 10, progressTotal: 40 }),
        status({ state: 'succeeded', itemsSeen: 40, newProfiles: 3, newExtractions: 2 }),
      ])
      const store = new WritingZoteroStore()

      await store.requestBibliographySync()
      await vi.advanceTimersByTimeAsync(0)
      expect(calls('processing_bibliography_sync_status')[0]?.[1]).toEqual({
        taskId: 'task-bibliography',
      })
      expect(store.snapshot.bibliographyProgress?.status?.state).toBe('running')

      await vi.advanceTimersByTimeAsync(2000)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({
        state: 'succeeded',
        newProfiles: 3,
        newExtractions: 2,
      })

      // A finished task is never polled again.
      const before = calls('processing_bibliography_sync_status').length
      await vi.advanceTimersByTimeAsync(10000)
      expect(calls('processing_bibliography_sync_status')).toHaveLength(before)
    } finally {
      vi.useRealTimers()
    }
  })

  it('keeps following the derived backlog after the sync succeeds and stops when it drains', async () => {
    vi.useFakeTimers()
    try {
      scheduler([
        status({
          state: 'succeeded',
          newProfiles: 2,
          newExtractions: 0,
          profilesDone: 0,
          profilesTotal: 2,
          extractionsDone: 0,
          extractionsTotal: 0,
          etaMs: null,
        }),
        status({
          state: 'succeeded',
          newProfiles: 2,
          newExtractions: 0,
          profilesDone: 1,
          profilesTotal: 2,
          extractionsDone: 0,
          extractionsTotal: 0,
          etaMs: 900,
        }),
        status({
          state: 'succeeded',
          newProfiles: 2,
          newExtractions: 0,
          profilesDone: 2,
          profilesTotal: 2,
          extractionsDone: 0,
          extractionsTotal: 0,
          etaMs: 0,
        }),
      ])
      const store = new WritingZoteroStore()

      await store.requestBibliographySync()
      await vi.advanceTimersByTimeAsync(0)
      // The sync task settled but its derived work has not: the store keeps
      // reading so the progress on screen keeps moving.
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({
        state: 'succeeded',
        profilesDone: 0,
        profilesTotal: 2,
      })

      await vi.advanceTimersByTimeAsync(2000)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({
        profilesDone: 1,
        profilesTotal: 2,
      })

      await vi.advanceTimersByTimeAsync(2000)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({
        profilesDone: 2,
        profilesTotal: 2,
      })

      // All derived work is settled: the follower stops.
      const before = calls('processing_bibliography_sync_status').length
      await vi.advanceTimersByTimeAsync(10000)
      expect(calls('processing_bibliography_sync_status')).toHaveLength(before)
    } finally {
      vi.useRealTimers()
    }
  })

  it('keeps following a sync that waits for Zotero and surfaces why', async () => {
    vi.useFakeTimers()
    try {
      scheduler([
        status({ state: 'retry_wait', errorCode: 'zotero_unreachable', errorMessage: 'nothing' }),
        status({ state: 'running' }),
      ])
      const store = new WritingZoteroStore()

      await store.requestBibliographySync()
      await vi.advanceTimersByTimeAsync(0)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({
        state: 'retry_wait',
        errorCode: 'zotero_unreachable',
      })

      await vi.advanceTimersByTimeAsync(2000)
      expect(store.snapshot.bibliographyProgress?.status?.state).toBe('running')
    } finally {
      vi.useRealTimers()
    }
  })

  it('says the status could not be read instead of inventing one', async () => {
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'processing_sync_bibliography_library'
        ? Promise.resolve(requested)
        : Promise.reject(new Error('db busy'))) as never)
    const store = new WritingZoteroStore()

    await store.requestBibliographySync()
    await vi.waitFor(() => {
      expect(store.snapshot.bibliographyProgress).toEqual({
        status: null,
        unreadable: 'db busy',
      })
    })
  })

  it('stops following when the selected library changes', async () => {
    vi.useFakeTimers()
    try {
      scheduler([status({ state: 'running' })])
      const store = new WritingZoteroStore()
      await store.requestBibliographySync()
      await vi.advanceTimersByTimeAsync(0)

      store.select('group', '7')
      const before = calls('processing_bibliography_sync_status').length
      await vi.advanceTimersByTimeAsync(10000)

      expect(calls('processing_bibliography_sync_status')).toHaveLength(before)
      expect(store.snapshot.bibliographyProgress).toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('invalidates a late request result after the selection changes', async () => {
    const releases = new Map<string, (value: unknown) => void>()
    mockInvoke.mockImplementation(((cmd: string, args: Record<string, unknown>) => {
      if (cmd !== 'processing_sync_bibliography_library') {
        return Promise.reject(new Error(`unexpected ${cmd}`))
      }
      return new Promise((resolve) => {
        releases.set(String(args.libraryId), resolve)
      })
    }) as never)
    const store = new WritingZoteroStore()
    store.select('group', '1')

    const requestA = store.requestBibliographySync()
    store.select('group', '2')
    const requestB = store.requestBibliographySync()
    const requestedB = { ...requested, batchId: 'batch-b', taskId: 'task-b' }
    releases.get('2')?.(requestedB)
    await requestB

    releases.get('1')?.(requested)
    await requestA

    expect(store.snapshot.selection).toEqual({ libraryType: 'group', libraryId: '2' })
    expect(store.snapshot.bibliographySync).toEqual({
      loading: false,
      error: null,
      requested: requestedB,
    })
  })

  it('exposes request errors separately from direct Zotero mirror errors', async () => {
    mockInvoke.mockRejectedValue(new Error('catalog unavailable'))
    const store = new WritingZoteroStore()

    await store.requestBibliographySync()

    expect(store.snapshot.bibliographySync).toEqual({
      loading: false,
      error: 'catalog unavailable',
      requested: null,
    })
    expect(store.snapshot.error).toBeNull()
  })
})

describe('P3 backlog follower across a restart', () => {
  const status = (overrides: Record<string, unknown> = {}) => ({
    state: 'succeeded',
    errorCode: null,
    errorMessage: null,
    progressDone: 40,
    progressTotal: 40,
    itemsSeen: 40,
    remoteTotal: 40,
    newProfiles: 2,
    newExtractions: 1,
    profilesDone: 0,
    profilesTotal: 2,
    extractionsDone: 0,
    extractionsTotal: 1,
    etaMs: null,
    ...overrides,
  })

  /** Answers each latest-status poll with the next scripted status. */
  function backlog(statuses: unknown[]) {
    const polled = [...statuses]
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'processing_latest_bibliography_sync_status') {
        return Promise.resolve(polled.length > 1 ? polled.shift() : polled[0])
      }
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
  }

  it('follows the draining backlog of the latest sync at startup and stops when it drains', async () => {
    vi.useFakeTimers()
    try {
      backlog([
        status({ profilesDone: 0, profilesTotal: 2, extractionsDone: 0, extractionsTotal: 1 }),
        status({ profilesDone: 1, profilesTotal: 2, extractionsDone: 1, extractionsTotal: 1 }),
        status({ profilesDone: 2, profilesTotal: 2, extractionsDone: 1, extractionsTotal: 1 }),
      ])
      const store = new WritingZoteroStore()

      const following = store.followBibliographyBacklog()
      await vi.advanceTimersByTimeAsync(0)
      expect(calls('processing_latest_bibliography_sync_status')).toHaveLength(1)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({
        profilesDone: 0,
        profilesTotal: 2,
      })

      await vi.advanceTimersByTimeAsync(2000)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({ profilesDone: 1 })

      await vi.advanceTimersByTimeAsync(2000)
      expect(store.snapshot.bibliographyProgress?.status).toMatchObject({ profilesDone: 2 })

      // The backlog drained: the follower stops and the line goes quiet.
      const before = calls('processing_latest_bibliography_sync_status').length
      await vi.advanceTimersByTimeAsync(10_000)
      expect(calls('processing_latest_bibliography_sync_status')).toHaveLength(before)
      await following
    } finally {
      vi.useRealTimers()
    }
  })

  it('claims nothing when no bibliography sync was ever requested', async () => {
    backlog([null])
    const store = new WritingZoteroStore()

    await store.followBibliographyBacklog()

    expect(calls('processing_latest_bibliography_sync_status')).toHaveLength(1)
    expect(store.snapshot.bibliographyProgress).toBeNull()
  })

  it('joins a follow already running instead of polling twice', async () => {
    vi.useFakeTimers()
    try {
      backlog([status({ state: 'running', profilesDone: 0, profilesTotal: 5 })])
      const store = new WritingZoteroStore()

      const first = store.followBibliographyBacklog()
      const joined = store.followBibliographyBacklog()
      await vi.advanceTimersByTimeAsync(0)

      expect(joined).toBe(first)
      expect(calls('processing_latest_bibliography_sync_status')).toHaveLength(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('leaves the screen to a sync requested in this session', async () => {
    vi.useFakeTimers()
    try {
      mockInvoke.mockImplementation(((cmd: string) => {
        if (cmd === 'processing_latest_bibliography_sync_status') {
          return Promise.resolve(status({ state: 'running' }))
        }
        if (cmd === 'processing_sync_bibliography_library') {
          return Promise.resolve({
            batchId: 'batch-bibliography',
            taskId: 'task-bibliography',
            created: true,
            requeued: false,
          })
        }
        if (cmd === 'processing_bibliography_sync_status') {
          return Promise.resolve(status({ state: 'running' }))
        }
        return Promise.reject(new Error(`unexpected ${cmd}`))
      }) as never)
      const store = new WritingZoteroStore()

      void store.followBibliographyBacklog()
      await vi.advanceTimersByTimeAsync(0)
      const latestBefore = calls('processing_latest_bibliography_sync_status').length

      await store.requestBibliographySync()
      await vi.advanceTimersByTimeAsync(10_000)

      expect(calls('processing_bibliography_sync_status').length).toBeGreaterThan(0)
      expect(calls('processing_latest_bibliography_sync_status')).toHaveLength(latestBefore)
    } finally {
      vi.useRealTimers()
    }
  })
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
    expect(calls('writing_zotero_sync')[0]?.[1]).toEqual({
      libraryType: 'user',
      libraryId: '0',
    })
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

  it('keeps native Zotero identity when the CSL id names the work differently', async () => {
    answer({
      writing_zotero_cached: { items: [MOORE_ITEM], version: 9756 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()

    await store.connect()

    expect(store.snapshot.entries[0]).toMatchObject({
      key: '37C8RJP8',
      itemVersion: 9756,
      libraryType: 'user',
      libraryId: '0',
      csl_json: MOORE,
    })
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

  describe('accents and typos', () => {
    const work = (id: string, title: string) =>
      JSON.stringify({ id, title, author: [{ family: 'Núñez' }] })
    const spatial = [work('a', 'La producción del espacio'), work('b', 'Another work')]
    async function spatialStore(fuzzy: boolean) {
      answer({
        writing_zotero_cached: { items: spatial, version: 1 },
        writing_zotero_probe: { state: 'endpoint_unavailable' },
      })
      const prefs = {
        fuzzyEnabled: async () => fuzzy,
        setFuzzyEnabled: async () => {},
      } as never
      const store = new WritingZoteroStore(prefs)
      await store.connect()
      await store.loadPreferences()
      return store
    }

    it('folds accents with approximate matching off', async () => {
      const store = await spatialStore(false)
      store.search('La produccion del espacio')
      expect(store.snapshot.entries.map((e) => e.title)).toEqual(['La producción del espacio'])
      store.search('nunez')
      expect(store.snapshot.entries).toHaveLength(2)
    })

    it('does not tolerate typos with the box off', async () => {
      const store = await spatialStore(false)
      store.search('La produción del espasio')
      expect(store.snapshot.entries).toHaveLength(0)
    })

    it('tolerates typos with the box on', async () => {
      const store = await spatialStore(true)
      store.search('La produción del espasio')
      expect(store.snapshot.entries.map((e) => e.title)).toEqual(['La producción del espacio'])
    })
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
      libraryType: 'user',
      libraryId: '0',
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

  it('keeps native Zotero identity on search results with a different CSL id', async () => {
    const store = await loaded([])
    // The search result is not in the cached list, so its identity must survive
    // the connector -> store -> list boundary rather than being inferred from CSL.
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'writing_zotero_search'
        ? Promise.resolve({ items: [MOORE_ITEM], version: 9756, total: 1, has_more: false })
        : Promise.reject(new Error(`unexpected ${cmd}`))) as never)

    await store.searchLibrary('Moore')

    expect(store.snapshot.entries[0]).toMatchObject({
      key: '37C8RJP8',
      itemVersion: 9756,
      libraryType: 'user',
      libraryId: '0',
      csl_json: MOORE,
    })
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

/**
 * B2: the same box also finds works by meaning, through the bibliography's
 * own hybrid search. Its scores never meet Zotero's: semantic-only works are
 * appended after every text match, in the order the bibliography ranked them.
 */
describe('searching by meaning', () => {
  const item = (key: string, csl: string) => ({
    key,
    itemVersion: 1,
    libraryType: 'user',
    libraryId: '0',
    cslJson: csl,
  })
  const hit = (itemKey: string, method: 'lexical' | 'vector' | 'hybrid') => ({
    itemId: `row-${itemKey}`,
    itemKey,
    libraryId: 'row-library',
    title: itemKey,
    method,
    lexicalScore: method === 'vector' ? null : -1.5,
    vectorScore: method === 'lexical' ? null : 0.9,
    fusedScore: 0.03,
    contractHash: null,
    generationId: null,
  })
  const semantic = (hits: unknown[], extra: Record<string, unknown> = {}) => ({
    hits,
    vectorAvailable: true,
    activeGenerationId: 'gen-1',
    contractHash: 'contract',
    librarySynced: true,
    ...extra,
  })

  async function loaded(bibliography: unknown, zotero: unknown = { items: [], total: 0 }) {
    answer({
      writing_zotero_cached: {
        items: [item('GIN1', GINZBURG), item('DAR1', DARNTON), item('MOO1', MOORE)],
        version: 1,
      },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
      writing_zotero_search: zotero,
      bibliography_search_works: bibliography,
    })
    const store = new WritingZoteroStore()
    await store.connect()
    mockInvoke.mockClear()
    return store
  }

  it('scopes the bibliography search to the selected Zotero library', async () => {
    const store = await loaded(semantic([]))
    store.select('group', '6680944')

    await store.searchLibrary('revoluciones')

    expect(mockInvoke).toHaveBeenCalledWith('bibliography_search_works', {
      request: expect.objectContaining({
        text: 'revoluciones',
        zoteroLibraryType: 'group',
        zoteroLibraryId: '6680944',
      }),
    })
  })

  it('appends works only the meaning found after every text match, in ranked order', async () => {
    const store = await loaded(
      semantic([hit('MOO1', 'vector'), hit('GIN1', 'hybrid'), hit('DAR1', 'vector')])
    )

    await store.searchLibrary('formaggio')

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1', 'MOO1', 'DAR1'])
    expect(store.snapshot.entries.map((entry) => entry.semantic)).toEqual([undefined, true, true])
    expect(store.snapshot.semanticStatus).toBe('ok')
  })

  it('never lists a work twice when both searches find it', async () => {
    const store = await loaded(semantic([hit('GIN1', 'hybrid')]), {
      items: [item('GIN1', GINZBURG)],
      total: 1,
    })

    await store.searchLibrary('formaggio')

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1'])
  })

  it('lists a hit the held list lacks from the catalog, citable through its own CSL', async () => {
    const store = await loaded(
      semantic([
        {
          ...hit('GONE', 'vector'),
          title: 'Plan Federal de Viviendas',
          authors: 'Pérez',
          year: 2010,
          libraryType: 'user',
          libraryNativeId: '0',
          cslJson: '{"id":"GONE","title":"Plan Federal de Viviendas"}',
        },
        hit('DAR1', 'vector'),
      ])
    )

    await store.searchLibrary('formaggio')

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1', 'GONE', 'DAR1'])
    const fromCatalog = store.snapshot.entries[1]
    expect(fromCatalog).toMatchObject({
      title: 'Plan Federal de Viviendas',
      authors: 'Pérez',
      year: '2010',
      libraryType: 'user',
      libraryId: '0',
      semantic: true,
      csl_json: '{"id":"GONE","title":"Plan Federal de Viviendas"}',
    })
  })

  it('shows meaning hits from the mirror-backed list while Zotero is closed and says nothing false', async () => {
    const store = await loaded(semantic([hit('DAR1', 'vector')]))
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'writing_zotero_search'
        ? Promise.reject(new Error('Nada responde en el puerto local'))
        : Promise.resolve(semantic([hit('DAR1', 'vector')]))) as never)

    await store.searchLibrary('programa dignidad')

    expect(store.snapshot.entries.map((entry) => [entry.key, entry.semantic])).toEqual([
      ['DAR1', true],
    ])
    expect(store.snapshot.semanticStatus).toBe('ok')
  })

  it('does not hold the meaning hits back while Zotero is still answering', async () => {
    const store = await loaded(semantic([hit('DAR1', 'vector')]))
    let zoteroReply: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'writing_zotero_search'
        ? new Promise((resolve) => {
            zoteroReply = resolve
          })
        : Promise.resolve(semantic([hit('DAR1', 'vector')]))) as never)

    const searching = store.searchLibrary('formaggio')
    await vi.waitFor(() => {
      expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1', 'DAR1'])
    })
    expect(store.snapshot.semanticStatus).toBe('ok')

    zoteroReply({ items: [], total: 0 })
    await searching
    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1', 'DAR1'])
  })

  it('says the library is not synced into EntropIA and keeps the text results', async () => {
    const store = await loaded(semantic([], { librarySynced: false, vectorAvailable: false }))

    await store.searchLibrary('formaggio')

    expect(store.snapshot.semanticStatus).toBe('not_synced')
    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1'])
  })

  it('says only the text match ran when there is no active embedding generation', async () => {
    const store = await loaded(
      semantic([hit('DAR1', 'lexical')], { vectorAvailable: false, activeGenerationId: null })
    )

    await store.searchLibrary('formaggio')

    expect(store.snapshot.semanticStatus).toBe('lexical_only')
    // A text match over the profile still adds the work, but is not labeled semantic.
    expect(store.snapshot.entries.map((entry) => [entry.key, entry.semantic])).toEqual([
      ['GIN1', undefined],
      ['DAR1', undefined],
    ])
  })

  it('keeps the text results and reports a failed bibliography search', async () => {
    answer({
      writing_zotero_cached: { items: [item('GIN1', GINZBURG)], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
      writing_zotero_search: { items: [], total: 0 },
    })
    const store = new WritingZoteroStore()
    await store.connect()

    await store.searchLibrary('formaggio')

    expect(store.snapshot.semanticStatus).toBe('failed')
    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1'])
    expect(store.snapshot.error).toBeNull()
  })

  it('still finds by meaning when Zotero itself cannot answer', async () => {
    answer({
      writing_zotero_cached: {
        items: [item('GIN1', GINZBURG), item('DAR1', DARNTON)],
        version: 1,
      },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
      writing_zotero_search: () => Promise.reject(new Error('timeout')),
      bibliography_search_works: semantic([hit('DAR1', 'vector')]),
    })
    const store = new WritingZoteroStore()
    await store.connect()

    await store.searchLibrary('formaggio')

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1', 'DAR1'])
    expect(store.snapshot.error).toBe('timeout')
  })

  it('drops a meaning answer for a query that is no longer in the box', async () => {
    const store = await loaded(semantic([hit('DAR1', 'vector')]))
    let reply: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'bibliography_search_works'
        ? new Promise((resolve) => {
            reply = resolve
          })
        : Promise.resolve({ items: [], total: 0 })) as never)

    const searching = store.searchLibrary('formaggio')
    store.search('Moore')
    reply(semantic([hit('DAR1', 'vector')]))
    await searching

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['MOO1'])
    expect(store.snapshot.semanticStatus).toBe('idle')
  })

  it('resets the notice when the library changes or the box is emptied', async () => {
    const store = await loaded(semantic([], { librarySynced: false }))
    await store.searchLibrary('formaggio')
    expect(store.snapshot.semanticStatus).toBe('not_synced')

    await store.searchLibrary('')
    expect(store.snapshot.semanticStatus).toBe('idle')

    await store.searchLibrary('formaggio')
    store.select('group', '1')
    expect(store.snapshot.semanticStatus).toBe('idle')
  })
})

/**
 * E1c-1 (TS half): explicit selection with library-keyed stale-response
 * isolation. RED first: none of this exists yet on the store.
 */
describe('searching by content', () => {
  const item = (key: string, csl: string) => ({
    key,
    itemVersion: 1,
    libraryType: 'user',
    libraryId: '0',
    cslJson: csl,
  })
  const noWorks = {
    hits: [],
    vectorAvailable: true,
    activeGenerationId: 'gen-1',
    contractHash: 'contract',
    librarySynced: true,
  }
  const passage = (itemKey: string, matchKind: string, matchTerms: string[], extra = {}) => ({
    chunkId: `${itemKey}:${matchKind}:${matchTerms.join('-')}`,
    itemId: `row-${itemKey}`,
    itemKey,
    title: `Obra ${itemKey}`,
    authors: 'Núñez',
    year: 2016,
    libraryName: 'Mi biblioteca',
    libraryType: 'user',
    libraryNativeId: '0',
    cslJson: JSON.stringify({ id: itemKey, title: `Obra ${itemKey}` }),
    snippet: '…',
    location: { kind: 'pages', from: 2, to: 2 },
    score: 0.1,
    matchKind,
    matchTerms,
    ...extra,
  })
  const passages = (list: unknown[]) => ({ passages: list, notice: null })

  async function loaded(answers: { works?: unknown; passages?: unknown }, prefs?: never) {
    answer({
      writing_zotero_cached: {
        items: [item('GIN1', GINZBURG), item('DAR1', DARNTON), item('MOO1', MOORE)],
        version: 1,
      },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
      writing_zotero_search: { items: [], total: 0 },
      bibliography_search_works: answers.works ?? noWorks,
      bibliography_search_passages: answers.passages ?? passages([]),
    })
    const store = new WritingZoteroStore(prefs)
    await store.connect()
    mockInvoke.mockClear()
    return store
  }

  it('lists a work found only by what its passage says, tagged and after the other matches', async () => {
    const store = await loaded({
      works: {
        ...noWorks,
        hits: [
          {
            itemId: 'row-DAR1',
            itemKey: 'DAR1',
            libraryId: 'row-library',
            title: 'DAR1',
            method: 'vector',
            lexicalScore: null,
            vectorScore: 0.9,
            fusedScore: 0.03,
            contractHash: null,
            generationId: null,
          },
        ],
      },
      passages: passages([passage('MOO1', 'exact', ['plan', 'federal'])]),
    })

    await store.searchLibrary('formaggio')

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1', 'DAR1', 'MOO1'])
    expect(store.snapshot.entries[2]?.content).toEqual({
      kind: 'exact',
      terms: ['plan', 'federal'],
    })
    expect(store.snapshot.entries[2]?.semantic).toBeUndefined()
  })

  it('asks for the passages of the selected library with the approximate switch', async () => {
    const store = await loaded({})
    store.select('group', '6680944')

    await store.searchLibrary('plan federal')

    expect(mockInvoke).toHaveBeenCalledWith('bibliography_search_passages', {
      request: expect.objectContaining({
        text: 'plan federal',
        fuzzy: true,
        zoteroLibraryType: 'group',
        zoteroLibraryId: '6680944',
      }),
    })
  })

  it('lists a work once however many passages and legs found it', async () => {
    const store = await loaded({
      passages: passages([
        passage('MOO1', 'approximate', ['crocitto']),
        passage('MOO1', 'exact', ['croitto']),
        passage('MOO1', 'exact', ['croitto', 'otro']),
        passage('GIN1', 'exact', ['formaggio']),
      ]),
    })

    await store.searchLibrary('formaggio')

    // GIN1 is a text match already: not listed twice, not re-tagged.
    expect(store.snapshot.entries.map((entry) => [entry.key, entry.content])).toEqual([
      ['GIN1', undefined],
      ['MOO1', { kind: 'exact', terms: ['croitto', 'otro'] }],
    ])
  })

  it('leaves out passages found only by meaning: the meaning leg owns those', async () => {
    const store = await loaded({ passages: passages([passage('MOO1', 'meaning', [])]) })

    await store.searchLibrary('revoluciones')

    expect(store.snapshot.entries.map((entry) => entry.key)).not.toContain('MOO1')
  })

  it('lists a work the held list lacks from the passage itself', async () => {
    const store = await loaded({ passages: passages([passage('NEW1', 'exact', ['dignidad'])]) })

    await store.searchLibrary('dignidad')

    expect(store.snapshot.entries.map((entry) => [entry.key, entry.title])).toEqual([
      ['NEW1', 'Obra NEW1'],
    ])
  })

  it('keeps the other matches when the passage search fails', async () => {
    const store = await loaded({ passages: passages([]) })
    mockInvoke.mockImplementation(((cmd: string) =>
      cmd === 'bibliography_search_passages'
        ? Promise.reject(new Error('boom'))
        : Promise.resolve(
            cmd === 'bibliography_search_works' ? noWorks : { items: [], total: 0 }
          )) as never)

    await store.searchLibrary('formaggio')

    expect(store.snapshot.entries.map((entry) => entry.key)).toEqual(['GIN1'])
  })

  it('turns the approximate switch off, remembers it and searches again', async () => {
    const saved: Record<string, string> = {}
    const prefs = {
      fuzzyEnabled: async () => saved.fuzzy !== 'off',
      setFuzzyEnabled: async (enabled: boolean) => {
        saved.fuzzy = enabled ? 'on' : 'off'
      },
    } as never
    const store = await loaded({ passages: passages([]) }, prefs)
    await store.searchLibrary('croitto')
    expect(store.snapshot.fuzzy).toBe(true)
    mockInvoke.mockClear()

    await store.setFuzzy(false)

    expect(saved.fuzzy).toBe('off')
    expect(store.snapshot.fuzzy).toBe(false)
    expect(mockInvoke).toHaveBeenCalledWith('bibliography_search_passages', {
      request: expect.objectContaining({ text: 'croitto', fuzzy: false }),
    })
  })
})

describe('E1c-1 library selection', () => {
  it('defaults to the personal library user/0', async () => {
    const store = new WritingZoteroStore()

    expect(store.selection).toEqual({ libraryType: 'user', libraryId: '0' })
  })

  it('sends the current selection to cached and sync', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG], version: 1 },
      writing_zotero_probe: AVAILABLE,
      writing_zotero_sync: unchanged,
    })
    const store = new WritingZoteroStore()

    await store.connect()

    expect(calls('writing_zotero_cached')[0]?.[1]).toEqual({
      libraryType: 'user',
      libraryId: '0',
    })
    expect(calls('writing_zotero_sync')[0]?.[1]).toEqual({
      libraryType: 'user',
      libraryId: '0',
    })
  })

  it('sends the current selection and query to search', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG, DARNTON], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
      writing_zotero_search: { items: [], version: 1, total: 0, has_more: false },
    })
    const store = new WritingZoteroStore()
    await store.connect()
    mockInvoke.mockClear()

    store.select('group', '6680944')
    await store.searchLibrary('Acha')

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_search', {
      libraryType: 'group',
      libraryId: '6680944',
      query: 'Acha',
    })
  })

  it('changing selection clears library A so B never shows its data', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()
    await store.connect()
    expect(store.snapshot.loaded).toBe(1)

    store.select('group', '6680944')

    expect(store.selection).toEqual({ libraryType: 'group', libraryId: '6680944' })
    expect(store.snapshot.entries).toEqual([])
    expect(store.snapshot.loaded).toBe(0)
    expect(store.snapshot.total).toBeNull()
    expect(store.snapshot.query).toBe('')
  })

  it('discards a late cached view from the previous selection', async () => {
    let releaseCached: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_cached') {
        return new Promise((resolve) => {
          releaseCached = resolve
        })
      }
      if (cmd === 'writing_zotero_probe') return Promise.resolve(AVAILABLE)
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    const store = new WritingZoteroStore()

    const openingA = store.connect()
    store.select('group', '6680944')
    releaseCached({ items: [GINZBURG], version: 1 })
    await openingA

    expect(store.snapshot.loaded).toBe(0)
    expect(store.snapshot.entries).toEqual([])
  })

  it('discards a late sync outcome from the previous selection', async () => {
    let releaseSync: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_cached') return Promise.resolve(NO_COPY)
      if (cmd === 'writing_zotero_probe') return Promise.resolve(AVAILABLE)
      if (cmd === 'writing_zotero_sync') {
        return new Promise((resolve) => {
          releaseSync = resolve
        })
      }
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    const store = new WritingZoteroStore()

    const syncingA = store.sync()
    store.select('group', '6680944')
    releaseSync(synced([GINZBURG]))
    await syncingA

    expect(store.snapshot.loaded).toBe(0)
    expect(store.snapshot.entries).toEqual([])
  })

  it('does not let the loaded===0 fast path leak across selections', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG, DARNTON], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()
    await store.connect()
    expect(store.snapshot.loaded).toBe(2)

    // A slow cached view for B starts while B is empty, then A is reselected
    // before it answers: the late B view must not wipe the loaded A library.
    let releaseB: (value: unknown) => void = () => {}
    mockInvoke.mockImplementation(((cmd: string) => {
      if (cmd === 'writing_zotero_cached') {
        return new Promise((resolve) => {
          releaseB = resolve
        })
      }
      return Promise.reject(new Error(`unexpected ${cmd}`))
    }) as never)
    store.select('group', '6680944')
    const openingB = store.connect()
    store.select('user', '0')
    // Reselecting A clears it (selection change always clears); what matters
    // is the stale B answer changes nothing afterwards.
    releaseB({ items: [GINZBURG], version: 1 })
    await openingB

    expect(store.snapshot.entries).toEqual([])
    expect(store.snapshot.loaded).toBe(0)
  })

  it('drops a late search answer from another library with the same query', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG, DARNTON], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()
    await store.connect()
    let reply: (value: unknown) => void = () => {}
    mockInvoke.mockReturnValueOnce(
      new Promise((resolve) => {
        reply = resolve
      }) as never
    )

    const searchingA = store.searchLibrary('Ginzburg')
    store.select('group', '6680944')
    reply({ items: [GINZBURG], version: 1, total: 1, has_more: false })
    await searchingA

    expect(store.snapshot.query).toBe('')
    expect(store.snapshot.entries).toEqual([])
    expect(store.snapshot.total).toBeNull()
  })

  it('reselecting the current library leaves the held list intact', async () => {
    answer({
      writing_zotero_cached: { items: [GINZBURG], version: 1 },
      writing_zotero_probe: { state: 'endpoint_unavailable' },
    })
    const store = new WritingZoteroStore()
    await store.connect()

    store.select('user', '0')

    expect(store.snapshot.loaded).toBe(1)
    expect(store.snapshot.entries.map((e) => e.key)).toEqual(['ABCD1234'])
  })

  it('exposes the selection on the snapshot and never by reference', async () => {
    const store = new WritingZoteroStore()
    expect(store.snapshot.selection).toEqual({ libraryType: 'user', libraryId: '0' })

    store.select('group', '6680944')
    expect(store.snapshot.selection).toEqual({ libraryType: 'group', libraryId: '6680944' })

    store.selection.libraryId = 'hacked'
    expect(store.selection.libraryId).toBe('6680944')
  })
})

describe('P3 derived work of a library sync', () => {
  const status: BibliographySyncStatus = {
    state: 'succeeded',
    errorCode: null,
    errorMessage: null,
    progressDone: 40,
    progressTotal: 40,
    itemsSeen: 40,
    remoteTotal: 40,
    newProfiles: 450,
    newExtractions: 400,
    profilesDone: 120,
    profilesTotal: 450,
    extractionsDone: 30,
    extractionsTotal: 400,
    etaMs: 720_000,
  }

  it('reads fichas and pasajes out of the live status window', () => {
    expect(bibliographyDerivedProgress(status)).toEqual({
      worksDone: 120,
      worksTotal: 450,
      passagesDone: 30,
      passagesTotal: 400,
      etaMs: 720_000,
      remaining: 700,
    })
  })

  it('formats the remaining time humanely', () => {
    expect(formatEtaMs(30_000)).toBe('<1 min')
    expect(formatEtaMs(59_000)).toBe('<1 min')
    expect(formatEtaMs(720_000)).toBe('12 min')
    expect(formatEtaMs(7_500_000)).toBe('2 h 5 min')
  })
})
