import { get } from 'svelte/store'
import { describe, expect, it, vi } from 'vitest'
import type { CaptureDraft, DownloadDraft } from './navegador-capture'
import { EMPTY_BROWSER, type BrowserState } from './navegador-tabs'
import { createNavegadorStore } from './navegador-store'

const capture: CaptureDraft = {
  id: 'draft-1',
  kind: 'page',
  finalUrl: 'https://example.com/',
  title: 'Example',
  canonicalUrl: null,
  siteName: null,
  lang: 'en',
  text: 'text',
  quote: null,
  quotePrefix: null,
  quoteSuffix: null,
  htmlBytes: 10,
  hashOf: 'html',
  sha256: 'a'.repeat(64),
  truncated: false,
  accessedAt: '2026-09-30T12:00:00Z',
}

const download = (id: string, patch: Partial<DownloadDraft> = {}): DownloadDraft => ({
  id,
  url: `https://example.com/${id}.pdf`,
  fileName: `${id}.pdf`,
  size: null,
  sha256: null,
  savedTo: null,
  accessedAt: '2026-09-30T12:00:00Z',
  status: 'downloading',
  reason: null,
  tab: null,
  pageUrl: null,
  alreadySavedIn: null,
  pageTitle: null,
  ...patch,
})

function fakeListen() {
  let handler: ((draft: DownloadDraft) => void) | undefined
  const unlisten = vi.fn()
  const listen = vi.fn(async (next: (draft: DownloadDraft) => void) => {
    handler = next
    return unlisten
  })
  return { listen, unlisten, emit: (draft: DownloadDraft) => handler?.(draft) }
}

describe('createNavegadorStore', () => {
  it('starts empty', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    expect(get(store)).toEqual({
      capture: null,
      captureError: null,
      downloads: [],
      browser: EMPTY_BROWSER,
      saving: [],
      saved: {},
      saveErrors: {},
    })
  })

  it('keeps a capture draft and its error until they are dismissed', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.setCapture(capture)
    expect(get(store).capture).toEqual(capture)
    store.setCaptureError('boom')
    expect(get(store).capture).toBeNull()
    expect(get(store).captureError).toBe('boom')
    store.setCapture(capture)
    expect(get(store).captureError).toBeNull()
    store.clearCapture()
    expect(get(store)).toMatchObject({ capture: null, captureError: null })
  })

  it('lists downloads newest first, updates them in place and keeps the last few', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    for (const id of ['a', 'b', 'c', 'd', 'e', 'f']) store.applyDownload(download(id))
    expect(get(store).downloads.map((d) => d.id)).toEqual(['f', 'e', 'd', 'c', 'b'])
    store.applyDownload(download('d', { status: 'ready' }))
    expect(get(store).downloads.map((d) => [d.id, d.status])).toEqual([
      ['f', 'downloading'],
      ['e', 'downloading'],
      ['d', 'ready'],
      ['c', 'downloading'],
      ['b', 'downloading'],
    ])
  })

  it('survives the view: a second reader sees what the first one left', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.setCapture(capture)
    store.applyDownload(download('a'))
    // A view unmounts and another mounts: it only has to subscribe again.
    expect(get(store).capture).toEqual(capture)
    expect(get(store).downloads).toHaveLength(1)
  })

  it('hears downloads that finish while no view is mounted', async () => {
    const fake = fakeListen()
    const store = createNavegadorStore({ listen: fake.listen })
    await store.startListening()
    fake.emit(download('a'))
    fake.emit(download('a', { status: 'ready', size: 1 }))
    expect(get(store).downloads).toEqual([download('a', { status: 'ready', size: 1 })])
  })

  it('listens once however many views ask', async () => {
    const fake = fakeListen()
    const store = createNavegadorStore({ listen: fake.listen })
    await Promise.all([store.startListening(), store.startListening()])
    await store.startListening()
    expect(fake.listen).toHaveBeenCalledTimes(1)
  })

  it('can try again after a listen that failed', async () => {
    const fake = fakeListen()
    fake.listen.mockRejectedValueOnce(new Error('no event bridge'))
    const store = createNavegadorStore({ listen: fake.listen })
    await expect(store.startListening()).rejects.toThrow('no event bridge')
    await store.startListening()
    expect(fake.listen).toHaveBeenCalledTimes(2)
  })

  it('clears the state without stopping to listen', async () => {
    const fake = fakeListen()
    const store = createNavegadorStore({ listen: fake.listen })
    await store.startListening()
    store.setCapture(capture)
    store.applyDownload(download('a'))
    store.clearAll()
    expect(get(store)).toEqual({
      capture: null,
      captureError: null,
      downloads: [],
      browser: EMPTY_BROWSER,
      saving: [],
      saved: {},
      saveErrors: {},
    })
    fake.emit(download('b'))
    expect(get(store).downloads.map((d) => d.id)).toEqual(['b'])
    expect(fake.unlisten).not.toHaveBeenCalled()
  })

  it('stops listening and forgets everything on reset', async () => {
    const fake = fakeListen()
    const store = createNavegadorStore({ listen: fake.listen })
    await store.startListening()
    store.applyDownload(download('a'))
    store.reset()
    expect(fake.unlisten).toHaveBeenCalledTimes(1)
    expect(get(store).downloads).toEqual([])
    await store.startListening()
    expect(fake.listen).toHaveBeenCalledTimes(2)
  })

  it('drops one download from the list and leaves the others', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    for (const id of ['a', 'b', 'c']) store.applyDownload(download(id, { status: 'ready' }))
    store.dismissDownload('b')
    expect(get(store).downloads.map((d) => d.id)).toEqual(['c', 'a'])
    store.dismissDownload('missing')
    expect(get(store).downloads).toHaveLength(2)
  })

  it('clears every download and leaves the capture draft alone', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.setCapture(capture)
    for (const id of ['a', 'b']) store.applyDownload(download(id, { status: 'ready' }))
    store.clearDownloads()
    expect(get(store).downloads).toEqual([])
    expect(get(store).capture).toEqual(capture)
  })

  it('dismissing a capture draft leaves the downloads alone', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.setCapture(capture)
    store.applyDownload(download('a', { status: 'ready' }))
    store.clearCapture()
    expect(get(store).downloads).toHaveLength(1)
  })

  it('does not bring back a dismissed download that reports again', async () => {
    const fake = fakeListen()
    const store = createNavegadorStore({ listen: fake.listen })
    await store.startListening()
    fake.emit(download('a'))
    store.dismissDownload('a')
    fake.emit(download('a', { status: 'ready', size: 1 }))
    expect(get(store).downloads).toEqual([])
    // A different download is unaffected.
    fake.emit(download('b'))
    expect(get(store).downloads.map((d) => d.id)).toEqual(['b'])
  })

  it('does not bring back a cleared download that was still running', async () => {
    const fake = fakeListen()
    const store = createNavegadorStore({ listen: fake.listen })
    await store.startListening()
    fake.emit(download('a'))
    fake.emit(download('b', { status: 'ready' }))
    store.clearDownloads()
    fake.emit(download('a', { status: 'ready' }))
    expect(get(store).downloads).toEqual([])
  })

  it('forgets what it dismissed when everything is reset', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.applyDownload(download('a'))
    store.dismissDownload('a')
    store.clearAll()
    store.applyDownload(download('a'))
    expect(get(store).downloads).toHaveLength(1)
  })
})

const browser = (revision: number, ids: number[], active: number | null): BrowserState => ({
  tabs: ids.map((id) => ({ id, url: `https://a.test/${id}`, title: null, blocked: null })),
  active,
  revision,
})

function fakeStateListen() {
  let handler: ((state: BrowserState) => void) | undefined
  const unlisten = vi.fn()
  const listenState = vi.fn(async (next: (state: BrowserState) => void) => {
    handler = next
    return unlisten
  })
  return { listenState, unlisten, emit: (state: BrowserState) => handler?.(state) }
}

describe('createNavegadorStore browser tabs', () => {
  it('keeps the tab list, so it survives the view', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.applyBrowser(browser(1, [1, 2], 2))
    expect(get(store).browser).toEqual(browser(1, [1, 2], 2))
  })

  it('takes a newer state and refuses an older one, whichever arrives first', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.applyBrowser(browser(5, [1, 2, 3], 3))
    store.applyBrowser(browser(4, [1], 1))
    expect(get(store).browser.tabs).toHaveLength(3)
    store.applyBrowser(browser(5, [1], 1))
    expect(get(store).browser.tabs).toHaveLength(3)
    store.applyBrowser(browser(6, [1, 2], 2))
    expect(get(store).browser.tabs).toHaveLength(2)
  })

  it('hears the backend while no view is mounted', async () => {
    const fake = fakeStateListen()
    const store = createNavegadorStore({
      listen: fakeListen().listen,
      listenState: fake.listenState,
    })
    await store.startListening()
    fake.emit(browser(1, [1], 1))
    fake.emit(browser(2, [1, 2], 2))
    expect(get(store).browser.active).toBe(2)
  })

  it('listens to the tab state once however many views ask, and stops with everything else', async () => {
    const fake = fakeStateListen()
    const downloads = fakeListen()
    const store = createNavegadorStore({ listen: downloads.listen, listenState: fake.listenState })
    await Promise.all([store.startListening(), store.startListening()])
    await store.startListening()
    expect(fake.listenState).toHaveBeenCalledTimes(1)
    store.reset()
    expect(fake.unlisten).toHaveBeenCalledTimes(1)
    expect(downloads.unlisten).toHaveBeenCalledTimes(1)
  })

  it('forgets the tabs when the browser goes away, and accepts a fresh one after', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.applyBrowser(browser(9, [1, 2], 1))
    store.clearAll()
    expect(get(store).browser).toEqual(EMPTY_BROWSER)
    store.applyBrowser(browser(1, [7], 7))
    expect(get(store).browser.active).toBe(7)
  })

  it('leaves the capture draft and the downloads alone when the tabs change', () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    store.setCapture(capture)
    store.applyDownload(download('a'))
    store.applyBrowser(browser(1, [1], 1))
    expect(get(store).capture).toEqual(capture)
    expect(get(store).downloads).toHaveLength(1)
  })
})

describe('saving from the store', () => {
  const saved = { sourceId: 's1', captureId: 'c1' }

  function savingStore(overrides: Record<string, unknown> = {}) {
    const deps = {
      listen: fakeListen().listen,
      saveCapture: vi.fn(async (_id: string) => saved),
      saveDownload: vi.fn(async (_id: string) => saved),
      discardDraft: vi.fn(async (_id: string) => undefined),
      ...overrides,
    }
    return { deps, store: createNavegadorStore(deps) }
  }

  it('saves a draft by id, shows it as saving meanwhile and as saved after', async () => {
    let finish: (value: typeof saved) => void = () => undefined
    const { deps, store } = savingStore({
      saveCapture: vi.fn(() => new Promise<typeof saved>((resolve) => (finish = resolve))),
    })
    store.setCapture(capture)

    const pending = store.saveCapture('draft-1')
    expect(get(store).saving).toEqual(['draft-1'])
    finish(saved)
    await pending

    expect(deps.saveCapture).toHaveBeenCalledWith('draft-1')
    expect(get(store).saving).toEqual([])
    expect(get(store).saved).toEqual({ 'draft-1': saved })
  })

  it('does not save the same item twice, while saving or after', async () => {
    const { deps, store } = savingStore()
    store.setCapture(capture)

    await Promise.all([store.saveCapture('draft-1'), store.saveCapture('draft-1')])
    await store.saveCapture('draft-1')

    expect(deps.saveCapture).toHaveBeenCalledTimes(1)
  })

  it('keeps the code of a failed save and lets the person try again', async () => {
    const saveCapture = vi
      .fn()
      .mockRejectedValueOnce('db_error: disk full')
      .mockResolvedValueOnce(saved)
    const { store } = savingStore({ saveCapture })
    store.setCapture(capture)

    await store.saveCapture('draft-1')
    expect(get(store).saved).toEqual({})
    expect(get(store).saving).toEqual([])
    expect(get(store).saveErrors).toEqual({
      'draft-1': { code: 'db_error', detail: 'disk full' },
    })

    await store.saveCapture('draft-1')
    expect(get(store).saved).toEqual({ 'draft-1': saved })
    expect(get(store).saveErrors).toEqual({})
  })

  it('saves a verified PDF through its own command', async () => {
    const { deps, store } = savingStore()
    store.applyDownload(download('dl-1', { status: 'ready' }))

    await store.saveDownload('dl-1')

    expect(deps.saveDownload).toHaveBeenCalledWith('dl-1')
    expect(deps.saveCapture).not.toHaveBeenCalled()
    expect(get(store).saved['dl-1']).toEqual(saved)
  })

  it('tells the backend to forget a draft that is dismissed or replaced', () => {
    const { deps, store } = savingStore()
    store.setCapture(capture)
    store.setCapture({ ...capture, id: 'draft-2' })
    expect(deps.discardDraft).toHaveBeenCalledWith('draft-1')

    store.clearCapture()
    expect(deps.discardDraft).toHaveBeenCalledWith('draft-2')
    expect(get(store).capture).toBeNull()
  })

  it('keeps a failed discard from breaking the panel', async () => {
    const { store } = savingStore({ discardDraft: vi.fn().mockRejectedValue('nope') })
    store.setCapture(capture)
    expect(() => store.clearCapture()).not.toThrow()
    await Promise.resolve()
    expect(get(store).capture).toBeNull()
  })

  it('forgets what was saved when everything is cleared', async () => {
    const { store } = savingStore()
    store.setCapture(capture)
    await store.saveCapture('draft-1')
    store.clearAll()
    expect(get(store)).toMatchObject({ saved: {}, saving: [], saveErrors: {} })
  })

  it('does nothing when saving is not wired', async () => {
    const store = createNavegadorStore({ listen: fakeListen().listen })
    await expect(store.saveCapture('x')).resolves.toBeUndefined()
    expect(get(store).saved).toEqual({})
  })
})
