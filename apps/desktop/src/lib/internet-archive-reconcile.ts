/**
 * Startup reconcile for Internet Archive leftovers (Phase 4, task 26b):
 * releases recovery leftovers once per session through
 * `resolveReconcileLeftovers()`. Never throws — a reconcile failure is a
 * warning, never a startup blocker — and a second call is a no-op, so
 * `App.svelte` can call it on every mount without tracking.
 */
import { resolveReconcileLeftovers } from './internet-archive-import'

let reconciled = false

/** Release IA recovery leftovers once; safe to call any number of times. */
export async function runStartupReconcile(): Promise<void> {
  if (reconciled) return
  reconciled = true
  try {
    await resolveReconcileLeftovers()
  } catch (reason) {
    console.warn('[internet-archive] startup reconcile failed:', reason)
  }
}
