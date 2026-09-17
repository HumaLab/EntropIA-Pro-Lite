import type { Editor } from '@tiptap/core'
import { sectionRange, siblingSection } from './sections'

/**
 * Editing the manuscript by its outline (plan-editor.md §6.1).
 *
 * Every operation here works on a whole section — the heading and the body it
 * owns — because that is what someone dragging a line in the outline believes
 * they are moving. Moving only the heading would leave its paragraphs stranded
 * under the chapter above, which looks like data loss even though nothing was
 * lost.
 *
 * Each is a single transaction, so each is a single undo.
 */

/** The new heading a fresh section starts with, plus somewhere to write. */
function sectionContent(level: number, title: string) {
  return [
    { type: 'heading', attrs: { level }, content: title ? [{ type: 'text', text: title }] : [] },
    { type: 'paragraph' },
  ]
}

/** Replaces a heading's text without disturbing the body under it. */
export function renameSection(editor: Editor, childIndex: number, title: string): boolean {
  const { doc } = editor.state
  const range = sectionRange(doc, childIndex)
  if (!range) return false
  const heading = doc.child(childIndex)

  const tr = editor.state.tr
  const from = range.from + 1
  const to = range.from + heading.nodeSize - 1
  // `insertText` with an empty string deletes the range, which is what an
  // emptied title should do: a heading with no text is legal and renders as the
  // untitled entry the outline already knows how to show.
  if (title) tr.insertText(title, from, to)
  else tr.delete(from, to)
  editor.view.dispatch(tr)
  return true
}

/**
 * Removes a section and everything it owns.
 *
 * A document must keep at least one block, so emptying it entirely leaves one
 * paragraph behind rather than an invalid document the schema would reject.
 */
export function deleteSection(editor: Editor, childIndex: number): boolean {
  const { doc } = editor.state
  const range = sectionRange(doc, childIndex)
  if (!range) return false

  const tr = editor.state.tr
  tr.delete(range.from, range.to)
  if (tr.doc.childCount === 0) {
    const paragraph = editor.state.schema.nodes.paragraph
    if (paragraph) tr.insert(0, paragraph.create())
  }
  editor.view.dispatch(tr)
  return true
}

/**
 * Swaps a section with the one beside it at the same level.
 *
 * The section is lifted out and put back on the far side of its neighbour, in
 * one transaction. Deleting first is what makes the arithmetic simple: with the
 * section gone, its neighbour has slid into the space it left, so the insertion
 * point is the neighbour's own length away from where the section started.
 */
export function moveSection(editor: Editor, childIndex: number, direction: 1 | -1): boolean {
  const { doc } = editor.state
  const range = sectionRange(doc, childIndex)
  const neighbourIndex = siblingSection(doc, childIndex, direction)
  if (!range || neighbourIndex === null) return false
  const neighbour = sectionRange(doc, neighbourIndex)
  if (!neighbour) return false

  const slice = doc.slice(range.from, range.to)
  const tr = editor.state.tr
  tr.delete(range.from, range.to)
  const target = direction === 1 ? range.from + (neighbour.to - neighbour.from) : neighbour.from
  tr.insert(target, slice.content)
  editor.view.dispatch(tr)
  return true
}

/**
 * Opens a new section after this one, at the same level, and puts the caret in
 * its heading so it can be named straight away.
 */
export function insertSectionAfter(editor: Editor, childIndex: number, title = ''): boolean {
  const { doc } = editor.state
  const range = sectionRange(doc, childIndex)
  if (!range) return false
  const level = doc.child(childIndex).attrs.level
  editor
    .chain()
    .insertContentAt(range.to, sectionContent(typeof level === 'number' ? level : 2, title))
    .setTextSelection(range.to + 1)
    .focus()
    .run()
  return true
}
