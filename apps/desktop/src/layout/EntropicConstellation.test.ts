import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { render } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import EntropicConstellation from './EntropicConstellation.svelte'

function readSource() {
  return readFileSync(resolve(import.meta.dirname, 'EntropicConstellation.svelte'), 'utf-8')
}

describe('EntropicConstellation visual contract', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders as a non-interactive canvas layer', () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn(() => ({ matches: false }))
    )
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)

    const { container } = render(EntropicConstellation)
    const canvas = container.querySelector('canvas')

    expect(canvas).toBeInTheDocument()
    expect(canvas).toHaveAttribute('aria-hidden', 'true')
  })

  it('keeps the main background subdued and canvas-rendered', () => {
    const source = readSource()

    expect(source).toContain('const MIN_NODES = 180')
    expect(source).toContain('const MAX_NODES = 520')
    expect(source).toContain('const NODE_DENSITY = 4200')
    expect(source).toContain("canvas.getContext('2d', { alpha: false })")
    expect(source).toContain('function buildSpatialGrid()')
    expect(source).toContain('function renderConstellation()')
    expect(source).toContain('aria-hidden="true"')
  })

  it('uses theme tokens for the canvas background instead of fixed dark colors', () => {
    const source = readSource()

    expect(source).toContain('--constellation-bg-start: var(--surface-app, var(--color-bg))')
    expect(source).toContain(
      "readThemeColor('--constellation-bg-start', '--surface-app', '--color-bg')"
    )
    expect(source).toContain(
      "readThemeColor('--constellation-bg-mid', '--color-bg-ambient', '--color-surface')"
    )
    expect(source).toContain(
      "readThemeColor('--constellation-bg-end', '--surface-panel', '--color-surface')"
    )
    expect(source).not.toMatch(/#080a10|#0c0f17|#10131b/i)
    expect(source).not.toContain('rgba(75, 83, 106, 0.035)')
    expect(source).not.toContain('rgba(38, 44, 62, 0.022)')
    expect(source).not.toContain('rgba(8, 10, 16, 0)')
  })

  it('uses lightweight transform drift instead of continuous canvas redraws', () => {
    const source = readSource()

    expect(source).toContain("'(prefers-reduced-motion: reduce)'")
    expect(source).toContain('const MAX_DEVICE_PIXEL_RATIO = 1.35')
    expect(source).toContain('const CANVAS_OVERSCAN = 140')
    expect(source).toContain("window.addEventListener('resize', scheduleResize)")
    expect(source).toContain('class:constellation--motion={!reducedMotion}')
    expect(source).toContain('@keyframes entropic-drift')
    expect(source).not.toContain('requestAnimationFrame')
  })
})
