import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest'

const { workspaceRef } = vi.hoisted(() => ({
  workspaceRef: {
    navigateActive: vi.fn(),
  },
}))

vi.mock('$lib/workspace', () => ({
  workspace: workspaceRef,
}))

import { CREATE_COLLECTION_EVENT, requestCreateCollection } from './document-explorer'

describe('requestCreateCollection', () => {
  // Typed spy: `MockInstance<typeof window.dispatchEvent>` records the real
  // `Event` signature, so the call reads below need no escaping cast.
  let dispatchSpy: MockInstance<typeof window.dispatchEvent>

  beforeEach(() => {
    workspaceRef.navigateActive.mockReset()
    dispatchSpy = vi.spyOn(window, 'dispatchEvent')
    vi.useFakeTimers()
  })

  afterEach(() => {
    dispatchSpy.mockRestore()
    vi.useRealTimers()
  })

  it('dispatches the create-collection event immediately when already on Colecciones', () => {
    requestCreateCollection(true)

    expect(workspaceRef.navigateActive).not.toHaveBeenCalled()
    expect(dispatchSpy).toHaveBeenCalledTimes(1)
    const dispatched = dispatchSpy.mock.calls[0]![0] as CustomEvent
    expect(dispatched.type).toBe(CREATE_COLLECTION_EVENT)
  })

  it('navigates to Colecciones first, then dispatches the event once CollectionsView has had a tick to mount', () => {
    requestCreateCollection(false)

    expect(workspaceRef.navigateActive).toHaveBeenCalledWith({ name: 'collections' })
    expect(dispatchSpy).not.toHaveBeenCalled()

    vi.advanceTimersByTime(200)

    expect(dispatchSpy).toHaveBeenCalledTimes(1)
    const dispatched = dispatchSpy.mock.calls[0]![0] as CustomEvent
    expect(dispatched.type).toBe(CREATE_COLLECTION_EVENT)
  })
})
