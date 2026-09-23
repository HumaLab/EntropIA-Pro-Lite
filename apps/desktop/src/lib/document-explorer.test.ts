import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { navigationRef } = vi.hoisted(() => ({
  navigationRef: {
    navigate: vi.fn(),
  },
}))

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

import { CREATE_COLLECTION_EVENT, requestCreateCollection } from './document-explorer'

describe('requestCreateCollection', () => {
  // `any`: vi.spyOn's overload resolution does not narrow cleanly through a
  // pre-declared ReturnType, and the project's lint config allows `any` here.
  let dispatchSpy: any

  beforeEach(() => {
    navigationRef.navigate.mockReset()
    dispatchSpy = vi.spyOn(window, 'dispatchEvent')
    vi.useFakeTimers()
  })

  afterEach(() => {
    dispatchSpy.mockRestore()
    vi.useRealTimers()
  })

  it('dispatches the create-collection event immediately when already on Colecciones', () => {
    requestCreateCollection(true)

    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(dispatchSpy).toHaveBeenCalledTimes(1)
    const dispatched = dispatchSpy.mock.calls[0]![0] as CustomEvent
    expect(dispatched.type).toBe(CREATE_COLLECTION_EVENT)
  })

  it('navigates to Colecciones first, then dispatches the event once CollectionsView has had a tick to mount', () => {
    requestCreateCollection(false)

    expect(navigationRef.navigate).toHaveBeenCalledWith({ name: 'collections' })
    expect(dispatchSpy).not.toHaveBeenCalled()

    vi.advanceTimersByTime(200)

    expect(dispatchSpy).toHaveBeenCalledTimes(1)
    const dispatched = dispatchSpy.mock.calls[0]![0] as CustomEvent
    expect(dispatched.type).toBe(CREATE_COLLECTION_EVENT)
  })
})
