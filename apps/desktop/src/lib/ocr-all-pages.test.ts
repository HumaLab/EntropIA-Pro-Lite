import { describe, expect, it } from 'vitest'
import { planOcrAllPages, type PageOcrProbe } from './ocr-all-pages'

/**
 * "OCR every page" in the Texto tab: which pages of the open document go to
 * the queue. The reported gap was that a PDF could only be read one page at a
 * time from its own view, and the whole document needed Lotes.
 */

const page = (id: string, type = 'image') => ({ id, path: `/a/${id}.png`, type })

function probe(state: {
  text?: string[]
  edited?: string[]
  queued?: string[]
  failing?: string[]
}): PageOcrProbe {
  const has = (list: string[] | undefined, id: string) => list?.includes(id) ?? false
  const fail = (id: string) => {
    if (has(state.failing, id)) throw new Error('unreadable')
  }
  return {
    hasText: async (id) => (fail(id), has(state.text, id)),
    editedByHand: async (id) => (fail(id), has(state.edited, id)),
    inQueue: async (id) => (fail(id), has(state.queued, id)),
  }
}

describe('planning OCR for every page of a document', () => {
  it('queues the pages without text, in order', async () => {
    const plan = await planOcrAllPages(
      [page('p1'), page('p2'), page('p3')],
      probe({ text: ['p2'] })
    )

    expect(plan.queue.map((asset) => asset.id)).toEqual(['p1', 'p3'])
    expect(plan.skipped).toBe(1)
  })

  it('leaves out pages corrected by hand, already queued, or audio', async () => {
    const plan = await planOcrAllPages(
      [page('p1'), page('p2'), page('p3'), page('a1', 'audio'), page('p4')],
      probe({ text: ['p1'], edited: ['p1', 'p2'], queued: ['p3'] })
    )

    expect(plan.queue.map((asset) => asset.id)).toEqual(['p4'])
    expect(plan.skipped).toBe(3)
  })

  it('leaves out a page it cannot read the state of, rather than overwrite it', async () => {
    const plan = await planOcrAllPages([page('p1'), page('p2')], probe({ failing: ['p1'] }))

    expect(plan.queue.map((asset) => asset.id)).toEqual(['p2'])
    expect(plan.skipped).toBe(1)
  })

  it('queues nothing when every page already has text', async () => {
    const plan = await planOcrAllPages([page('p1'), page('p2')], probe({ text: ['p1', 'p2'] }))

    expect(plan).toEqual({ queue: [], skipped: 2 })
  })
})
