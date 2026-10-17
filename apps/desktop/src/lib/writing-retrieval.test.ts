import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { RelatedNote } from '@entropia/store'
import { evidenceOf, buildContext } from './agent-context'
import {
  EVIDENCE_ACTIONS,
  evidencePieces,
  needsEvidence,
  needsNotes,
  notePieces,
  retrieveNotes,
  retrievePassages,
} from './writing-retrieval'

const { related } = vi.hoisted(() => ({ related: vi.fn() }))
vi.mock('$lib/db', () => ({ getStore: () => ({ notes: { related } }) }))

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
  related.mockReset()
})

describe('which actions need evidence', () => {
  it('sends three actions to the corpus and nothing else', () => {
    expect(needsEvidence('find_evidence')).toBe(true)
    expect(needsEvidence('improve_clarity')).toBe(false)
    expect(EVIDENCE_ACTIONS).toHaveLength(3)
  })

  /** Given OCR as "notes", the model related the passage to notes never written. */
  it('sends "relate to my notes" to the notes, never to the corpus', () => {
    expect(needsNotes('recall_notes')).toBe(true)
    expect(needsEvidence('recall_notes')).toBe(false)
    expect(needsNotes('find_evidence')).toBe(false)
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

describe('asking the notes', () => {
  /** "de", "la", "mi" would relate every note to every passage. */
  it('asks with the words that carry meaning, not the stopwords', async () => {
    related.mockResolvedValue([])

    await retrieveNotes('No soy el hijo de mi padre', ['col1'])

    expect(related).toHaveBeenCalledWith(['hijo', 'padre'], {
      collectionIds: ['col1'],
      limit: 5,
      minMatchedTerms: 1,
    })
  })

  /** Across a long passage one shared word is coincidence, not a relation. */
  it('asks a long passage for two shared words, counting each word once', async () => {
    related.mockResolvedValue([])

    await retrieveNotes('molineros molineros huelga')
    expect(related).toHaveBeenLastCalledWith(
      ['molineros', 'huelga'],
      expect.objectContaining({ minMatchedTerms: 1 })
    )

    await retrieveNotes('los molineros declararon la huelga en el puerto')
    expect(related).toHaveBeenLastCalledWith(
      ['molineros', 'declararon', 'huelga', 'puerto'],
      expect.objectContaining({ minMatchedTerms: 2 })
    )
  })

  it('does not ask when the passage has no meaningful word', async () => {
    expect(await retrieveNotes('de la que')).toEqual([])
    expect(related).not.toHaveBeenCalled()
  })

  /** "No related notes" is acted on; a failed read must not pass for it. */
  it('lets a failed read surface', async () => {
    related.mockRejectedValue(new Error('db locked'))

    await expect(retrieveNotes('los molineros')).rejects.toThrow('db locked')
  })
})

describe('the notes as context', () => {
  const note: RelatedNote = {
    id: 'n1',
    itemId: 'it1',
    itemTitle: 'Acta del gremio',
    collectionId: 'col1',
    assetId: null,
    content: '<p>los <strong>molineros</strong> pararon</p>',
    createdAt: 0,
    updatedAt: 0,
    matchedTerms: 1,
  }

  it('gives the model the words of the note and records it as a note', () => {
    const pieces = notePieces([note])

    expect(pieces[0]).toMatchObject({
      kind: 'note',
      label: 'Acta del gremio',
      text: 'los molineros pararon',
    })
    expect(evidenceOf(buildContext(pieces))).toMatchObject({
      notes: ['n1'],
      corpus: [],
      consultedText: true,
    })
  })
})
