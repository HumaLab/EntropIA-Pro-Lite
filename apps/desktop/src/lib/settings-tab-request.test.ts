import { afterEach, describe, expect, it } from 'vitest'
import { onSettingsTabRequest, requestSettingsTab } from './settings-tab-request'

describe('settings tab request', () => {
  afterEach(() => {
    // Drain anything a test left pending.
    onSettingsTabRequest(() => {})()
  })

  it('delivers a request made before Settings mounts', () => {
    requestSettingsTab('sync')
    const received: string[] = []
    const stop = onSettingsTabRequest((tab) => received.push(tab))
    expect(received).toEqual(['sync'])
    stop()
  })

  it('is consumed once, so the next plain visit opens the default tab', () => {
    requestSettingsTab('sync')
    onSettingsTabRequest(() => {})()
    const received: string[] = []
    const stop = onSettingsTabRequest((tab) => received.push(tab))
    expect(received).toEqual([])
    stop()
  })

  it('reaches a Settings view that is already open', () => {
    const received: string[] = []
    const stop = onSettingsTabRequest((tab) => received.push(tab))
    requestSettingsTab('sync')
    expect(received).toEqual(['sync'])
    stop()
  })
})
