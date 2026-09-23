import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { render } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import EntropicConstellation from './EntropicConstellation.svelte'

function functionBody(source: string, name: string): string {
  const start = source.indexOf(`function ${name}(`)
  if (start < 0) throw new Error(`function ${name} not found`)
  const open = source.indexOf('{', start)
  let depth = 0
  for (let i = open; i < source.length; i++) {
    if (source[i] === '{') depth++
    else if (source[i] === '}' && --depth === 0) return source.slice(open, i + 1)
  }
  throw new Error(`function ${name} is not closed`)
}

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

  it('redraws continuously only on Inicio, never for the static field', () => {
    // Until 2026-09-23 the field never redrew continuously anywhere (this test
    // asserted no requestAnimationFrame at all). The user then chose to animate
    // it on Inicio only. The rule is now: the static field (every other view)
    // is drawn once per resize, and the loop exists only in the animated mode,
    // which AppShell turns on for the home view alone.
    const source = readSource()

    expect(source).toContain("'(prefers-reduced-motion: reduce)'")
    expect(source).toContain('const MAX_DEVICE_PIXEL_RATIO = 1.35')
    expect(source).toContain('const CANVAS_OVERSCAN = 140')
    expect(source).toContain("window.addEventListener('resize', scheduleResize)")
    expect(source).toContain('class:constellation--motion={!reducedMotion}')

    // The static render path never schedules a frame.
    const staticRender = functionBody(source, 'renderConstellation')
    expect(staticRender).not.toMatch(/scheduleFrame|requestAnimationFrame/)

    // The loop only starts behind the animated prop.
    expect(source).toMatch(/if \(animated\) \{\s*if \(!motionMode\) startMotion\(\)/)

    // And that prop is on for the home view only.
    const shell = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')
    expect(shell).toContain(
      "<EntropicConstellation animated={$navigation.current.name === 'home'} />"
    )
  })
})
