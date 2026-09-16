import { Extension } from '@tiptap/core'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { ANCHORED_NODES, CITATION_NODE, duplicatedCitationIds } from './citations'

/**
 * Keeps every citation in the manuscript distinct (plan-editor.md §10.1, and
 * the lifecycle requirement of Unit 4).
 *
 * # What this is for
 *
 * The projection is derived from the document and unique per
 * `(document_id, citation_node_id)`, which settles copying, moving, deleting
 * and undo without any bookkeeping — except for one case the document cannot
 * settle by itself. A pasted citation arrives carrying the identity of the one
 * it was copied from, so two nodes claim one row: the second would collide with
 * the first, or silently replace it.
 *
 * # Why this also gets "move keeps its identity" right, for free
 *
 * §10.1 asks that copying create a new identity while keeping the reference to
 * the source, and that moving keep the identity. From the document's side those
 * two operations look the same — both end in a paste. What tells them apart is
 * what is left behind: after a cut there is no second claimant, so nothing is
 * duplicated and nothing is renamed. The identity survives a move precisely
 * because only an actual collision is repaired.
 *
 * Only `citationNodeId` is reissued. The asset, page, range, quoted text and
 * metadata snapshot are the reference to the source, and a copy keeps pointing
 * at the same place it was copied from.
 */

const KEY = new PluginKey('writingUniqueCitationIds')

/** A fresh citation identity. */
export function newCitationId(): string {
  const cryptoRef = globalThis.crypto
  if (cryptoRef && typeof cryptoRef.randomUUID === 'function') return cryptoRef.randomUUID()
  // Test environments and older webviews. The id only has to be unique within
  // one document, which this comfortably is.
  return `c-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}

export const UniqueCitationIds = Extension.create({
  name: 'writingUniqueCitationIds',

  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: KEY,

        appendTransaction: (transactions, _oldState, newState) => {
          if (!transactions.some((transaction) => transaction.docChanged)) return null

          const duplicated = new Set(duplicatedCitationIds(newState.doc))
          const tr = newState.tr
          let changed = false
          // The first node holding a duplicated id keeps it; every later one is
          // reissued. Walking in document order is what makes "first" mean the
          // original rather than whichever copy happens to be found first.
          const kept = new Set<string>()

          newState.doc.descendants((node, pos) => {
            const attribute = ANCHORED_NODES[node.type.name]
            if (!attribute) return true
            const id = node.attrs[attribute]

            if (typeof id !== 'string' || id.length === 0) {
              // A citation with no identity cannot be projected at all, so it
              // is given one rather than dropped from the database while it
              // still shows on the page.
              tr.setNodeAttribute(pos, attribute, newCitationId())
              changed = true
              return true
            }

            if (!duplicated.has(id)) return true
            if (!kept.has(id)) {
              kept.add(id)
              return true
            }
            tr.setNodeAttribute(pos, attribute, newCitationId())
            changed = true
            return true
          })

          return changed ? tr : null
        },
      }),
    ]
  },
})
