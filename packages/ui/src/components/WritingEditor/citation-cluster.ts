import type { Editor } from '@tiptap/core'
import { newCitationId } from './unique-citation-ids'

/**
 * Citing a work into the manuscript (plan-editor.md §11.5).
 *
 * # A citation is a cluster
 *
 * `(Acha, 2015; Acha, 2008)` is **one** citation of two works, inside one pair
 * of brackets. Treating each work as its own citation is what produces
 * `(Acha, 2015)(Acha, 2008)`, which is wrong in every style — and the
 * projection said so from the start: `writing_zotero_citations` is unique on
 * `(document, cluster, position)`, a cluster of many rows.
 *
 * So citing a work beside an existing citation joins that citation. "Beside"
 * means immediately before the caret, which is exactly where the previous
 * insertion left it. Anything typed in between — even a space — means the
 * writer moved on, and a new citation is then what they meant.
 *
 * # Why this lives here rather than in the component
 *
 * The rule is the interesting part and a component is an awkward place to
 * exercise it. Keeping it in a plain function means the test drives the same
 * code the editor does, instead of a copy of it that can quietly drift.
 */

/** Adds `item` to the citation at the caret, or starts one. Returns its identity. */
export function citeWork(editor: Editor, item: Record<string, unknown>): string | null {
  const selection = editor.state.selection
  // `$from` is read rather than destructured: Svelte reserves the `$` prefix,
  // and this function is called from a component.
  const before = selection.empty ? selection.$from.nodeBefore : null

  if (before?.type.name === 'zoteroCitation') {
    const id = before.attrs.citationNodeId
    const existing = Array.isArray(before.attrs.items) ? before.attrs.items : []
    const pos = selection.$from.pos - before.nodeSize
    editor.view.dispatch(editor.state.tr.setNodeAttribute(pos, 'items', [...existing, item]))
    return typeof id === 'string' ? id : null
  }

  const citationNodeId = newCitationId()
  const inserted = editor
    .chain()
    .focus()
    .insertContent({ type: 'zoteroCitation', attrs: { citationNodeId, items: [item] } })
    .run()
  return inserted ? citationNodeId : null
}

/** The works a citation cites, in the order they read. */
export function worksOf(node: { attrs: Record<string, unknown> }): Record<string, unknown>[] {
  return Array.isArray(node.attrs.items) ? (node.attrs.items as Record<string, unknown>[]) : []
}
