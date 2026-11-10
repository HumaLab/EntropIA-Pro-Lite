import { describe, expect, it } from 'vitest'
import { relationGraphToGexf } from './relation-graph-gexf'

const node = (id: string, label: string) => ({ id, label, degree: 1, hiddenRelations: 0 })

describe('relationGraphToGexf', () => {
  it('writes nodes with their type and links with predicates and weight', () => {
    const gexf = relationGraphToGexf(
      {
        nodes: [node('perón', 'Perón'), node('cgt', 'CGT')],
        links: [
          { source: 'perón', target: 'cgt', predicates: ['habló ante', 'fundó'], counts: [2, 1] },
        ],
      },
      new Map([['perón', 'PER']])
    )
    const doc = new DOMParser().parseFromString(gexf, 'application/xml')
    expect(doc.querySelector('parsererror')).toBeNull()
    expect(doc.querySelector('graph')?.getAttribute('defaultedgetype')).toBe('directed')
    const nodes = [...doc.querySelectorAll('node')]
    expect(nodes.map((n) => n.getAttribute('label'))).toEqual(['Perón', 'CGT'])
    expect(nodes[0]!.querySelector('attvalue')?.getAttribute('value')).toBe('PER')
    expect(nodes[1]!.querySelector('attvalue')).toBeNull()
    const edge = doc.querySelector('edge')!
    expect(edge.getAttribute('source')).toBe('perón')
    expect(edge.getAttribute('target')).toBe('cgt')
    expect(edge.getAttribute('label')).toBe('habló ante · fundó')
    expect(edge.getAttribute('weight')).toBe('3')
  })

  it('escapes markup and drops control characters from document text', () => {
    const gexf = relationGraphToGexf({
      nodes: [node('a', 'Pérez & "Hnos" <S.A.>\u0001')],
      links: [],
    })
    expect(gexf).toContain('label="Pérez &amp; &quot;Hnos&quot; &lt;S.A.&gt;"')
    expect(gexf).not.toContain('\u0001')
  })
})
