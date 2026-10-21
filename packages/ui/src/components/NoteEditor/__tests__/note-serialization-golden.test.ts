import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createNoteExtensions } from '../note-extensions'
import { normalizeNoteContentForEditor, normalizeNoteContentForRender } from '../note-content'

/**
 * Golden HTML of a stored note.
 *
 * A note is stored as the HTML the editor writes (normalised by note-content.ts)
 * and synced between devices. If that HTML changes when the editor library
 * does, every note opened on a new build looks edited, and a dropped link or
 * mark is lost writing. These snapshots were recorded on Tiptap 2 and must hold
 * on whatever version comes next.
 */

const editors: Editor[] = []

afterEach(() => {
  editors.splice(0).forEach((instance) => instance.destroy())
})

/** What the component does: normalise the stored note, mount it, read it back. */
function roundTrip(stored: string) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  const instance = new Editor({
    element,
    extensions: createNoteExtensions(),
    content: normalizeNoteContentForEditor(stored),
  })
  editors.push(instance)
  return { raw: instance.getHTML(), stored: normalizeNoteContentForRender(instance.getHTML()) }
}

const NOTES: Record<string, string> = {
  plainLegacyText: 'Primera línea\nsegunda línea\n\nOtro párrafo',
  marks:
    '<p><strong>negrita</strong> <em>itálica</em> <u>subrayado</u> <code>código</code> <s>tachado</s></p>',
  headings: '<h1>Uno</h1><h2>Dos</h2><h3>Tres</h3><p>cuerpo</p>',
  links:
    '<p><a href="https://example.com/a?b=1">https</a> <a href="http://example.com">http</a> ' +
    '<a href="mailto:a@b.c">mail</a> <a href="tel:+541100">tel</a> <a href="#ancla">ancla</a> ' +
    '<a href="example.com">sin esquema</a> <a href="javascript:alert(1)">script</a></p>',
  lists: '<ul><li><p>uno</p></li><li><p>dos</p></li></ul><ol><li><p>tres</p></li></ol>',
  endsInList: '<p>antes</p><ul><li><p>último</p></li></ul>',
  endsInQuote: '<p>antes</p><blockquote><p>cita final</p></blockquote>',
  endsInCode: '<p>antes</p><pre><code>const x = 1</code></pre>',
  hardBreak: '<p>línea<br>otra</p>',
  empty: '',
}

describe('stored note HTML (golden)', () => {
  for (const [name, stored] of Object.entries(NOTES)) {
    it(`stores ${name} as recorded`, () => {
      expect(roundTrip(stored).stored).toMatchSnapshot()
    })
  }

  it('reads its own stored HTML back unchanged', () => {
    for (const stored of Object.values(NOTES)) {
      const first = roundTrip(stored).stored
      expect(roundTrip(first).stored).toBe(first)
    }
  })

  /** The editor must not grow an empty paragraph after a note's last block. */
  it('adds no trailing paragraph after a list, a quote or a code block', () => {
    for (const stored of [NOTES.endsInList!, NOTES.endsInQuote!, NOTES.endsInCode!]) {
      expect(roundTrip(stored).raw).not.toMatch(/<p><\/p>$/)
    }
  })

  it('keeps the link attributes the note editor sets', () => {
    const container = document.createElement('div')
    container.innerHTML = roundTrip(NOTES.links!).raw
    const anchors = Array.from(container.querySelectorAll('a'))
    expect(anchors).toHaveLength(6)
    for (const anchor of anchors) {
      expect(anchor.getAttribute('target')).toBe('_blank')
      expect(anchor.getAttribute('rel')).toBe('noopener noreferrer nofollow')
    }
  })
})
