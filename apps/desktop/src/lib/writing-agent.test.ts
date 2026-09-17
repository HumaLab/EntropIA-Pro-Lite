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
    mockInvoke.mockResolvedValueOnce([{ id: 's1' }, { id: 's2' }] as never)
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

describe('asking the agent', () => {
  const target = {
    documentId: 'd1',
    actionType: 'improve_clarity',
    selection: 'el parrafo',
    selectionAnchorJson: '{"nodeId":"n1"}',
    sourceRevision: 4,
    selectedContentHash: 'hash-del-parrafo',
  }

  /**
   * §14.4: the writer must be able to know what will be sent. That is only true
   * while the preview and the request are the same object, so the context the
   * request carries is the one `prepare` built and the panel is showing.
   */
  it('sends the context that was previewed, not a second copy of it', async () => {
    mockInvoke.mockResolvedValue({ id: 's1', status: 'pending' } as never)
    const store = new WritingAgentStore()

    await store.ask(
      [
        { kind: 'selection', label: 'Lo seleccionado', text: 'el parrafo' },
        { kind: 'corpus', label: 'Un fragmento', text: 'evidencia', sourceId: 'as1' },
      ],
      target
    )

    const [, payload] = mockInvoke.mock.calls[0] as [string, { input: Record<string, unknown> }]
    expect(payload.input.context).toEqual(store.snapshot.context?.pieces)
  })

  /** §14.2: the sources a proposal rests on travel with it, or it cannot be judged. */
  it('records what the proposal rests on', async () => {
    mockInvoke.mockResolvedValue({ id: 's1' } as never)
    const store = new WritingAgentStore()

    await store.ask(
      [{ kind: 'corpus', label: 'Un fragmento', text: 'evidencia', sourceId: 'as1' }],
      target
    )

    const [, payload] = mockInvoke.mock.calls[0] as [string, { input: { evidence_json: string } }]
    expect(JSON.parse(payload.input.evidence_json)).toMatchObject({
      corpus: ['as1'],
      consultedText: true,
    })
  })

  /** The answer is a pending proposal, so it belongs on the pending list. */
  it('puts the answer on the pending list without applying anything', async () => {
    const row = { id: 's1', status: 'pending', suggested_text: 'el texto propuesto' }
    mockInvoke.mockResolvedValue(row as never)
    const store = new WritingAgentStore()

    const out = await store.ask([{ kind: 'selection', label: 'L', text: 'el parrafo' }], target)

    expect(out).toEqual(row)
    expect(store.snapshot.pending).toEqual([row])
  })

  /** A failure is never a proposal: nothing lands on the list and the code travels. */
  it('leaves the pending list alone when the model could not be reached', async () => {
    mockInvoke.mockRejectedValue({ code: 'agent_no_credential', message: 'falta la credencial' })
    const store = new WritingAgentStore()

    const out = await store.ask([{ kind: 'selection', label: 'L', text: 'el parrafo' }], target)

    expect(out).toMatchObject({ code: 'agent_no_credential' })
    expect(store.snapshot.pending).toEqual([])
    expect(store.snapshot.busy).toBe(false)
  })

  /**
   * Asking about nothing would spend a request to be told the obvious, so it is
   * refused here rather than at the provider.
   */
  it('refuses to ask about an empty passage', async () => {
    const store = new WritingAgentStore()

    const out = await store.ask([], { ...target, selection: '   ' })

    expect(out).toMatchObject({ code: 'no_selection' })
    expect(mockInvoke).not.toHaveBeenCalled()
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
