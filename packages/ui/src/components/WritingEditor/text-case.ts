import { Extension } from '@tiptap/core'
import type { Mark } from '@tiptap/pm/model'

/**
 * Change case, as Word's Aa menu does it: UPPER, lower, Sentence case and
 * Capitalize Each Word.
 *
 * The rules are a pure function over the text runs of one block, so they can
 * be held exactly by a test; the command only finds the runs and writes the
 * result back.
 */
export type TextCase = 'upper' | 'lower' | 'sentence' | 'words'

export interface ChangeCaseOptions {
  /** A BCP 47 tag. Turkish capitalizes i as İ; the default locale would not. */
  locale?: string
  /**
   * The selection starts inside a word. Capitalize Each Word then leaves the
   * rest of that word alone rather than raising a letter in its middle.
   */
  afterWord?: boolean
}

const LETTER = /\p{L}/u
const DIGIT = /\p{N}/u
/** What a word is made of: letters, digits, combining marks, apostrophes. */
const WORD = /[\p{L}\p{N}\p{M}'’]/u
const TERMINAL = /[.!?…]/u
const SPACE = /\s/u

/**
 * The runs of one block with their case changed.
 *
 * `null` is an atom in the line — a citation, a footnote marker — which has no
 * text of its own here: it keeps its place, and for Capitalize Each Word it
 * ends the word before it.
 *
 * A sentence starts at the start of the block, at the start of the selection,
 * and after `.`, `!`, `?` or `…` followed by a space; the capital goes on its
 * first letter, so `¿qué` becomes `¿Qué`. A point between digits (`3.5`) ends
 * nothing.
 */
export function changeCase(
  pieces: readonly (string | null)[],
  mode: TextCase,
  options: ChangeCaseOptions = {}
): (string | null)[] {
  const upper = (value: string) => value.toLocaleUpperCase(options.locale)
  const lower = (value: string) => value.toLocaleLowerCase(options.locale)

  if (mode === 'upper') return pieces.map((piece) => (piece === null ? null : upper(piece)))
  if (mode === 'lower') return pieces.map((piece) => (piece === null ? null : lower(piece)))

  let inWord = options.afterWord ?? false
  let capitalizeNext = true
  let afterTerminal = false

  return pieces.map((piece) => {
    if (piece === null) {
      inWord = false
      return null
    }
    let out = ''
    for (const char of piece) {
      if (mode === 'words') {
        out += LETTER.test(char) ? (inWord ? lower(char) : upper(char)) : char
        inWord = WORD.test(char)
        continue
      }
      if (LETTER.test(char)) {
        out += capitalizeNext ? upper(char) : lower(char)
        capitalizeNext = false
        afterTerminal = false
      } else {
        out += char
        if (DIGIT.test(char)) {
          capitalizeNext = false
          afterTerminal = false
        } else if (TERMINAL.test(char)) {
          afterTerminal = true
        } else if (SPACE.test(char)) {
          if (afterTerminal) capitalizeNext = true
          afterTerminal = false
        }
        // Anything else — quotes, ¿, ¡, brackets — leaves the state as it was.
      }
    }
    return out
  })
}

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    textCase: {
      /**
       * Changes the case of the selected text, in one step. Every run keeps its
       * marks, atoms are left alone, and the selection covers the same words.
       * Does nothing at a caret.
       */
      setTextCase: (mode: TextCase, locale?: string) => ReturnType
    }
  }
}

interface TextPiece {
  from: number
  to: number
  text: string
  marks: readonly Mark[]
}

interface Block {
  pieces: (TextPiece | null)[]
  afterWord: boolean
}

export const TextCaseCommands = Extension.create({
  name: 'textCase',

  addCommands() {
    return {
      setTextCase:
        (mode, locale) =>
        ({ tr, state, dispatch }) => {
          const { selection } = tr
          if (selection.empty) return false

          const blocks: Block[] = []
          for (const range of selection.ranges) {
            const from = range.$from.pos
            const to = range.$to.pos
            let block: Block | undefined
            tr.doc.nodesBetween(from, to, (node, pos) => {
              if (node.isTextblock) {
                // What sits just before the selection matters only when the
                // selection starts inside this block.
                const before = from > pos + 1 ? tr.doc.textBetween(from - 1, from, '', '') : ''
                block = { pieces: [], afterWord: WORD.test(before) }
                blocks.push(block)
                return true
              }
              if (!block) return true
              if (node.isText) {
                const start = Math.max(pos, from)
                const end = Math.min(pos + node.nodeSize, to)
                block.pieces.push({
                  from: start,
                  to: end,
                  text: node.text!.slice(start - pos, end - pos),
                  marks: node.marks,
                })
                return false
              }
              // An inline atom is not text: it is passed over, never entered.
              if (node.isInline) {
                block.pieces.push(null)
                return false
              }
              return true
            })
          }

          const replacements: { piece: TextPiece; text: string }[] = []
          for (const block of blocks) {
            const changed = changeCase(
              block.pieces.map((piece) => piece?.text ?? null),
              mode,
              { locale, afterWord: block.afterWord }
            )
            block.pieces.forEach((piece, index) => {
              const text = changed[index]
              if (piece && text && text !== piece.text) replacements.push({ piece, text })
            })
          }
          if (replacements.length === 0) return false

          if (dispatch) {
            // In document order, each range mapped through the replacements
            // before it: a capital can be longer than its letter (ß → SS).
            for (const { piece, text } of replacements) {
              tr.replaceWith(
                tr.mapping.map(piece.from),
                tr.mapping.map(piece.to),
                state.schema.text(text, piece.marks)
              )
            }
          }
          return true
        },
    }
  },
})
