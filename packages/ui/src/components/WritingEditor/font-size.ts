import { Extension, type CommandProps } from '@tiptap/core'
import TextStyle from '@tiptap/extension-text-style'
import type { Mark, MarkType } from '@tiptap/pm/model'

/**
 * Relative font size, on a fixed scale.
 *
 * # Why em and not points
 *
 * The typography presets set the manuscript's base size. A size stored in em is
 * a proportion of that base, so enlarged text stays enlarged by the same amount
 * whichever preset is on; a size in points would pin it and fight the preset.
 *
 * # Why a scale and not a free value
 *
 * A+ and A− are steps, as in Word. A fixed scale keeps every size in the
 * manuscript one of a few deliberate values, and gives the buttons an end to
 * stop at. 1 is the text's own size and is never stored: it is the absence of
 * the attribute, so a run stepped back to it carries no mark at all.
 */
export const FONT_SIZE_SCALE = [0.75, 0.875, 1, 1.125, 1.25, 1.5, 1.75, 2] as const

const EM = /^\s*(\d*\.?\d+)em\s*$/

/**
 * A stored or pasted size, as a step of the scale, or null for the unmarked
 * size. Only em is read: a pasted `12pt` means "the size the other program
 * happened to use", which is not something this document should keep.
 */
export function parseFontSize(value: unknown): number | null {
  if (typeof value !== 'string') return null
  const match = EM.exec(value)
  if (!match) return null
  const em = Number(match[1])
  if (!Number.isFinite(em) || em <= 0) return null
  const nearest = FONT_SIZE_SCALE.reduce((best, step) =>
    Math.abs(step - em) < Math.abs(best - em) ? step : best
  )
  return nearest === 1 ? null : nearest
}

/** The attribute value for a step; null for the unmarked size. */
export function formatFontSize(step: number): string | null {
  return step === 1 ? null : `${step}em`
}

/** One step up or down the scale, staying put at either end. */
export function stepFontSize(current: number, direction: 1 | -1): number {
  const at = FONT_SIZE_SCALE.indexOf(current as (typeof FONT_SIZE_SCALE)[number])
  const index = at === -1 ? FONT_SIZE_SCALE.indexOf(1) : at
  const next = Math.min(Math.max(index + direction, 0), FONT_SIZE_SCALE.length - 1)
  return FONT_SIZE_SCALE[next]!
}

/**
 * The `textStyle` mark, reading only the styles this editor gives meaning to.
 *
 * Stock TextStyle turns every `<span style>` into a mark, so pasting from a web
 * page or a word processor would fill the manuscript with empty marks that
 * carry nothing. A span is a text style here only when it carries a style one
 * of the attributes reads — today the size; the colour attributes join this
 * list when they arrive.
 */
export const WritingTextStyle = TextStyle.extend({
  parseHTML() {
    return [
      {
        tag: 'span',
        getAttrs: (element) =>
          parseFontSize((element as HTMLElement).style.fontSize) === null ? false : {},
      },
    ]
  },
})

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    fontSize: {
      /** Every text run in the selection one step up the scale. */
      increaseFontSize: () => ReturnType
      /** Every text run in the selection one step down the scale. */
      decreaseFontSize: () => ReturnType
    }
  }
}

function sizeOf(marks: readonly Mark[], type: MarkType): number {
  return parseFontSize(type.isInSet(marks)?.attrs.fontSize) ?? 1
}

/**
 * The mark with its size changed and every other attribute kept, or null when
 * nothing would be left on it.
 */
function resized(marks: readonly Mark[], type: MarkType, step: number): Mark | null {
  const attrs = { ...type.isInSet(marks)?.attrs, fontSize: formatFontSize(step) }
  return Object.values(attrs).some((value) => value !== null && value !== undefined)
    ? type.create(attrs)
    : null
}

function stepSelection(direction: 1 | -1) {
  return ({ tr, state, dispatch }: CommandProps): boolean => {
    const type = state.schema.marks.textStyle
    if (!type) return false
    const { selection } = tr

    // A caret: the size waits in the stored marks for the next thing typed.
    if (selection.empty) {
      const marks = tr.storedMarks ?? selection.$from.marks()
      const current = sizeOf(marks, type)
      const next = stepFontSize(current, direction)
      if (next === current) return false
      if (dispatch) {
        const mark = resized(marks, type, next)
        tr.setStoredMarks(mark ? mark.addToSet(marks) : type.removeFromSet(marks))
      }
      return true
    }

    // Collected before any change, so each run is stepped from its own size.
    const changes: { from: number; to: number; mark: Mark | null }[] = []
    for (const range of selection.ranges) {
      const from = range.$from.pos
      const to = range.$to.pos
      tr.doc.nodesBetween(from, to, (node, pos, parent) => {
        if (!node.isText || !parent?.type.allowsMarkType(type)) return
        const current = sizeOf(node.marks, type)
        const next = stepFontSize(current, direction)
        if (next === current) return
        changes.push({
          from: Math.max(pos, from),
          to: Math.min(pos + node.nodeSize, to),
          mark: resized(node.marks, type, next),
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
 * The size attribute on `textStyle`, and the commands that step it.
 *
 * The attribute renders as an inline style in em and is read back from one,
 * which is also what makes a copied passage keep its size when pasted.
 */
export const FontSize = Extension.create({
  name: 'fontSize',

  addGlobalAttributes() {
    return [
      {
        types: ['textStyle'],
        attributes: {
          fontSize: {
            default: null,
            parseHTML: (element) => formatFontSize(parseFontSize(element.style.fontSize) ?? 1),
            renderHTML: (attributes) => {
              const step = parseFontSize(attributes.fontSize)
              return step === null ? {} : { style: `font-size: ${step}em` }
            },
          },
        },
      },
    ]
  },

  addCommands() {
    return {
      increaseFontSize: () => stepSelection(1),
      decreaseFontSize: () => stepSelection(-1),
    }
  },

  /** Word's own: Ctrl+Shift+. grows, Ctrl+Shift+, shrinks. */
  addKeyboardShortcuts() {
    return {
      'Mod-Shift-.': () => this.editor.commands.increaseFontSize(),
      'Mod-Shift-,': () => this.editor.commands.decreaseFontSize(),
    }
  },
})
