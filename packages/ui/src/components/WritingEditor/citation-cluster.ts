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

/** Whether two entries name the same work. Identity, never a rendered string. */
function sameWork(a: Record<string, unknown>, b: Record<string, unknown>): boolean {
  const left = a.itemKey
  const right = b.itemKey
  return typeof left === 'string' && left.length > 0 && left === right
}

/**
 * Adds `item` to the citation at the caret, or starts one. Returns its identity.
 *
 * A work already in that citation is not added again. CSL has nothing to
 * distinguish a work from itself, so citing it twice renders as
 * `(Nieto, 2022; Nieto, 2022)` — a repetition with no letters to tell apart,
 * because there is only one work there. The letters people expect,
 * `2022a`/`2022b`, are for two *different* works of one author and year, and
 * those the engine handles on its own.
 */
export function citeWork(editor: Editor, item: Record<string, unknown>): string | null {
  const selection = editor.state.selection
  // `$from` is read rather than destructured: Svelte reserves the `$` prefix,
  // and this function is called from a component.
  const before = selection.empty ? selection.$from.nodeBefore : null

  if (before?.type.name === 'zoteroCitation') {
    const id = before.attrs.citationNodeId
    const existing = worksOf(before)
    const identity = typeof id === 'string' ? id : null
    // Already cited here. The citation is left exactly as it is, and its
    // identity still comes back so the caller can re-render and scroll to it.
    if (existing.some((work) => sameWork(work, item))) return identity

    const pos = selection.$from.pos - before.nodeSize
    editor.view.dispatch(editor.state.tr.setNodeAttribute(pos, 'items', [...existing, item]))
    return identity
  }

  const citationNodeId = newCitationId()
  const inserted = editor
    .chain()
    .focus()
    .insertContent({ type: 'zoteroCitation', attrs: { citationNodeId, items: [item] } })
    .run()
  return inserted ? citationNodeId : null
}

/**
 * The works a citation cites, in the order they read.
 *
 * # Why this looks for a work in two places
 *
 * A citation used to be one work, with `itemKey` and `metadataSnapshot` on the
 * node itself. It is a cluster now, and every citation written before that
 * change still has the old shape — so reading only `items` would find nothing
 * in them and the manuscripts already on disk would lose their citations.
 *
 * Nothing is rewritten here. The old shape is simply understood as what it
 * always meant: a cluster of one. The next save writes it in the new shape,
 * because the projection is derived from what this returns.
 */
export function worksOf(node: { attrs: Record<string, unknown> }): Record<string, unknown>[] {
  const items = node.attrs.items
  if (Array.isArray(items) && items.length > 0) {
    return items as Record<string, unknown>[]
  }

  const itemKey = node.attrs.itemKey
  if (typeof itemKey !== 'string' || !itemKey) return []
  return [
    {
      itemKey,
      libraryType: node.attrs.libraryType,
      libraryId: node.attrs.libraryId,
      itemVersion: node.attrs.itemVersion,
      locator: node.attrs.locator,
      locatorType: node.attrs.locatorType,
      suppressAuthor: node.attrs.suppressAuthor === true,
      metadataSnapshot: node.attrs.metadataSnapshot,
    },
  ]
}
