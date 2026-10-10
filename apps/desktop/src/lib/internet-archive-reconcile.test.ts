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
})
