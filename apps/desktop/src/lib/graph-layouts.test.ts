import { describe, expect, it } from 'vitest'
import { GRAPH_LAYOUTS, layoutConfig } from './graph-layouts'
import { normalizeGraphSettings, graphStructureKey } from './analysis-settings'

const render = normalizeGraphSettings(undefined).render

describe('graph layouts', () => {
  it('offers three layouts, free forces first and by default', () => {
    expect(GRAPH_LAYOUTS).toEqual(['force', 'communities', 'radial'])
    expect(render.layout).toBe('force')
  })

  it("keeps today's forces for the force layout", () => {
    const config = layoutConfig('force', render)
    expect(config.dagMode).toBeNull()
    expect(config.charge({ degree: 9 })).toBe(render.chargeStrength)
    expect(config.linkDistance).toBe(render.linkDistance)
  })

  it('repels hubs harder in the communities layout, so their neighbours cluster', () => {
    const config = layoutConfig('communities', render)
    expect(config.dagMode).toBeNull()
    expect(config.charge({ degree: 9 })).toBeLessThan(config.charge({ degree: 0 }))
    expect(config.charge({ degree: 0 })).toBe(render.chargeStrength)
    expect(config.linkDistance).toBeLessThan(render.linkDistance)
  })

  it('arranges rings from subjects to objects in the radial layout', () => {
    const config = layoutConfig('radial', render)
    expect(config.dagMode).toBe('radialout')
    expect(config.dagLevelDistance).toBeGreaterThan(0)
  })

  it('stores the layout as a render setting, falling back on unknown values', () => {
    expect(normalizeGraphSettings({ render: { layout: 'radial' } }).render.layout).toBe('radial')
    expect(normalizeGraphSettings({ render: { layout: 'spiral' } }).render.layout).toBe('force')
    const radial = normalizeGraphSettings({ render: { layout: 'radial' } })
    // A layout only moves nodes: it never rebuilds the graph from the triples.
    expect(graphStructureKey(radial)).toBe(graphStructureKey(normalizeGraphSettings(undefined)))
  })
})
