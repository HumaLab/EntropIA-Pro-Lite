import { describe, expect, it } from 'vitest'
import { parseFlows, upsertFlow, type BatchFlow } from './batch-flows'

const steps = { ocr: true, embeddings: false, ner: true, triples: false, schemaId: 's1' }

describe('batch flows', () => {
  it('reads valid flows and drops malformed ones', () => {
    const raw = JSON.stringify([
      { name: 'Partes de puerto', steps },
      { name: '', steps },
      { name: 'Sin pasos' },
      'basura',
    ])
    expect(parseFlows(raw)).toEqual([{ name: 'Partes de puerto', steps }])
    expect(parseFlows('{no es json')).toEqual([])
    expect(parseFlows(null)).toEqual([])
  })

  it('replaces a flow with the same name and keeps the list sorted', () => {
    const flows: BatchFlow[] = [
      { name: 'Prensa', steps },
      { name: 'Partes de puerto', steps },
    ]
    const updated = upsertFlow(flows, { name: 'partes de puerto', steps: { ...steps, ocr: false } })
    expect(updated.map((flow) => flow.name)).toEqual(['partes de puerto', 'Prensa'])
    expect(updated[0]!.steps.ocr).toBe(false)
  })
})
