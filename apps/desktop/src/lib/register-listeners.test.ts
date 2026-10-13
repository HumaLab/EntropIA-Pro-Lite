import { describe, expect, it, vi } from 'vitest'
import { registerListeners } from './register-listeners'

describe('registerListeners', () => {
  it('returns the unlisten functions in order and calls none', async () => {
    const first = vi.fn()
    const second = vi.fn()
    const third = vi.fn()

    const registered = await registerListeners([
      () => Promise.resolve(first),
      () => Promise.resolve(second),
      () => Promise.resolve(third),
    ])

    expect(registered).toEqual([first, second, third])
    expect(first).not.toHaveBeenCalled()
    expect(second).not.toHaveBeenCalled()
    expect(third).not.toHaveBeenCalled()
  })

  it('releases the ones already registered and rethrows when a later factory rejects', async () => {
    const first = vi.fn()
    const second = vi.fn()
    const fourth = vi.fn()
    const failure = new Error('registration failed')

    const registration = registerListeners([
      () => Promise.resolve(first),
      () => Promise.resolve(second),
      () => Promise.reject(failure),
      () => {
        fourth()
        return Promise.resolve(vi.fn())
      },
    ])

    await expect(registration).rejects.toBe(failure)
    expect(first).toHaveBeenCalledOnce()
    expect(second).toHaveBeenCalledOnce()
    expect(fourth).not.toHaveBeenCalled()
  })

  it('releases every listener and returns [] when cancelled after registration', async () => {
    const first = vi.fn()
    const second = vi.fn()

    const registered = await registerListeners(
      [() => Promise.resolve(first), () => Promise.resolve(second)],
      () => true
    )

    expect(registered).toEqual([])
    expect(first).toHaveBeenCalledOnce()
    expect(second).toHaveBeenCalledOnce()
  })

  it('still propagates the original error when an unlisten throws during cleanup', async () => {
    const healthy = vi.fn()
    const throwing = vi.fn(() => {
      throw new Error('unlisten failed')
    })
    const failure = new Error('registration failed')

    await expect(
      registerListeners([
        () => Promise.resolve(throwing),
        () => Promise.resolve(healthy),
        () => Promise.reject(failure),
      ])
    ).rejects.toBe(failure)

    expect(throwing).toHaveBeenCalledOnce()
    expect(healthy).toHaveBeenCalledOnce()
  })
})
