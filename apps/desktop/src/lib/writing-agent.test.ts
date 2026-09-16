import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { WritingAgentStore, isResolution } from './writing-agent'

/**
 * The agent panel's state (plan-editor.md §14).
 *
 * What is asserted here is the half of §14.2 that lives on this side: the
 * decision to apply comes back from the write that made it, and a failure is
 * never read as permission.
 */

const mockInvoke = vi.mocked(invoke)

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('the capability matrix', () => {
  it('asks with the answers the caller supplied and holds what came back', async () => {
    const actions = [
      { id: 'improve_clarity', requires: 'chat', available: true, unavailable_reason: '' },
    ]
    mockInvoke.mockResolvedValue(actions as never)
    const store = new WritingAgentStore()

    await store.loadActions(true, false)

    expect(mockInvoke).toHaveBeenCalledWith('writing_agent_actions', {
      hasChat: true,
      hasRetrieval: false,
    })
    expect(store.snapshot.actions).toEqual(actions)
  })

  /**
   * A matrix that could not be read must not be read as "everything works".
   * Offering an action that will fail when used is worse than offering none —
   * by then the writer has chosen a passage.
   */
  it('offers nothing when the matrix cannot be read', async () => {
    mockInvoke.mockRejectedValue(new Error('the command is not registered'))
    const store = new WritingAgentStore()

    await store.loadActions(true, true)

    expect(store.snapshot.actions).toEqual([])
    expect(store.snapshot.error).toContain('not registered')
  })
})

describe('resolving a suggestion', () => {
  it('reports that the text should be applied, once', async () => {
    mockInvoke.mockResolvedValue({
      apply: true,
      status: 'accepted',
      suggested_text: 'el texto propuesto',
    } as never)
    const store = new WritingAgentStore()

    const out = await store.resolve('s1', 'accepted', 'hash')

    expect(isResolution(out) && out.apply).toBe(true)
    expect(mockInvoke).toHaveBeenCalledWith('writing_agent_resolve', {
      id: 's1',
      status: 'accepted',
      currentContentHash: 'hash',
    })
  })

  /** The backend owns the decision; a second acceptance applies nothing. */
  it('passes on a refusal to apply rather than deciding for itself', async () => {
    mockInvoke.mockResolvedValue({
      apply: false,
      status: 'accepted',
      suggested_text: null,
    } as never)
    const store = new WritingAgentStore()

    const out = await store.resolve('s1', 'accepted', 'hash')

    expect(isResolution(out) && out.apply).toBe(false)
    expect(isResolution(out) && out.suggested_text).toBeNull()
  })

  /**
   * A target that moved and a suggestion already resolved need different words
   * on screen, so the code travels rather than only the message.
   */
  it('carries the code so the panel can say which failure this was', async () => {
    mockInvoke.mockRejectedValue({
      code: 'suggestion_target_changed',
      message: 'the text has changed since',
    })
    const store = new WritingAgentStore()

    const out = await store.resolve('s1', 'accepted', 'otro-hash')

    expect(isResolution(out)).toBe(false)
    expect(out).toMatchObject({ code: 'suggestion_target_changed' })
  })

  /** A failure is never permission: nothing is applied and the panel is idle. */
  it('is not busy after a failure, and applied nothing', async () => {
    mockInvoke.mockRejectedValue({ code: 'x', message: 'y' })
    const store = new WritingAgentStore()

    const out = await store.resolve('s1', 'accepted', 'hash')

    expect(store.snapshot.busy).toBe(false)
    expect(isResolution(out)).toBe(false)
  })

  it('takes a resolved suggestion off the pending list', async () => {
    mockInvoke.mockResolvedValueOnce([
      { id: 's1' },
      { id: 's2' },
    ] as never)
    const store = new WritingAgentStore()
    await store.loadPending('d1')

    mockInvoke.mockResolvedValueOnce({
      apply: false,
      status: 'discarded',
      suggested_text: null,
    } as never)
    await store.resolve('s1', 'discarded', null)

    expect(store.snapshot.pending.map((row) => row.id)).toEqual(['s2'])
  })
})

/**
 * §14.3, §14.4: what is previewed, what is sent and what is recorded are the
 * same object, so they cannot disagree.
 */
describe('the context held for a selection', () => {
  const pieces = [
    { kind: 'selection' as const, label: 'Lo seleccionado', text: 'el parrafo' },
    { kind: 'corpus' as const, label: 'Un fragmento', text: 'evidencia', sourceId: 'as1' },
  ]

  it('is assembled once and answers for itself', () => {
    const store = new WritingAgentStore()

    store.prepare(pieces)

    expect(store.snapshot.context?.pieces).toHaveLength(2)
    expect(store.sent().map((r) => r.kind)).toEqual(['selection', 'corpus'])
    expect(store.evidence()).toMatchObject({ corpus: ['as1'], consultedText: true })
  })

  it('has nothing to report before a selection was prepared', () => {
    const store = new WritingAgentStore()

    expect(store.evidence()).toBeNull()
    expect(store.sent()).toEqual([])
  })
})
