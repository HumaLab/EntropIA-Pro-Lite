/**
 * Close-time flush handshake (the Rust half lives in `src-tauri/src/lib.rs`).
 *
 * On `app:closing` — emitted once per close attempt after the window's
 * `CloseRequested` is prevented — the frontend durably flushes the open writing
 * editor (`writing.flush()`, the canonical save) and acks exactly once through
 * `app_close_flushed`. Rust then runs one best-effort sync cycle and closes no
 * matter what: every phase is bounded, and durability is local-first (SQLite
 * WAL + savepoints), so a failed save or a slow server delays the push, never
 * local safety.
 *
 * The ack is sent even when `writing.flush()` rejects — the store already
 * surfaces the save failure to the writer, and withholding the ack would only
 * make close end through its timeout with the same outcome.
 */

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { writing } from '$lib/writing'

/** The Tauri event Rust emits when a close attempt starts. */
export const CLOSING_EVENT = 'app:closing'

/** The one-shot ack command Rust waits on after the canonical save. */
export const FLUSH_ACK_COMMAND = 'app_close_flushed'

let unlisten: UnlistenFn | null = null
let listening = false
let handled = false

/** Starts listening for the close handshake. Idempotent; never leaks a listener. */
export async function start(): Promise<void> {
  if (listening) return
  listening = true
  try {
    const registered = await listen(CLOSING_EVENT, () => {
      void handleClose()
    })
    if (!listening) {
      // stop() ran while listen() was in flight: unlisten the late
      // registration immediately instead of leaking it.
      registered()
      return
    }
    unlisten = registered
  } catch (error) {
    listening = false
    throw error
  }
}

/** Removes the listener and resets the handshake. Safe without `start()`. */
export function stop(): void {
  listening = false
  handled = false
  unlisten?.()
  unlisten = null
}

async function handleClose(): Promise<void> {
  // One close attempt, one flush, one ack — no matter how often the event lands.
  if (handled) return
  handled = true
  try {
    await writing.flush()
  } catch {
    // Best effort: the save failure is already visible in the store. The close
    // must still be acked or the window would only close by timeout.
  }
  try {
    await invoke(FLUSH_ACK_COMMAND)
  } catch {
    // Best effort: the ack is a courtesy signal; Rust always closes regardless.
  }
}
