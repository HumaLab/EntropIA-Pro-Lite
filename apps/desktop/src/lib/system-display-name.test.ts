import { describe, expect, it } from 'vitest'
import liteTauriConfigJson from '../../src-tauri/tauri.lite.conf.json' with { type: 'json' }
import proTauriConfigJson from '../../src-tauri/tauri.conf.json' with { type: 'json' }

type WindowConfig = Record<string, unknown> & { title?: string }

interface SystemDisplayConfig {
  productName: string
  app?: {
    windows?: WindowConfig[]
  }
}

const liteTauriConfig = liteTauriConfigJson as SystemDisplayConfig
const proTauriConfig = proTauriConfigJson as SystemDisplayConfig

function windowSettingsWithoutTitle(window: WindowConfig | undefined): Record<string, unknown> {
  expect(window).toBeDefined()
  const { title: _title, ...settings } = window ?? {}
  return settings
}

describe('Tauri system display metadata', () => {
  it('uses edition-specific names without changing main-window behavior', () => {
    const liteWindow = liteTauriConfig.app?.windows?.[0]
    const proWindow = proTauriConfig.app?.windows?.[0]

    expect(liteTauriConfig.productName).toBe('EntropIA Lite')
    expect(liteWindow?.title).toBe('EntropIA Lite')
    expect(proTauriConfig.productName).toBe('EntropIA Pro')
    expect(proWindow?.title).toBe('EntropIA Pro')
    expect(windowSettingsWithoutTitle(liteWindow)).toEqual(windowSettingsWithoutTitle(proWindow))
  })
})
