/**
 * Deciding what the agent is told (plan-editor.md §14.3, §14.4).
 *
 * # Never the whole document
 *
 * §14.3 opens by ruling that out. A manuscript is the writer's, and sending all
 * of it to a provider because an action needed one paragraph is both a waste
 * and a disclosure nobody asked for. The context is assembled from named
 * pieces, each of which the writer can see before it leaves.
 *
 * # Why the preview is the point, not a courtesy
 *
 * §14.4 requires that the writer be able to know which portion of the article
 * and which sources will be sent. That is only true if what is shown and what
 * is sent are the same object — so the preview is not rendered from a separate
 * description, it is the assembled context itself, and the record of what was
 * sent is the same list again.
 *
 * # Limits are part of the contract
 *
 * A budget that silently drops the tail of the context produces an answer the
 * writer cannot account for. So a piece that does not fit is *reported* as
 * omitted rather than quietly left out, and the caller can show that.
 */

/** Where a piece of context came from, and what it therefore means. */
export type ContextKind =
  | 'instruction'
  | 'selection'
  | 'heading'
  | 'surrounding'
  | 'outline'
  | 'corpus'
  | 'citation'
  | 'note'
  | 'zotero'

/** One named piece, before any decision about whether it fits. */
export interface ContextPiece {
  kind: ContextKind
  /** Shown to the writer, so it says what the piece *is*, not where it is from. */
  label: string
  text: string
  /**
   * Identity, when the piece has one — an asset, a note, a Zotero key.
   *
   * Two pieces with the same identity are the same piece however they were
   * reached, which is what makes deduplication mean something: a fragment
   * pulled in both as corpus evidence and as a linked citation is one fragment.
   */
  sourceId?: string | null
}

export interface ContextBudget {
  /** Characters. A blunt measure, but one the writer can see the effect of. */
  maxChars: number
  /** How many pieces may be sent, whatever their size. */
  maxPieces: number
}

export const DEFAULT_BUDGET: ContextBudget = { maxChars: 12_000, maxPieces: 24 }

export interface BuiltContext {
  /** Exactly what will be sent, in order. The preview shows this. */
  pieces: ContextPiece[]
  /** Pieces left out, and why — never silently dropped. */
  omitted: { piece: ContextPiece; reason: 'duplicate' | 'over_budget' }[]
  chars: number
}

/**
 * The order pieces are kept in when the budget bites.
 *
 * The selection and the instruction are what the action *is*: without them
 * there is no question to answer, so they go first and are never the ones
 * dropped. Evidence is next, because an answer without it is the kind the
 * writer cannot check. The structural summary is last precisely because §14.3
 * calls it optional — "si resulta necesario".
 */
const PRIORITY: ContextKind[] = [
  'instruction',
  'selection',
  'heading',
  'surrounding',
  'citation',
  'corpus',
  'note',
  'zotero',
  'outline',
]

function rank(kind: ContextKind): number {
  const at = PRIORITY.indexOf(kind)
  return at === -1 ? PRIORITY.length : at
}

/** What identifies a piece for the purpose of not sending it twice. */
function identityOf(piece: ContextPiece): string {
  return piece.sourceId ? `${piece.kind}:${piece.sourceId}` : `${piece.kind}:${piece.text}`
}

/**
 * Assembles the context, within its budget, reporting everything left out.
 *
 * Deduplication comes before the budget on purpose: dropping a real piece to
 * make room for a copy of another would be the worst possible use of the space.
 */
export function buildContext(
  pieces: ContextPiece[],
  budget: ContextBudget = DEFAULT_BUDGET
): BuiltContext {
  const omitted: BuiltContext['omitted'] = []
  const seen = new Set<string>()
  const unique: ContextPiece[] = []

  for (const piece of pieces) {
    if (!piece.text.trim()) continue
    const identity = identityOf(piece)
    if (seen.has(identity)) {
      omitted.push({ piece, reason: 'duplicate' })
      continue
    }
    seen.add(identity)
    unique.push(piece)
  }

  // Ordered by what the action needs most, so what the budget removes is what
  // matters least rather than whatever happened to arrive last.
  const ordered = [...unique].sort((a, b) => rank(a.kind) - rank(b.kind))

  const kept: ContextPiece[] = []
  let chars = 0
  for (const piece of ordered) {
    const cost = piece.text.length
    if (kept.length >= budget.maxPieces || chars + cost > budget.maxChars) {
      omitted.push({ piece, reason: 'over_budget' })
      continue
    }
    kept.push(piece)
    chars += cost
  }

  return { pieces: kept, omitted, chars }
}

/**
 * The record of what was sent (§14.3).
 *
 * Built from the same object the preview showed and the request carried, so
 * there is no way for the three to disagree. A record assembled separately
 * would be a description of what someone believed was sent.
 */
export function sentRecord(context: BuiltContext): {
  kind: ContextKind
  label: string
  sourceId: string | null
  chars: number
}[] {
  return context.pieces.map((piece) => ({
    kind: piece.kind,
    label: piece.label,
    sourceId: piece.sourceId ?? null,
    chars: piece.text.length,
  }))
}

/**
 * Whether the agent was given anything it could reason *from*, as opposed to
 * reason *about* (§14.2).
 *
 * §14.2 asks a suggestion to distinguish corpus evidence, Zotero metadata and
 * text that was actually consulted. This is the first half of that: a proposal
 * built with no evidence at all is the model's own prose, and saying so is more
 * honest than a sources list that is empty for unexplained reasons.
 */
export function evidenceOf(context: BuiltContext): {
  corpus: string[]
  zotero: string[]
  notes: string[]
  consultedText: boolean
} {
  const ids = (kind: ContextKind) =>
    context.pieces
      .filter((piece) => piece.kind === kind && piece.sourceId)
      .map((piece) => piece.sourceId as string)

  return {
    corpus: ids('corpus'),
    zotero: ids('zotero'),
    notes: ids('note'),
    // Zotero metadata is not the work. A proposal that saw only a title and an
    // author has not read anything, and §14.2 draws that line explicitly.
    consultedText: context.pieces.some(
      (piece) => piece.kind === 'corpus' || piece.kind === 'citation' || piece.kind === 'note'
    ),
  }
}
