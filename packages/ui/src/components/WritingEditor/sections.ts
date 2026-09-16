import type { Node } from '@tiptap/pm/model'

/**
 * What a section is, and where it ends (plan-editor.md §6.1).
 *
 * A section is a heading plus everything that follows it until the next heading
 * of the same level or higher. That definition is the whole feature: get it
 * wrong and "delete this section" takes the next chapter with it, or leaves its
 * body orphaned under the heading above.
 *
 * Positions come from ProseMirror's own accounting — `doc.forEach` hands each
 * child its offset — rather than from arithmetic of our own. The outline
 * already learned that lesson once: an inline atom counts as one, not two, and
 * reimplementing the rule is how a range ends up off by a citation.
 *
 * Pure over a ProseMirror node. `childIndex` addresses a top-level child, not a
 * document position, so no caller has to know the difference.
 */

export interface SectionRange {
  /** Document position where the section starts. */
  from: number
  /** Document position where it ends. */
  to: number
}

/**
 * Blocks that are the document's, not any section's.
 *
 * The footnotes container belongs at the very end of the manuscript regardless
 * of which section happens to be last, so deleting a chapter must not take the
 * notes with it.
 */
const DOCUMENT_OWNED = new Set(['footnotes'])

interface TopLevelChild {
  node: Node
  offset: number
  index: number
}

function topLevel(doc: Node): TopLevelChild[] {
  const children: TopLevelChild[] = []
  doc.forEach((node, offset, index) => children.push({ node, offset, index }))
  return children
}

function headingLevel(node: Node): number | null {
  if (node.type.name !== 'heading') return null
  return typeof node.attrs.level === 'number' ? node.attrs.level : 1
}

/**
 * The range the section at `childIndex` covers, or null when that child is not
 * a heading — a paragraph before the first heading belongs to no section.
 */
export function sectionRange(doc: Node, childIndex: number): SectionRange | null {
  const children = topLevel(doc)
  const start = children[childIndex]
  if (!start) return null
  const level = headingLevel(start.node)
  if (level === null) return null

  let to = start.offset + start.node.nodeSize
  for (const child of children.slice(childIndex + 1)) {
    if (DOCUMENT_OWNED.has(child.node.type.name)) break
    const other = headingLevel(child.node)
    // A heading at the same level or shallower opens the next section; a deeper
    // one is a subsection and stays inside this range.
    if (other !== null && other <= level) break
    to = child.offset + child.node.nodeSize
  }

  return { from: start.offset, to }
}

/**
 * The child index of the section next to this one at the same level, or null
 * when there is none in that direction.
 *
 * Subsections are skipped rather than stepped into: the section after "1" is
 * "2", not "1.a". And the search stops at a shallower heading, so a subsection
 * never finds a sibling outside the chapter it belongs to — moving "1.a" down
 * must not launch it into chapter 2.
 */
export function siblingSection(
  doc: Node,
  childIndex: number,
  direction: 1 | -1
): number | null {
  const children = topLevel(doc)
  const start = children[childIndex]
  if (!start) return null
  const level = headingLevel(start.node)
  if (level === null) return null

  for (
    let index = childIndex + direction;
    index >= 0 && index < children.length;
    index += direction
  ) {
    const child = children[index]
    if (!child || DOCUMENT_OWNED.has(child.node.type.name)) break
    const other = headingLevel(child.node)
    if (other === null) continue
    if (other === level) return index
    // A shallower heading is the boundary of the parent section. Whatever lies
    // beyond it is somebody else's subsection.
    if (other < level) break
  }

  return null
}
