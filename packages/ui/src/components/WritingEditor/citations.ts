import type { Node } from '@tiptap/pm/model'
import { writingSchema, type CanonicalDocument } from './document-contract'

/**
 * The document-citation projection (plan-editor.md §9.4, §10.1).
 *
 * # Why this is derived and not maintained
 *
 * `save_document` deletes the document's citation rows and reinserts them
 * inside the same transaction that writes the content (§8.4). That makes a
 * *derived* projection the only design that cannot drift: the manuscript is the
 * single source of truth, and copying, moving, deleting, undo, redo and
 * restoring a version all stay consistent for free, because none of them can
 * change the projection without changing the document it comes from.
 *
 * The alternative — keeping a parallel list and patching it on each operation —
 * is the one the outline was explicitly forbidden from using in §6.1, for the
 * same reason: every operation someone forgets to patch is a silent
 * desynchronisation.
 *
 * Pure over a ProseMirror node: no editor, no store, no database.
 */

/** One row of `writing_document_citations`, in the shape Rust deserialises. */
export interface DocumentCitationRow {
  id: string
  citation_node_id: string
  collection_id: string | null
  item_id: string | null
  asset_id: string | null
  page_number: number | null
  start_char: number | null
  end_char: number | null
  quoted_text: string | null
  source_text_hash: string | null
  metadata_snapshot_json: string
}

export const CITATION_NODE = 'documentCitation'
export const NOTE_LINK_NODE = 'noteLink'
export const ZOTERO_CITATION_NODE = 'zoteroCitation'

/**
 * Node types whose identity must be unique within a manuscript.
 *
 * Both anchor something outside the document and are referred to by that
 * identity — a citation by its projection row, a note link by the provenance
 * that records where it came from. A paste duplicates it in either case.
 */
export const ANCHORED_NODES: Record<string, string> = {
  [CITATION_NODE]: 'citationNodeId',
  [NOTE_LINK_NODE]: 'noteLinkNodeId',
  [ZOTERO_CITATION_NODE]: 'citationNodeId',
}

function str(value: unknown): string | null {
  return typeof value === 'string' && value.length > 0 ? value : null
}

function num(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function snapshot(value: unknown): string {
  if (typeof value === 'string') return value
  if (value && typeof value === 'object') return JSON.stringify(value)
  // The column is NOT NULL: an absent snapshot is an empty one, never null.
  return '{}'
}

/** Every citation node in the manuscript, wherever it sits. */
function citationNodes(doc: Node): Node[] {
  const found: Node[] = []
  doc.descendants((node) => {
    if (node.type.name === CITATION_NODE) found.push(node)
    return true
  })
  return found
}

/** Every node whose identity has to be unique, whatever kind it is. */
function anchoredNodes(doc: Node): { node: Node; attribute: string }[] {
  const found: { node: Node; attribute: string }[] = []
  doc.descendants((node) => {
    const attribute = ANCHORED_NODES[node.type.name]
    if (attribute) found.push({ node, attribute })
    return true
  })
  return found
}

/**
 * The projection for this document.
 *
 * A citation with no `citationNodeId` is skipped rather than given one here:
 * this function is pure and must not mint identity, and the row is keyed by
 * that id. `UniqueCitationIds` is what keeps such a node from existing.
 */
export function citationsFromDocument(doc: Node): DocumentCitationRow[] {
  const rows: DocumentCitationRow[] = []

  for (const node of citationNodes(doc)) {
    const nodeId = str(node.attrs.citationNodeId)
    if (!nodeId) continue
    rows.push({
      // The rows are replaced wholesale on every save, so the row id has no
      // referent outside the document. Deriving it from the node id is what
      // makes saving the same manuscript twice produce the same projection.
      id: nodeId,
      citation_node_id: nodeId,
      collection_id: str(node.attrs.collectionId),
      item_id: str(node.attrs.itemId),
      asset_id: str(node.attrs.assetId),
      page_number: num(node.attrs.pageNumber),
      start_char: num(node.attrs.startChar),
      end_char: num(node.attrs.endChar),
      quoted_text: str(node.attrs.quotedText),
      source_text_hash: str(node.attrs.sourceTextHash),
      metadata_snapshot_json: snapshot(node.attrs.metadataSnapshot),
    })
  }

  return rows
}

/**
 * Identities that appear on more than one citation.
 *
 * Pasting is the one operation the manuscript cannot settle by itself: a copied
 * citation arrives carrying the identity of the one it came from, and the
 * projection is unique per `(document_id, citation_node_id)`. Left alone, the
 * second copy would collide with the first or silently replace it.
 *
 * Reported once per duplicated id — the first occurrence keeps it, and only the
 * later ones need a new one.
 */
export function duplicatedCitationIds(doc: Node): string[] {
  const seen = new Set<string>()
  const duplicated = new Set<string>()

  for (const { node, attribute } of anchoredNodes(doc)) {
    const nodeId = str(node.attrs[attribute])
    if (!nodeId) continue
    if (seen.has(nodeId)) duplicated.add(nodeId)
    else seen.add(nodeId)
  }

  return [...duplicated]
}

/**
 * The projection for a stored manuscript, the form the autosave loop holds.
 *
 * The store keeps canonical JSON, not a ProseMirror node, so this parses it
 * through the one schema the contract validates against. A document that will
 * not parse projects nothing rather than throwing: the save path is not where a
 * malformed manuscript should first be discovered, and `parseCanonical` has
 * already refused it long before this runs.
 */
export function citationProjection(document: CanonicalDocument | null): DocumentCitationRow[] {
  if (!document?.doc) return []
  try {
    return citationsFromDocument(writingSchema().nodeFromJSON(document.doc))
  } catch {
    return []
  }
}
