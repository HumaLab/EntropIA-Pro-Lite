import { Extension } from '@tiptap/core'

/**
 * Marks that are content rather than formatting, and so survive "clear
 * formatting". A link says where a citation points; removing it would change
 * what the text says, not how it looks.
 *
 * The rule is "every mark except these" on purpose: a format added later —
 * a colour, a highlight — is cleared without anyone remembering to list it.
 */
export const CONTENT_MARKS: readonly string[] = ['link']

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    clearFormatting: {
      /**
       * Removes every character format from the selection, keeping links and
       * leaving the blocks (headings, lists) as they are. At a caret, clears
       * the formats waiting for the next thing typed.
       */
      clearFormatting: () => ReturnType
    }
  }
}

export const ClearFormatting = Extension.create({
  name: 'clearFormatting',

  addCommands() {
    return {
      clearFormatting:
        () =>
        ({ tr, state, dispatch }) => {
          const formats = Object.values(state.schema.marks).filter(
            (type) => !CONTENT_MARKS.includes(type.name)
          )
          const { selection } = tr

          if (selection.empty) {
            if (dispatch) {
              const marks = tr.storedMarks ?? selection.$from.marks()
              tr.setStoredMarks(marks.filter((mark) => !formats.includes(mark.type)))
            }
            return true
          }

          if (dispatch) {
            for (const range of selection.ranges) {
              for (const type of formats) tr.removeMark(range.$from.pos, range.$to.pos, type)
            }
          }
          return true
        },
    }
  },
})
