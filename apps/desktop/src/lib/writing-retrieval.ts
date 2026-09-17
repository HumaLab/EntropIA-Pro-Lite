import { invoke } from '@tauri-apps/api/core'
import type { ContextPiece } from './agent-context'

/**
 * Corpus evidence for the agent's four evidence actions (§14.1, §14.3).
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
  'recall_notes',
] as const

export function needsEvidence(action: string): boolean {
  return (EVIDENCE_ACTIONS as readonly string[]).includes(action)
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
