import Subscript from '@tiptap/extension-subscript'
import Superscript from '@tiptap/extension-superscript'

/**
 * Subscript and superscript, one excluding the other.
 *
 * Text cannot sit both above and below the line, so the schema says so
 * (`excludes`) and the commands clear the other mark first. The schema alone is
 * not enough at a caret: TipTap refuses to store a mark that a stored mark
 * excludes, so without clearing it, pressing x₂ while x² is waiting would do
 * nothing.
 *
 * Their shortcuts are the extensions' own, Mod-, and Mod-. — the font size
 * steps take the shifted pair, Mod-Shift-, and Mod-Shift-., so nothing collides.
 *
 * `<sup>` is also how a footnote reference draws itself. The reference's parse
 * rule outranks this one (priority 1000 against the default 50), so a pasted
 * footnote marker is still read as a footnote and not as raised digits.
 */
export const WritingSubscript = Subscript.extend({
  excludes: 'superscript',

  addCommands() {
    return {
      setSubscript:
        () =>
        ({ commands }) =>
          commands.unsetMark('superscript') && commands.setMark(this.name),
      toggleSubscript:
        () =>
        ({ commands }) =>
          commands.unsetMark('superscript') && commands.toggleMark(this.name),
      unsetSubscript:
        () =>
        ({ commands }) =>
          commands.unsetMark(this.name),
    }
  },
})

export const WritingSuperscript = Superscript.extend({
  excludes: 'subscript',

  addCommands() {
    return {
      setSuperscript:
        () =>
        ({ commands }) =>
          commands.unsetMark('subscript') && commands.setMark(this.name),
      toggleSuperscript:
        () =>
        ({ commands }) =>
          commands.unsetMark('subscript') && commands.toggleMark(this.name),
      unsetSuperscript:
        () =>
        ({ commands }) =>
          commands.unsetMark(this.name),
    }
  },
})
