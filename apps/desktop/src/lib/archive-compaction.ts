/**
 * Close-time compaction notice (the Rust half is `src-tauri/src/archive_close.rs`).
 *
 * After the canonical save and the sync cycle, Rust may rewrite the archive to
 * give its free pages back to the disk. That takes a while on a big archive, so
 * the window stays open and shows a small notice: Rust emits `app:compacting`
 * with `true` when it starts and `false` when it ends, whatever the outcome.
 * The notice is not interactive and never decides anything; Rust closes the
 * window itself when it is done.
 */

import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { writable, type Readable } from 'svelte/store'

/** The Tauri event Rust emits when the compaction starts and ends. */
export const COMPACTING_EVENT = 'app:compacting'

const active = writable(false)

/** True while the archive is being compacted. */
export const compacting: Readable<boolean> = { subscribe: active.subscribe }

let unlisten: UnlistenFn | null = null
let listening = false

/** Starts listening. Idempotent; never leaks a listener. */
export async function start(): Promise<void> {
  if (listening) return
  listening = true
  try {
    const registered = await listen<boolean>(COMPACTING_EVENT, (event) => {
      active.set(event.payload === true)
    })
    if (!listening) {
      registered()
      return
    }
    unlisten = registered
  } catch (error) {
    listening = false
    throw error
  }
}

/** Removes the listener and clears the notice. Safe without `start()`. */
export function stop(): void {
  listening = false
  unlisten?.()
  unlisten = null
  active.set(false)
}
