import { describe, expect, it, vi } from 'vitest'

const { navigateActive } = vi.hoisted(() => ({ navigateActive: vi.fn() }))
vi.mock('$lib/workspace', () => ({ workspace: { navigateActive } }))

import { openDependenciesSettings } from './dictation-help'
import { onSettingsTabRequest } from './settings-tab-request'

describe('openDependenciesSettings', () => {
  it('opens Settings on the Dependencias de IA tab', () => {
    openDependenciesSettings()

    expect(navigateActive).toHaveBeenCalledWith({ name: 'settings' })
    const opened: string[] = []
    const stop = onSettingsTabRequest((tab) => opened.push(tab))
    stop()
    expect(opened).toEqual(['dependencias'])
  })
})
