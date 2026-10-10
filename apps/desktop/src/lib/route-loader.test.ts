import { describe, expect, it, vi } from 'vitest'
import { createRouteLoader, loadRouteView } from './route-loader'

// The mapping is what is under test, not the views: compiling them cold took
// past the 5 s timeout under the full parallel suite.
vi.mock('../views/BibliotecaView.svelte', () => ({ default: 'biblioteca-view' }))
vi.mock('../views/BibliographyWorkView.svelte', () => ({ default: 'bibliography-work-view' }))

describe('createRouteLoader', () => {
  it('shares one promise for repeated loads of the same route', async () => {
    const module = { default: 'item-view' }
    const importRoute = vi.fn<() => Promise<typeof module>>().mockResolvedValue(module)
    const load = createRouteLoader({ item: importRoute })

    const first = load('item')
    const second = load('item')

    expect(second).toBe(first)
    await expect(first).resolves.toBe(module)
    expect(importRoute).toHaveBeenCalledTimes(1)
  })

  it('evicts a rejected promise so retry performs a fresh import', async () => {
    const importRoute = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error('chunk unavailable'))
      .mockResolvedValueOnce('loaded')
    const load = createRouteLoader({ item: importRoute })

    await expect(load('item')).rejects.toThrow('chunk unavailable')
    await expect(load('item')).resolves.toBe('loaded')
    expect(importRoute).toHaveBeenCalledTimes(2)
  })
})

describe('loadRouteView — Biblioteca routes', () => {
  it('loads the Biblioteca views under their route names', async () => {
    const biblioteca = await loadRouteView('biblioteca')
    const work = await loadRouteView('bibliography-work')

    expect(biblioteca.default).toBe('biblioteca-view')
    expect(work.default).toBe('bibliography-work-view')
  })
})
