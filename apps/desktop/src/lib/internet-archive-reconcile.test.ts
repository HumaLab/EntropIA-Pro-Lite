import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { resolveRef } = vi.hoisted(() => ({
  resolveRef: {
    resolveReconcileLeftovers: vi.fn(),
  },
}))

vi.mock('$lib/internet-archive-import', () => ({
  resolveReconcileLeftovers: resolveRef.resolveReconcileLeftovers,
}))

beforeEach(() => {
  vi.resetModules()
  resolveRef.resolveReconcileLeftovers.mockReset()
  resolveRef.resolveReconcileLeftovers.mockResolvedValue({ released: 0, failed: [] })
  vi.spyOn(console, 'warn').mockImplementation(() => {})
})

afterEach(() => {
  vi.restoreAllMocks()
})

async function loadRunStartupReconcile(): Promise<() => Promise<void>> {
  const module = await import('$lib/internet-archive-reconcile')
  return module.runStartupReconcile
}

describe('runStartupReconcile', () => {
  it('calls resolveReconcileLeftovers once', async () => {
    const runStartupReconcile = await loadRunStartupReconcile()

    await runStartupReconcile()

    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledOnce()
  })

  it('never throws and warns when the reconcile fails', async () => {
    resolveRef.resolveReconcileLeftovers.mockRejectedValue(new Error('db locked'))
    const runStartupReconcile = await loadRunStartupReconcile()

    await expect(runStartupReconcile()).resolves.toBeUndefined()
    expect(console.warn).toHaveBeenCalledOnce()
  })

  it('is a no-op on the second call', async () => {
    const runStartupReconcile = await loadRunStartupReconcile()

    await runStartupReconcile()
    await runStartupReconcile()

    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledOnce()
  })

  it('retries after a failure and succeeds', async () => {
    resolveRef.resolveReconcileLeftovers
      .mockRejectedValueOnce(new Error('db locked'))
      .mockResolvedValueOnce({ released: 1, failed: [] })
    const runStartupReconcile = await loadRunStartupReconcile()

    await runStartupReconcile()
    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledTimes(1)
    await runStartupReconcile()
    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledTimes(2)
    // After success, further calls are a no-op.
    await runStartupReconcile()
    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledTimes(2)
  })

  it('shares one in-flight promise across concurrent calls', async () => {
    let release!: () => void
    const gate = new Promise<{ released: number; failed: [] }>((resolve) => {
      release = () => resolve({ released: 0, failed: [] })
    })
    resolveRef.resolveReconcileLeftovers.mockReturnValue(gate)
    const runStartupReconcile = await loadRunStartupReconcile()

    const first = runStartupReconcile()
    const second = runStartupReconcile()
    // The second call must not resolve before the shared work does.
    let secondSettled = false
    void second.then(() => {
      secondSettled = true
    })
    await Promise.resolve()
    await Promise.resolve()
    expect(secondSettled).toBe(false)
    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledTimes(1)
    release()
    await first
    await second
    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledTimes(1)
    // After the shared run succeeds, later calls are a no-op.
    await runStartupReconcile()
    expect(resolveRef.resolveReconcileLeftovers).toHaveBeenCalledTimes(1)
  })
})
