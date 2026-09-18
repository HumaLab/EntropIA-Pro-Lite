import { Extension, type CommandProps } from '@tiptap/core'
import type { Mark } from '@tiptap/pm/model'
import { restyled } from './font-size'
import { parseWritingColor, readTextColor, textColorVar, type WritingColor } from './writing-colors'

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    textColor: {
      /** Colours the selection, or the text typed next at a caret. */
      setTextColor: (name: WritingColor) => ReturnType
      /** Takes the colour off, keeping any size on the same mark. */
      unsetTextColor: () => ReturnType
    }
  }
}

/**
 * Applies `patch` to the `textStyle` of every text run in the selection, in
 * one transaction, so it is one undo step. At a caret it goes on the stored
 * marks instead, for the next thing typed.
 */
function restyleSelection(patch: Record<string, unknown>) {
  return ({ tr, state, dispatch }: CommandProps): boolean => {
    const type = state.schema.marks.textStyle
    if (!type) return false
    const { selection } = tr

    if (selection.empty) {
      if (dispatch) {
        const marks = tr.storedMarks ?? selection.$from.marks()
        const mark = restyled(marks, type, patch)
        tr.setStoredMarks(mark ? mark.addToSet(marks) : type.removeFromSet(marks))
      }
      return true
    }

    // Collected before any change, so each run keeps its own size.
    const changes: { from: number; to: number; mark: Mark | null }[] = []
    for (const range of selection.ranges) {
      const from = range.$from.pos
      const to = range.$to.pos
      tr.doc.nodesBetween(from, to, (node, pos, parent) => {
        if (!node.isText || !parent?.type.allowsMarkType(type)) return
        changes.push({
          from: Math.max(pos, from),
          to: Math.min(pos + node.nodeSize, to),
          mark: restyled(node.marks, type, patch),
        })
      })
    }
    if (changes.length === 0) return false
    if (dispatch) {
      for (const change of changes) {
        if (change.mark) tr.addMark(change.from, change.to, change.mark)
        else tr.removeMark(change.from, change.to, type)
      }
    }
    return true
  }
}

/**
 * The colour attribute on `textStyle`, and the commands that set it.
 *
 * It holds a palette name and draws it through the theme's token, so the same
 * manuscript reads in every theme. The data attribute is what a copy inside
 * EntropIA carries back in on paste (writing-colors.ts).
 */
export const TextColor = Extension.create({
  name: 'textColor',

  addGlobalAttributes() {
    return [
      {
        types: ['textStyle'],
        attributes: {
          color: {
            default: null,
            parseHTML: (element) => readTextColor(element),
            renderHTML: (attributes) => {
              const name = parseWritingColor(attributes.color)
              return name ? { 'data-text-color': name, style: `color: ${textColorVar(name)}` } : {}
            },
          },
        },
      },
    ]
  },

  addCommands() {
    return {
      setTextColor: (name) => (props) =>
        parseWritingColor(name) ? restyleSelection({ color: name })(props) : false,
      unsetTextColor: () => restyleSelection({ color: null }),
    }
  },
})
