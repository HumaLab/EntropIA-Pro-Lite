/**
 * What the Navegador view shows besides the page: the capture draft and the
 * list of downloads.
 *
 * The view is unmounted whenever the person leaves the section or the tab, and
 * the browser (see `createViewerSession`) outlives it, so this state cannot
 * live in the component: a module-level store keeps it, and it keeps listening
 * to download events while no view is mounted, so a download that finishes
 * while the person is elsewhere is in the list when they come back.
 */

import { writable, type Readable } from 'svelte/store'
import {
  onNavegadorDownload,
  upsertDownload,
  type CaptureDraft,
  type DownloadDraft,
} from './navegador-capture'

export type NavegadorPanelState = {
  capture: CaptureDraft | null
  captureError: string | null
  downloads: DownloadDraft[]
}

type Stop = () => void

const empty = (): NavegadorPanelState => ({ capture: null, captureError: null, downloads: [] })

export function createNavegadorStore(deps: {
  listen: (handler: (draft: DownloadDraft) => void) => Promise<Stop>
}) {
  const state = writable<NavegadorPanelState>(empty())
  let stop: Stop | null = null
  let starting: Promise<void> | null = null

  return {
    subscribe: state.subscribe as Readable<NavegadorPanelState>['subscribe'],

    /** A capture draft replaces the previous one and clears its error. */
    setCapture(capture: CaptureDraft): void {
      state.update((s) => ({ ...s, capture, captureError: null }))
    },

    setCaptureError(message: string): void {
      state.update((s) => ({ ...s, capture: null, captureError: message }))
    },

    clearCapture(): void {
      state.update((s) => ({ ...s, capture: null, captureError: null }))
    },

    applyDownload(draft: DownloadDraft): void {
      state.update((s) => ({ ...s, downloads: upsertDownload(s.downloads, draft) }))
    },

    /**
     * Follow download events for the rest of the session. Safe to call from
     * every view that mounts: it listens once, and tries again only after a
     * listen that failed.
     */
    startListening(): Promise<void> {
      if (stop) return Promise.resolve()
      if (!starting) {
        starting = deps
          .listen((draft) => this.applyDownload(draft))
          .then((unlisten) => {
            stop = unlisten
          })
          .finally(() => {
            starting = null
          })
      }
      return starting
    },

    /** Forget the drafts and the downloads; keep listening. */
    clearAll(): void {
      state.set(empty())
    },

    /** Forget everything and stop listening (teardown and tests). */
    reset(): void {
      stop?.()
      stop = null
      state.set(empty())
    },
  }
}

export const navegadorStore = createNavegadorStore({ listen: onNavegadorDownload })
