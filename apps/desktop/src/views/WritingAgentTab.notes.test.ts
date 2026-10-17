import { fireEvent, render, screen } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WritingAgentTab from './WritingAgentTab.svelte'

/**
 * "Relacionar con mis notas" asks the model only about notes that exist.
 *
 * Given nothing to relate the passage to, a model invents the relation — which
 * is how this action once answered with OCR passed off as notes. So with no
 * related note the request never leaves, and a note that could not be read is
 * reported as a failure rather than as "no notes".
 */

const { related } = vi.hoisted(() => ({ related: vi.fn() }))
vi.mock('$lib/db', () => ({ getStore: () => ({ notes: { related } }) }))

const mockInvoke = vi.mocked(invoke)

function backend() {
  mockInvoke.mockImplementation(async (command: string) => {
    switch (command) {
      case 'writing_agent_actions':
        return [{ id: 'recall_notes', requires: 'chat', available: true, unavailable_reason: '' }]
      case 'writing_agent_pending':
        return []
      case 'writing_agent_ask':
        return {
          id: 's1',
          document_id: 'doc1',
          selection_anchor_json: null,
          source_revision: 1,
          selected_content_hash: 'h',
          action_type: 'recall_notes',
          original_text: 'los molineros',
          suggested_text: 'propuesta',
          rationale: null,
          evidence_json: '{}',
          status: 'pending',
          provider: 'p',
          model: 'm',
          created_at: 0,
          resolved_at: null,
        }
      default:
        throw new Error(`unexpected command ${command}`)
    }
  })
}

const asked = () => mockInvoke.mock.calls.filter(([command]) => command === 'writing_agent_ask')

async function pressNotes() {
  render(WritingAgentTab, {
    props: { documentId: 'doc1', hasChat: true, selection: () => 'los molineros' },
  })
  await fireEvent.click(await screen.findByRole('button', { name: 'Notas' }))
}

beforeEach(() => {
  mockInvoke.mockReset()
  related.mockReset()
  backend()
})

describe('relating a passage to the writer’s notes', () => {
  it('does not ask the model when no note is related, and says why', async () => {
    related.mockResolvedValue([])

    await pressNotes()

    expect(await screen.findByText(/Ninguna de tus notas comparte palabras/)).toBeInTheDocument()
    expect(asked()).toEqual([])
    // Searching is over: the actions can be pressed again.
    expect(screen.getByRole('button', { name: 'Notas' })).toBeEnabled()
  })

  it('reports a failed read as a failure, not as "no notes", and asks nothing', async () => {
    related.mockRejectedValue(new Error('db locked'))

    await pressNotes()

    expect(await screen.findByText(/db locked/)).toBeInTheDocument()
    expect(screen.queryByText(/Ninguna de tus notas/)).toBeNull()
    expect(asked()).toEqual([])
  })

  it('sends the related notes as notes, and no corpus passage', async () => {
    related.mockResolvedValue([
      {
        id: 'n1',
        itemId: 'it1',
        itemTitle: 'Acta del gremio',
        collectionId: 'col1',
        assetId: null,
        content: '<p>los molineros pararon</p>',
        createdAt: 0,
        updatedAt: 0,
        matchedTerms: 1,
      },
    ])

    await pressNotes()

    await vi.waitFor(() => expect(asked()).toHaveLength(1))
    const input = (asked()[0]![1] as { input: { context: { kind: string; text: string }[] } })
      .input
    expect(input.context.map((piece) => piece.kind)).toEqual(['selection', 'note'])
    expect(input.context[1]!.text).toBe('los molineros pararon')
    expect(mockInvoke).not.toHaveBeenCalledWith('writing_corpus_retrieve', expect.anything())
  })
})
