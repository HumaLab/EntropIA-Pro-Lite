import { writingSchema, type CanonicalDocument } from './document-contract'

/**
 * The document's heading structure (plan-editor.md §6.1).
 *
 * §6.1 is explicit that the outline must be *derived* from the document, never
 * kept as an independent copy that can drift out of sync. So this is a pure
 * function over the canonical JSON, recomputed from it, with no state of its
 * own.
 *
 * `position` is the ProseMirror document position of the heading node, which is
 * what the editor needs to scroll to it. It is computed the way ProseMirror
 * counts: a document position advances by one for each node boundary plus the
 * size of its content.
 */

export interface OutlineEntry {
  /** Heading level, 1 through 4. */
  level: number
  /** The heading's plain text. Empty when the heading has not been typed yet. */
  text: string
  /** ProseMirror position of the heading node, for scrolling to it. */
  position: number
  /** Stable within one computation; used as a keyed-each identity. */
  index: number
  /**
   * Which top-level child of the document this heading is.
   *
   * Not the same as `index`, which counts headings only. Section operations
   * address the document's children, so carrying it here saves every caller
   * from rediscovering it — and from getting it wrong when the manuscript
   * opens with a paragraph before its first heading.
   */
  childIndex: number
}

/**
 * Walks the top level of the document collecting headings.
 *
 * Positions come from ProseMirror, not from arithmetic of our own: a leaf node
 * counts as one and a block as two plus its content, and an inline atom like a
 * corpus citation is a leaf. Reimplementing that rule is how an outline ends up
 * scrolling to the wrong place once someone inserts a citation.
 *
 * Only top-level headings are collected: a heading nested inside a table cell
 * or a blockquote is not a section of the manuscript, and treating it as one
 * would put phantom entries in the panel.
 */
export function outlineFromDocument(document: CanonicalDocument | null): OutlineEntry[] {
  if (!document?.doc?.content) return []

  let root
  try {
    root = writingSchema().nodeFromJSON(document.doc)
  } catch {
    // The contract keeps invalid documents away from the editor; an outline is
    // not the place to report that, so it simply has nothing to show.
    return []
  }

  const entries: OutlineEntry[] = []
  root.forEach((child, offset, childIndex) => {
    if (child.type.name !== 'heading') return
    const level = typeof child.attrs?.level === 'number' ? child.attrs.level : 1
    entries.push({
      level,
      text: child.textContent,
      // `offset` is where the node starts among its parent's children; the
      // document's own opening token puts its first child at 1.
      position: offset + 1,
      index: entries.length,
      childIndex,
    })
  })

  return entries
}

/**
 * Indentation depth for display, normalised so a document that starts at H2
 * does not render its whole outline pushed to the right.
 */
export function outlineDepth(entries: OutlineEntry[], entry: OutlineEntry): number {
  const shallowest = entries.reduce((min, e) => Math.min(min, e.level), Number.POSITIVE_INFINITY)
  return Number.isFinite(shallowest) ? entry.level - shallowest : 0
}
