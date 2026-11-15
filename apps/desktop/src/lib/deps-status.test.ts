import { describe, expect, it } from 'vitest'
import { ACTION_ICON_NAMES } from '@entropia/ui'
import { dependencyStatusIcon, dependencyStatusIconClass } from './deps-status'
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

describe('dependencyStatusIconClass', () => {
  it('maps each of the four colour states to its modifier class', () => {
    // The class picks the token in DependenciasTab's stylesheet: installed is
    // success, missing is danger, failed is warning (not danger) and every
    // in-progress or unknown status is the muted neutral.
    expect(dependencyStatusIconClass({ type: 'installed' })).toBe('deps-row__icon--installed')
    expect(dependencyStatusIconClass({ type: 'installed', version: '3.12.1' })).toBe(
      'deps-row__icon--installed'
    )
    expect(dependencyStatusIconClass({ type: 'missing' })).toBe('deps-row__icon--missing')
    expect(dependencyStatusIconClass({ type: 'failed', message: 'boom' })).toBe(
      'deps-row__icon--failed'
    )
    expect(dependencyStatusIconClass({ type: 'checking' })).toBe('deps-row__icon--neutral')
    expect(dependencyStatusIconClass({ type: 'installing', percent: 40 })).toBe(
      'deps-row__icon--neutral'
    )
    expect(dependencyStatusIconClass({ type: 'unknown' })).toBe('deps-row__icon--neutral')
  })

  it('gives every dependency status one of the four classes', () => {
    const classes = new Set(EVERY_STATUS.map(dependencyStatusIconClass))
    expect([...classes].sort()).toEqual([
      'deps-row__icon--failed',
      'deps-row__icon--installed',
      'deps-row__icon--missing',
      'deps-row__icon--neutral',
    ])
  })
})
