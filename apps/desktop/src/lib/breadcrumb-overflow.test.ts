import { describe, it, expect } from 'vitest'
import { collapseBreadcrumb } from './breadcrumb-overflow'

describe('collapseBreadcrumb', () => {
  it('returns every crumb unchanged when it fits', () => {
    expect(collapseBreadcrumb(['Colecciones', 'Archivo'], 3)).toEqual([
      { label: 'Colecciones', index: 0, collapsed: false },
      { label: 'Archivo', index: 1, collapsed: false },
    ])
  })

  it('collapses the middle into … when it overflows, keeping first and last', () => {
    const result = collapseBreadcrumb(['Colecciones', 'Investigación', 'Pregunta larga'], 2)
    expect(result).toEqual([
      { label: 'Colecciones', index: 0, collapsed: false },
      { label: '…', index: -1, collapsed: true },
      { label: 'Pregunta larga', index: 2, collapsed: false },
    ])
  })

  it('never collapses two crumbs, however small maxVisible is', () => {
    expect(collapseBreadcrumb(['Colecciones', 'Archivo'], 1)).toHaveLength(2)
  })
})
