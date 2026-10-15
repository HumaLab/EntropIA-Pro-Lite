/**
 * Startup reconcile for Internet Archive leftovers (Phase 4, task 26b):
 * releases recovery leftovers once per session through
 * `resolveReconcileLeftovers()`. Never throws — a reconcile failure is a
 * warning, never a startup blocker — and success is remembered only after
 * the call resolves, so a failure allows a later retry. Concurrent calls
 * share one in-flight promise, so `App.svelte` can call it on every mount
 * without tracking.
 */
import { resolveReconcileLeftovers } from './internet-archive-import'

let reconciled = false
let inFlight: Promise<void> | null = null

/** Release IA recovery leftovers once; safe to call any number of times. */
export async function runStartupReconcile(): Promise<void> {
  if (reconciled) return
  if (inFlight) return inFlight
  inFlight = (async () => {
    try {
      await resolveReconcileLeftovers()
      reconciled = true
    } catch (reason) {
      console.warn('[internet-archive] startup reconcile failed:', reason)
    } finally {
      inFlight = null
    }
  })()
  return inFlight
}
