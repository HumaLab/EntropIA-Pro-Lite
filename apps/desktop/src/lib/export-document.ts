import type { CitationRepresentation } from './export-fidelity'

/**
 * What the three exporters share (plan-editor.md §17).
 *
 * Small on purpose. The formats differ enough that a common renderer would be
 * a tangle of conditionals; what they genuinely share is the shape of the
 * document, where the footnote bodies live, and what a Zotero citation says —
 * and that last one is worth sharing hard, because §17.3 requires the
 * citations and the bibliography to read as the CSL style dictates in every
 * format.
 */

/** A node of the canonical manuscript, read structurally rather than parsed. */
export interface Node {
  type?: string
  text?: string
  attrs?: Record<string, unknown>
  marks?: { type?: string; attrs?: Record<string, unknown> }[]
  content?: Node[]
}

/** Everything the exporters need that is not in the document itself. */
export interface ExportContext {
  title: string
  /** Which of §17.2's representations the writer chose for corpus citations. */
  citations: CitationRepresentation
  /**
   * The CSL rendering of each Zotero citation, by `citationNodeId`.
   *
   * Passed in rather than read off the node: `renderedText` on the node is a
   * cache so the page is not blank while the engine answers, and §11.5 is
   * explicit that it is never the source of truth. An export that trusted it
   * would put yesterday's citation style into today's document.
   */
  zotero: Record<string, string>
  /** The bibliography, already rendered by the same style. */
  bibliography: string[]
  /** What the bibliography section is called, in the interface's language. */
  bibliographyHeading: string
}

export function childrenOf(node: Node | undefined): Node[] {
  return Array.isArray(node?.content) ? node.content : []
}

/** All the text under a node, marks and structure ignored. */
export function textOf(node: Node): string {
  if (typeof node.text === 'string') return node.text
  return childrenOf(node).map(textOf).join('')
}

/**
 * What a Zotero citation reads as.
 *
 * The freshly rendered string when there is one; the node's cached rendering
 * only as a last resort, and the plain marker when there is not even that. A
 * citation that exports as nothing is worse than one that exports as `[cita]`:
 * the first disappears, the second can be found.
 */
export function zoteroTextOf(node: Node, context: ExportContext): string {
  const id = typeof node.attrs?.citationNodeId === 'string' ? node.attrs.citationNodeId : ''
  const fresh = context.zotero[id]
  if (fresh) return fresh
  const cached = node.attrs?.renderedText
  return typeof cached === 'string' && cached ? cached : '[cita]'
}

/**
 * The body of each footnote, keyed by the `data-id` its references carry.
 *
 * `tiptap-footnotes` keeps the bodies in a `footnotes` block at the end of the
 * document and the markers inline, joined by that id. An exporter that walked
 * the document in order would emit the bodies after everything else and in the
 * wrong order; this lets each marker carry its own text.
 *
 * `render` turns a footnote's blocks into whatever the format wants, so the
 * same index serves Markdown, HTML and DOCX.
 */
export function footnoteBodies<T>(doc: Node, render: (nodes: Node[]) => T): Map<string, T> {
  const bodies = new Map<string, T>()

  const walk = (node: Node) => {
    if (node.type === 'footnote') {
      const id = typeof node.attrs?.['data-id'] === 'string' ? node.attrs['data-id'] : ''
      // A footnote holds `paragraph+`, so its body is the inline content of its
      // blocks rather than the blocks themselves — a note is one run of prose.
      bodies.set(id, render(childrenOf(node).flatMap((block) => childrenOf(block))))
      return
    }
    for (const child of childrenOf(node)) walk(child)
  }

  walk(doc)
  return bodies
}

/**
 * Whether the export should add a title heading of its own.
 *
 * A manuscript that already opens with a level-1 heading has its title; adding
 * the document's name above it prints it twice, which every writer notices and
 * nobody wants. So the heading is added only when the manuscript does not
 * already begin with one.
 */
export function needsTitleHeading(doc: Node, title: string): boolean {
  if (!title.trim()) return false
  const first = childrenOf(doc).find((node) => node.type !== 'footnotes')
  return !(first?.type === 'heading' && first.attrs?.level === 1)
}
