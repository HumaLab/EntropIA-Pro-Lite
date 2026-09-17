import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { evidenceOf, buildContext } from './agent-context'
import {
  EVIDENCE_ACTIONS,
  evidencePieces,
  needsEvidence,
  retrievePassages,
} from './writing-retrieval'

/**
 * Corpus evidence for the agent (plan-editor.md §14.1, §14.2).
 *
 * The assertion that carries the most weight is the last one: a passage is only
 * evidence once it carries its identity. Without a `sourceId` the same words
 * reach the model as something the writer said, and `evidenceOf` records the
 * proposal as having consulted nothing — a suggestion that looks sourced and is
 * recorded as prose.
 */

const mockInvoke = vi.mocked(invoke)

const passage = (assetId: string, text: string) => ({
  asset_id: assetId,
  item_id: 'it1',
  item_title: 'Acta del gremio',
  collection_id: 'col1',
  collection_name: 'Movimiento obrero',
  text,
  start_char: 40,
})

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('which actions need evidence', () => {
  it('names the four of §14.1 and nothing else', () => {
    expect(needsEvidence('find_evidence')).toBe(true)
    expect(needsEvidence('recall_notes')).toBe(true)
    expect(needsEvidence('improve_clarity')).toBe(false)
    expect(EVIDENCE_ACTIONS).toHaveLength(4)
  })
})

describe('asking the corpus', () => {
  it('asks about the passage and hands back what it found', async () => {
    mockInvoke.mockResolvedValue([passage('as1', 'los obreros declararon la huelga')] as never)

    const found = await retrievePassages('el pasaje', 5)

    expect(mockInvoke).toHaveBeenCalledWith('writing_corpus_retrieve', {
      passage: 'el pasaje',
      limit: 5,
    })
    expect(found).toHaveLength(1)
  })

  /**
   * §11.3's rule applies here too: what cannot be fetched must not stop the
   * writing. An action that found no evidence says so through the proposal it
   * produces, never by failing in front of someone mid-sentence.
   */
  it('reports nothing found rather than failing', async () => {
    mockInvoke.mockRejectedValue(new Error('the index is not built'))

    expect(await retrievePassages('el pasaje')).toEqual([])
  })

  it('does not ask about an empty passage', async () => {
    expect(await retrievePassages('   ')).toEqual([])
    expect(mockInvoke).not.toHaveBeenCalled()
  })
})

describe('the passages as context', () => {
  /** `as1f3c…` tells the writer nothing; the item and its collection do. */
  it('labels a passage by the source a reader would recognise', () => {
    const [piece] = evidencePieces([passage('as1', 'los obreros')])

    expect(piece!.label).toBe('Acta del gremio · Movimiento obrero')
    expect(piece!.kind).toBe('corpus')
  })

  it('falls back to the item alone when there is no collection name', () => {
    const [piece] = evidencePieces([{ ...passage('as1', 'x'), collection_name: '' }])

    expect(piece!.label).toBe('Acta del gremio')
  })

  /**
   * The one that matters. Without the asset id the same words reach the model
   * as something the writer said, and the proposal is recorded as having
   * consulted nothing — sourced on screen, prose in the record.
   */
  it('makes the passage count as evidence, not as prose', () => {
    const context = buildContext(evidencePieces([passage('as1', 'los obreros')]))

    expect(evidenceOf(context)).toMatchObject({
      corpus: ['as1'],
      consultedText: true,
    })
  })

  /**
   * Two passages of the same asset are two pieces of evidence: the context
   * builder deduplicates by identity, so they must not collapse into one.
   */
  it('keeps two passages of one source apart', () => {
    const pieces = evidencePieces([
      { ...passage('as1', 'primero'), start_char: 0 },
      { ...passage('as1', 'segundo'), start_char: 900 },
    ])
    const context = buildContext(pieces)

    expect(context.pieces).toHaveLength(1)
    expect(context.omitted[0]?.reason).toBe('duplicate')
  })
})
