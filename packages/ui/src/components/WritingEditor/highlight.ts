import Highlight from '@tiptap/extension-highlight'
import {
  highlightColorVar,
  parseWritingColor,
  readHighlightColor,
  type WritingColor,
} from './writing-colors'

/**
 * The highlight mark, holding a palette name instead of a colour.
 *
 * The stock extension's multicolour mode stores whatever colour it is given and
 * writes it straight into the style, which is the hex-per-theme problem
 * writing-colors.ts exists to avoid. So its attribute, its parse rule and its
 * commands are replaced; the mark's name, tag and Mod-Shift-H stay the stock
 * ones. A highlight with no colour given is yellow, the one everybody expects.
 *
 * Only the background is drawn here. The ink is the text's own — or the body
 * colour, which the editor's stylesheet sets for highlighted text — so a text
 * colour on the same words still shows.
 */
export const WritingHighlight = Highlight.extend({
  addAttributes() {
    return {
      color: {
        default: 'yellow' satisfies WritingColor,
        parseHTML: (element) => readHighlightColor(element),
        renderHTML: (attributes) => {
          const name = parseWritingColor(attributes.color)
          return name
            ? { 'data-highlight': name, style: `background-color: ${highlightColorVar(name)}` }
            : {}
        },
      },
    }
  },

  /** Only this editor's own highlights: a `<mark>` from a web page is not one. */
  parseHTML() {
    return [
      {
        tag: 'mark',
        getAttrs: (element) => (readHighlightColor(element as HTMLElement) ? {} : false),
      },
    ]
  },

  addCommands() {
    return {
      setHighlight:
        (attributes) =>
        ({ commands }) =>
          parseWritingColor(attributes?.color ?? 'yellow')
            ? commands.setMark(this.name, attributes)
            : false,
      toggleHighlight:
        (attributes) =>
        ({ commands }) =>
          parseWritingColor(attributes?.color ?? 'yellow')
            ? commands.toggleMark(this.name, attributes)
            : false,
      unsetHighlight:
        () =>
        ({ commands }) =>
          commands.unsetMark(this.name),
    }
  },

  // `==x==` is notation in a manuscript (`a == b == c`), not a request for a
  // highlight, whether typed or pasted.
  addInputRules() {
    return []
  },

  addPasteRules() {
    return []
  },
})
