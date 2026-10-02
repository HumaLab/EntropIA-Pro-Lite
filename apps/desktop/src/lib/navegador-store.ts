/**
 * What the Navegador view shows besides the page: the browser's tabs, the
 * capture draft and the list of downloads.
 *
 * The view is unmounted whenever the person leaves the section or the tab, and
 * the browser (see `createViewerSession`) outlives it, so this state cannot
 * live in the component: a module-level store keeps it, and it keeps listening
 * to the backend while no view is mounted, so a download that finishes (or a
 * tab a page opens) while the person is elsewhere is there when they come back.
 */

import { get, writable, type Readable } from 'svelte/store'
import { onNavegadorState } from './navegador'
import {
  navegadorDiscardDraft,
  navegadorSaveCapture,
  navegadorSaveDownload,
  onNavegadorDownload,
  upsertDownload,
  parseSaveError,
  type CaptureDraft,
  type DownloadDraft,
  type SaveErrorCode,
  type SavedCapture,
} from './navegador-capture'
import { EMPTY_BROWSER, isNewer, type BrowserState } from './navegador-tabs'

/** Why saving an item failed; the view turns the code into a message. */
export type SaveFailure = { code: SaveErrorCode; detail: string | null }

export type NavegadorPanelState = {
  capture: CaptureDraft | null
  captureError: string | null
  downloads: DownloadDraft[]
  /** The browser's tabs and the active one, as the backend last said. */
  browser: BrowserState
  /** Drafts and downloads being saved right now, by id. */
  saving: string[]
  /** What was saved, by the id of the draft or download it came from. */
  saved: Record<string, SavedCapture>
  /** The last failure of each item that could not be saved. */
  saveErrors: Record<string, SaveFailure>
}

type Stop = () => void

/** Ids the person removed from the list, remembered so a late update for one
 *  of them (a download that was still running) does not bring it back. */
const MAX_DISMISSED = 100

const empty = (): NavegadorPanelState => ({
  capture: null,
  captureError: null,
  downloads: [],
  browser: EMPTY_BROWSER,
  saving: [],
  saved: {},
  saveErrors: {},
})

export function createNavegadorStore(deps: {
  listen: (handler: (draft: DownloadDraft) => void) => Promise<Stop>
  /** Follows the tabs; without it the store only holds what it is given. */
  listenState?: (handler: (state: BrowserState) => void) => Promise<Stop>
  /** Saving; without them the store never saves. */
  saveCapture?: (draftId: string) => Promise<SavedCapture>
  saveDownload?: (downloadId: string) => Promise<SavedCapture>
  /** Frees the HTML the backend holds for a draft nobody will save. */
  discardDraft?: (draftId: string) => Promise<void>
}) {
  const state = writable<NavegadorPanelState>(empty())
  let stop: Stop | null = null
  let starting: Promise<void> | null = null
  const dismissed = new Set<string>()

  const forget = (ids: readonly string[]) => {
    for (const id of ids) {
      dismissed.delete(id)
      dismissed.add(id)
    }
    while (dismissed.size > MAX_DISMISSED) {
      const oldest = dismissed.values().next().value
      if (oldest === undefined) break
      dismissed.delete(oldest)
    }
  }

  /** A draft that leaves the panel is no longer worth holding in the backend. */
  const discard = (draft: CaptureDraft | null) => {
    if (!draft || !deps.discardDraft) return
    void deps.discardDraft(draft.id).catch(() => undefined)
  }

  const save = async (
    id: string,
    run: ((id: string) => Promise<SavedCapture>) | undefined
  ): Promise<void> => {
    if (!run) return
    const current = get(state)
    if (current.saving.includes(id) || id in current.saved) return
    state.update((s) => {
      const { [id]: _cleared, ...saveErrors } = s.saveErrors
      return { ...s, saving: [...s.saving, id], saveErrors }
    })
    try {
      const saved = await run(id)
      state.update((s) => ({
        ...s,
        saving: s.saving.filter((entry) => entry !== id),
        saved: { ...s.saved, [id]: saved },
      }))
    } catch (reason) {
      state.update((s) => ({
        ...s,
        saving: s.saving.filter((entry) => entry !== id),
        saveErrors: { ...s.saveErrors, [id]: parseSaveError(reason) },
      }))
    }
  }

  return {
    subscribe: state.subscribe as Readable<NavegadorPanelState>['subscribe'],

    /** A capture draft replaces the previous one and clears its error. */
    setCapture(capture: CaptureDraft): void {
      const previous = get(state).capture
      if (previous && previous.id !== capture.id) discard(previous)
      state.update((s) => ({ ...s, capture, captureError: null }))
    },

    /** Save the capture draft `draftId` (a page or a selection). */
    saveCapture(draftId: string): Promise<void> {
      return save(draftId, deps.saveCapture)
    },

    /** Save the verified PDF `downloadId`. */
    saveDownload(downloadId: string): Promise<void> {
      return save(downloadId, deps.saveDownload)
    },

    setCaptureError(message: string): void {
      discard(get(state).capture)
      state.update((s) => ({ ...s, capture: null, captureError: message }))
    },

    clearCapture(): void {
      discard(get(state).capture)
      state.update((s) => ({ ...s, capture: null, captureError: null }))
    },

    /**
     * Take the backend's tabs. A command's answer and an event can arrive in
     * either order, so an older state than the one held is ignored.
     */
    applyBrowser(browser: BrowserState): void {
      state.update((s) => (isNewer(browser, s.browser) ? { ...s, browser } : s))
    },

    applyDownload(draft: DownloadDraft): void {
      if (dismissed.has(draft.id)) return
      state.update((s) => ({ ...s, downloads: upsertDownload(s.downloads, draft) }))
    },

    /**
     * Take one download off the list. Only the list entry goes: a PDF that
     * reached quarantine stays there until the 24 h sweep (or, later, until it
     * is saved), and a file saved to the person's folder is theirs.
     */
    dismissDownload(id: string): void {
      forget([id])
      state.update((s) => ({ ...s, downloads: s.downloads.filter((d) => d.id !== id) }))
    },

    /** Take every download off the list (same as dismissing each one). */
    clearDownloads(): void {
      state.update((s) => {
        forget(s.downloads.map((d) => d.id))
        return { ...s, downloads: [] }
      })
    },

    /**
     * Follow download and tab events for the rest of the session. Safe to call
     * from every view that mounts: it listens once, and tries again only after
     * a listen that failed.
     */
    startListening(): Promise<void> {
      if (stop) return Promise.resolve()
      if (!starting) {
        starting = Promise.all([
          deps.listen((draft) => this.applyDownload(draft)),
          deps.listenState?.((browser) => this.applyBrowser(browser)),
        ])
          .then(([unlistenDownloads, unlistenState]) => {
            stop = () => {
              unlistenDownloads()
              unlistenState?.()
            }
          })
          .finally(() => {
            starting = null
          })
      }
      return starting
    },

    /** Forget the tabs, the drafts and the downloads; keep listening. */
    clearAll(): void {
      discard(get(state).capture)
      dismissed.clear()
      state.set(empty())
    },

    /** Forget everything and stop listening (teardown and tests). */
    reset(): void {
      stop?.()
      stop = null
      dismissed.clear()
      state.set(empty())
    },
  }
}

export const navegadorStore = createNavegadorStore({
  listen: onNavegadorDownload,
  listenState: onNavegadorState,
  saveCapture: navegadorSaveCapture,
  saveDownload: navegadorSaveDownload,
  discardDraft: navegadorDiscardDraft,
})
