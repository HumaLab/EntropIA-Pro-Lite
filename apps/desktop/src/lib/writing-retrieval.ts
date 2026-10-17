import { invoke } from '@tauri-apps/api/core'
import type { RelatedNote } from '@entropia/store'
import { getStore } from '$lib/db'
import type { ContextPiece } from './agent-context'
import { plainTextOf } from './note-text'
import { tokenize } from './text-analysis'

/**
 * Evidence for the agent's evidence actions (§14.1, §14.3): corpus passages for
 * three of them, the writer's own notes for "Relacionar con mis notas".
 *
 * # Why the passages become context pieces here
 *
 * Because a passage is only evidence once it carries its identity. §14.2 asks a
 * proposal to distinguish corpus evidence from Zotero metadata from text that
 * was actually consulted, and `evidenceOf` makes that distinction by looking
 * for a `sourceId`. A passage pasted in as prose would read to the model as
 * something the writer said, and would be recorded as nothing at all.
 */

export interface RetrievedPassage {
  asset_id: string
  item_id: string
  item_title: string
  collection_id: string
  collection_name: string
  text: string
  start_char: number
}

/** The actions of §14.1 whose whole purpose is evidence from the corpus. */
export const EVIDENCE_ACTIONS = [
  'find_evidence',
  'find_counter_evidence',
  'find_counterexamples',
] as const

export function needsEvidence(action: string): boolean {
  return (EVIDENCE_ACTIONS as readonly string[]).includes(action)
}

/**
 * The action that relates a passage to the writer's notes. Its evidence is the
 * notes alone: given corpus passages instead, the model reads OCR as notes and
 * relates the passage to something the writer never wrote.
 */
export const NOTES_ACTION = 'recall_notes'

export function needsNotes(action: string): boolean {
  return action === NOTES_ACTION
}

/**
 * Passages of the corpus related to one of the manuscript's.
 *
 * Returns an empty list rather than throwing. §11.3's rule — that what cannot
 * be fetched must not stop the writing — holds here too: an action that found
 * no evidence should say so through the proposal it produces, not by failing
 * in front of someone mid-sentence.
 */
export async function retrievePassages(passage: string, limit = 5): Promise<RetrievedPassage[]> {
  if (!passage.trim()) return []
  try {
    return await invoke<RetrievedPassage[]>('writing_corpus_retrieve', { passage, limit })
  } catch {
    return []
  }
}

/**
 * The passages as context, labelled by where they came from.
 *
 * The label names the source a reader would recognise — the item, and the
 * collection holding it — because the preview shows these to the writer before
 * they judge a proposal, and `as1f3c…` tells them nothing.
 */
export function evidencePieces(passages: RetrievedPassage[]): ContextPiece[] {
  return passages.map((passage) => ({
    kind: 'corpus' as const,
    label: passage.collection_name
      ? `${passage.item_title} · ${passage.collection_name}`
      : passage.item_title,
    text: passage.text,
    // What makes this evidence rather than prose: `evidenceOf` records the
    // asset, and the writer can open it.
    sourceId: passage.asset_id,
  }))
}

/**
 * The writer's notes that share words with a passage, most shared first.
 *
 * By words only: stopwords are dropped so that "de" or "la" relate nothing, and
 * each remaining word counts on its own, typos forgiven. A note that says the
 * same thing in other words is not found — that needs embeddings over notes.
 *
 * A passage of four or more meaningful words asks for two in common: across a
 * long selection almost every note shares some word, and a note related by
 * coincidence is exactly what the model would then build a relation on.
 *
 * Unlike corpus retrieval this throws: "no related notes" is an answer the
 * writer acts on, and a failed read must not pass for it.
 */
export async function retrieveNotes(
  passage: string,
  collectionIds?: string[],
  limit = 5
): Promise<RelatedNote[]> {
  const terms = [...new Set(tokenize(passage))]
  if (terms.length === 0) return []
  return getStore().notes.related(terms, {
    collectionIds,
    limit,
    minMatchedTerms: terms.length >= LONG_PASSAGE_TERMS ? 2 : 1,
  })
}

/** From this many meaningful words on, one shared word is not a relation. */
const LONG_PASSAGE_TERMS = 4

/** The notes as context, each carrying its id so the proposal records it. */
export function notePieces(notes: RelatedNote[]): ContextPiece[] {
  return notes.map((note) => ({
    kind: 'note' as const,
    label: note.itemTitle,
    // Notes are stored as HTML; the model is given the words.
    text: plainTextOf(note.content),
    sourceId: note.id,
  }))
}
