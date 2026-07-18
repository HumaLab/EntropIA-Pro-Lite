import { describe, expect, it } from 'vitest'

import { resolveDesktopPlatform } from './platform'

describe('resolveDesktopPlatform', () => {
  it('classifies Darwin as macOS before matching the broader win substring', () => {
    expect(resolveDesktopPlatform({ platform: 'Darwin' } as Navigator)).toBe('macos')
  })

  it('still classifies Windows platforms correctly', () => {
    expect(resolveDesktopPlatform({ platform: 'Win32' } as Navigator)).toBe('windows')
  })
})
