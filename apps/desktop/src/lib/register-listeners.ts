import type { UnlistenFn } from '@tauri-apps/api/event'

/**
 * Register listeners one at a time. If one registration rejects, the ones
 * already registered are released before the error propagates, so a partial
 * failure leaks nothing. If `isCancelled()` is true once all are registered
 * (the owner was destroyed while awaiting), they are released and [] is
 * returned.
 */
export async function registerListeners(
  factories: ReadonlyArray<() => Promise<UnlistenFn>>,
  isCancelled: () => boolean = () => false
): Promise<UnlistenFn[]> {
  const registered: UnlistenFn[] = []
  try {
    for (const factory of factories) {
      registered.push(await factory())
    }
  } catch (error) {
    releaseAll(registered)
    throw error
  }
  if (isCancelled()) {
    releaseAll(registered)
    return []
  }
  return registered
}

/** Release every unlisten, swallowing errors so one cannot mask another. */
function releaseAll(unlisteners: ReadonlyArray<UnlistenFn>): void {
  for (const unlisten of unlisteners) {
    try {
      unlisten()
    } catch {
      // Best effort: the original registration error must keep propagating.
    }
  }
}
