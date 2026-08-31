import { describe, expect, it } from 'vitest'
import { ACTION_ICON_NAMES } from '@entropia/ui'
import { dependencyStatusIcon } from './deps-status'
import type { DependencyStatus } from './deps'

const EVERY_STATUS: DependencyStatus[] = [
  { type: 'unknown' },
  { type: 'checking' },
  { type: 'installed' },
  { type: 'installed', version: '3.12.1' },
  { type: 'missing' },
  { type: 'installing', percent: 0 },
  { type: 'installing', percent: 64 },
  { type: 'failed', message: 'paddle wheel not found' },
]

describe('dependencyStatusIcon', () => {
  it('maps every dependency status to a name in the icon catalogue', () => {
    for (const status of EVERY_STATUS) {
      expect(ACTION_ICON_NAMES).toContain(dependencyStatusIcon(status))
    }
  })

  it('distinguishes the outcomes by shape, not only by colour', () => {
    // Colour alone fails for a colour-blind reader: installed/missing/failed
    // must be three different glyphs, not three tints of one.
    expect(dependencyStatusIcon({ type: 'installed' })).toBe('circle-check')
    expect(dependencyStatusIcon({ type: 'missing' })).toBe('circle-x')
    expect(dependencyStatusIcon({ type: 'failed', message: 'boom' })).toBe('triangle-alert')
  })

  it('shows the same in-progress glyph while checking and while installing', () => {
    expect(dependencyStatusIcon({ type: 'checking' })).toBe('loader')
    expect(dependencyStatusIcon({ type: 'installing', percent: 12 })).toBe('loader')
  })

  it('falls back to a question mark glyph for an unknown status', () => {
    expect(dependencyStatusIcon({ type: 'unknown' })).toBe('circle-help')
  })
})
