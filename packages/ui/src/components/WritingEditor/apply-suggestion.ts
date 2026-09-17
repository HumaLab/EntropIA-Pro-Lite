import type { Editor } from '@tiptap/core'
import { findMatches } from './search'

/**
 * Putting an agent's proposal into the manuscript (plan-editor.md §14.2).
 *
 * # The target is the text, never a stored position
 *
 * A proposal is made about words, and between making it and accepting it the
 * writer keeps typing. A persisted `{from, to}` survives none of that — spike
 * S2 measured a stored range pointing at a different paragraph after a reload —
 * so the passage is found by looking for itself. Gone means changed, and §14.2
 * asks for a changed target to be reviewed again rather than written over.
 *
 * # Ambiguity is refused, not resolved
 *
 * A short passage can occur twice. Taking the first occurrence because it is
 * first would edit a paragraph the writer was not looking at, and they would
 * find it much later, with no way to tell what happened. Refusing costs one
 * action; guessing costs the premise that the agent touches only what it was
 * asked about.
 */

/** Where the passage currently stands, or `null` if it is gone or ambiguous. */
export function locateText(editor: Editor, passage: string): { from: number; to: number } | null {
  const needle = passage.trim()
  if (!needle) return null

  // Case-sensitive: the proposal was made about these exact words, and a match
  // that differs in case is a different passage, not the same one.
  const matches = findMatches(editor.state.doc, needle, { caseSensitive: true })
  return matches.length === 1 ? matches[0]! : null
}

/** The proposal's blocks, as the writer would have typed them. */
function blocksOf(proposal: string): string[] {
  return proposal
    .split(/\n{2,}/)
    .map((block) => block.trim())
    .filter(Boolean)
}

const paragraphs = (blocks: string[]) =>
  blocks.map((block) => ({
    type: 'paragraph' as const,
    content: [{ type: 'text' as const, text: block }],
  }))

/**
 * Whether the passage covers whole blocks, and the range of the blocks it
 * covers.
 *
 * This is the distinction the replacement turns on. A passage that *is* a
 * paragraph is replaced by paragraphs: dropping block nodes into the inline
 * range instead leaves the emptied original standing above them. A passage
 * that is part of a sentence is replaced inline, because a paragraph break
 * inside a clause is not what anyone asked for.
 */
function wholeBlocks(editor: Editor, at: { from: number; to: number }) {
  const from = editor.state.doc.resolve(at.from)
  const to = editor.state.doc.resolve(at.to)
  if (from.parentOffset !== 0 || to.parentOffset !== to.parent.content.size) return null
  return { from: from.before(from.depth), to: to.after(to.depth) }
}

/**
 * Replaces the passage with the proposal. One transaction, so one undo.
 *
 * Returns false and writes nothing when the passage cannot be located — never
 * a partial application, which would leave the manuscript in a state neither
 * the writer nor the agent chose.
 */
export function applySuggestion(editor: Editor, passage: string, proposal: string): boolean {
  const at = locateText(editor, passage)
  const blocks = blocksOf(proposal)
  if (!at || blocks.length === 0) return false

  const covered = wholeBlocks(editor, at)
  if (covered) {
    return editor.chain().focus().insertContentAt(covered, paragraphs(blocks)).run()
  }

  // Part of a sentence. The blocks are joined rather than kept apart: breaking
  // a clause in two is a worse outcome than losing a break the proposal only
  // suggested.
  return editor
    .chain()
    .focus()
    .insertContentAt({ from: at.from, to: at.to }, blocks.join(' '))
    .run()
}

/**
 * Puts the proposal after the passage, leaving the original in place.
 *
 * The difference from replacing has to be real: §14.2 offers both so that a
 * writer can keep what they wrote and compare it against what was proposed.
 */
export function insertBelow(editor: Editor, passage: string, proposal: string): boolean {
  const at = locateText(editor, passage)
  const content = paragraphs(blocksOf(proposal))
  if (!at || content.length === 0) return false

  // After the block the passage ends in, so the proposal is its own paragraph
  // rather than a continuation of the one it comments on.
  const end = editor.state.doc.resolve(at.to)
  const after = end.end(end.depth) + 1

  return editor.chain().focus().insertContentAt(after, content).run()
}
