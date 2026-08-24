type Timer = ReturnType<typeof setTimeout>

export class DebouncedAssetTextPersistor {
  private timers = new Map<string, Timer>()
  private inFlight = new Map<string, Set<Promise<void>>>()

  constructor(
    private readonly options: {
      delayMs: number
      persist: (assetId: string, text: string) => Promise<unknown>
      afterPersist?: (assetId: string, text: string) => void
      onError?: (error: unknown) => void
    }
  ) {}

  schedule(assetId: string, text: string) {
    this.cancel(assetId)

    const timer = setTimeout(() => {
      this.timers.delete(assetId)
      const persistence = (async () => {
        try {
          await this.options.persist(assetId, text)
          this.options.afterPersist?.(assetId, text)
        } catch (error) {
          this.options.onError?.(error)
        }
      })()
      const assetPersistence = this.inFlight.get(assetId) ?? new Set<Promise<void>>()
      assetPersistence.add(persistence)
      this.inFlight.set(assetId, assetPersistence)
      void persistence.finally(() => {
        assetPersistence.delete(persistence)
        if (assetPersistence.size === 0 && this.inFlight.get(assetId) === assetPersistence) {
          this.inFlight.delete(assetId)
        }
      })
    }, this.options.delayMs)

    this.timers.set(assetId, timer)
  }

  cancel(assetId: string) {
    const existing = this.timers.get(assetId)
    if (existing) {
      clearTimeout(existing)
      this.timers.delete(assetId)
    }
  }

  async cancelAndWait(assetId: string): Promise<void> {
    while (true) {
      this.cancel(assetId)
      const pending = [...(this.inFlight.get(assetId) ?? [])]
      if (pending.length === 0) return
      await Promise.all(pending)
    }
  }

  cancelAll() {
    for (const timer of this.timers.values()) {
      clearTimeout(timer)
    }
    this.timers.clear()
  }
}
