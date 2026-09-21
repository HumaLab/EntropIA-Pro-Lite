import { Extension } from '@tiptap/core'
import { PluginKey, Plugin } from '@tiptap/pm/state'

/**
 * Keeps an empty paragraph at the end of the manuscript.
 *
 * Without it, a table, a blockquote or a code block sitting last leaves the
 * writer trapped: there is nowhere after it to put the caret, so there is no
 * way to keep writing below it. A gap cursor does not solve this — it can only
 * place the caret in a gap that exists, and at the end of the document there is
 * none.
 *
 * The paragraph is appended by the editor, not stored as a deliberate choice of
 * the writer's. It is empty, so it costs nothing in the exported document and
 * disappears from the outline.
 */

const KEY = new PluginKey('writingTrailingParagraph')

/** Node types that trap the caret when they are last. */
const TRAPPING = new Set([
  'table',
  'blockquote',
  'codeBlock',
  'footnotes',
  'horizontalRule',
  'writingImage',
])

export const TrailingParagraph = Extension.create({
  name: 'writingTrailingParagraph',

  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: KEY,
        appendTransaction: (_transactions, _oldState, newState) => {
          const { doc, tr, schema } = newState
          const last = doc.lastChild
          if (!last) return null

          // The footnotes block belongs at the very end, so the paragraph goes
          // before it rather than after.
          const anchorIsFootnotes = last.type.name === 'footnotes'
          const candidate = anchorIsFootnotes ? doc.child(Math.max(doc.childCount - 2, 0)) : last
          if (!TRAPPING.has(candidate.type.name)) return null

          const paragraph = schema.nodes.paragraph
          if (!paragraph) return null

          const insertAt = anchorIsFootnotes ? doc.content.size - last.nodeSize : doc.content.size
          return tr.insert(insertAt, paragraph.create())
        },
      }),
    ]
  },
})
