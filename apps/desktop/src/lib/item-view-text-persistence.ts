type Timer = ReturnType<typeof setTimeout>

type PendingPayload = { text: string }

export class DebouncedAssetTextPersistor {
  private timers = new Map<string, Timer>()
  private pending = new Map<string, PendingPayload>()
  private inFlight = new Map<string, Map<PendingPayload, Promise<void>>>()

  constructor(
    private readonly options: {
      delayMs: number
      persist: (assetId: string, text: string) => Promise<unknown>
      afterPersist?: (assetId: string, text: string) => void
      onError?: (error: unknown) => void
    }
  ) {}

  private clearTimer(assetId: string) {
    const timer = this.timers.get(assetId)
    if (!timer) return
    clearTimeout(timer)
    this.timers.delete(assetId)
  }

  private startPersistence(assetId: string, payload: PendingPayload): Promise<void> {
    const assetWrites = this.inFlight.get(assetId) ?? new Map<PendingPayload, Promise<void>>()
    const existing = assetWrites.get(payload)
    if (existing) return existing
    const priorWrites = [...assetWrites.values()]

    const persistence = (async () => {
      try {
        await Promise.allSettled(priorWrites)
        if (this.pending.get(assetId) !== payload) return
        await this.options.persist(assetId, payload.text)
        if (this.pending.get(assetId) === payload) {
          this.pending.delete(assetId)
        }
        this.options.afterPersist?.(assetId, payload.text)
      } catch (error) {
        this.options.onError?.(error)
        throw error
      } finally {
        assetWrites.delete(payload)
        if (assetWrites.size === 0 && this.inFlight.get(assetId) === assetWrites) {
          this.inFlight.delete(assetId)
        }
      }
    })()
    assetWrites.set(payload, persistence)
    this.inFlight.set(assetId, assetWrites)
    return persistence
  }

  schedule(assetId: string, text: string) {
    this.cancel(assetId)
    const payload = { text }
    this.pending.set(assetId, payload)

    const timer = setTimeout(() => {
      if (this.timers.get(assetId) === timer) {
        this.timers.delete(assetId)
      }
      if (this.pending.get(assetId) !== payload) return
      void this.startPersistence(assetId, payload).catch(() => {
        // Scheduled persistence reports through onError and remains retryable.
      })
    }, this.options.delayMs)
    this.timers.set(assetId, timer)
  }

  cancel(assetId: string) {
    this.clearTimer(assetId)
    this.pending.delete(assetId)
  }

  async flushAndWait(assetId: string): Promise<void> {
    while (true) {
      this.clearTimer(assetId)
      const pending = this.pending.get(assetId)
      if (pending) {
        this.startPersistence(assetId, pending)
      }
      const writes = [...(this.inFlight.get(assetId)?.values() ?? [])]
      if (writes.length === 0) return

      const results = await Promise.allSettled(writes)
      const failure = results.find(
        (result): result is PromiseRejectedResult => result.status === 'rejected'
      )
      if (failure) throw failure.reason
    }
  }

  async cancelAndWait(assetId: string): Promise<void> {
    while (true) {
      this.cancel(assetId)
      const writes = [...(this.inFlight.get(assetId)?.values() ?? [])]
      if (writes.length === 0) return
      await Promise.allSettled(writes)
    }
  }

  cancelAll() {
    for (const timer of this.timers.values()) {
      clearTimeout(timer)
    }
    this.timers.clear()
    this.pending.clear()
  }
}
