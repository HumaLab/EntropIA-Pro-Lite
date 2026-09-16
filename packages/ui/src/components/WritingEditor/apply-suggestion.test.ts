import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { applySuggestion, insertBelow, locateText } from './apply-suggestion'

/**
 * Putting an agent's proposal into the manuscript (plan-editor.md §14.2).
 *
 * # Why the passage is located by its text and not by a position
 *
 * A proposal is made about words, and between making it and accepting it the
 * writer keeps typing. A stored `{from, to}` survives none of that: one
 * character added above it and the range points at different words, and the
 * replacement lands somewhere nobody chose. Spike S2 measured exactly this.
 *
 * So the target is found by looking for the passage itself. If it is still
 * there, that is the target; if it is not, the answer is `null` and nothing is
 * written — which is the outcome §14.2 asks for when the target has changed.
 *
 * # Why an ambiguous passage is refused
 *
 * A short passage can occur twice. Replacing the first occurrence because it is
 * first would silently edit a paragraph the writer was not looking at, and they
 * would find it much later. Refusing costs them one action; guessing costs them
 * the trust that the agent only touches what it was asked about.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

const text = (value: string) => ({ type: 'text', text: value })
const p = (body: string) => ({ type: 'paragraph', content: [text(body)] })

function mount(content: unknown[]) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions(),
    content: { type: 'doc', content } as never,
  })
  return editor
}

const bodyOf = (instance: Editor) =>
  instance.state.doc.content.content.map((node) => node.textContent)

describe('locating the passage a proposal was made about', () => {
  it('finds it where it still stands', () => {
    const instance = mount([p('primer parrafo'), p('el pasaje en cuestion')])

    const at = locateText(instance, 'el pasaje en cuestion')

    expect(at).not.toBeNull()
    expect(instance.state.doc.textBetween(at!.from, at!.to)).toBe('el pasaje en cuestion')
  })

  /** §14.2: a target that is gone is a target that changed. Nothing is written. */
  it('reports nothing when the passage is no longer there', () => {
    const instance = mount([p('el writer lo reescribio entero')])

    expect(locateText(instance, 'el pasaje en cuestion')).toBeNull()
  })

  /**
   * Guessing between two identical passages would edit one the writer was not
   * looking at, and they would find it much later.
   */
  it('refuses to choose when the passage occurs twice', () => {
    const instance = mount([p('lo mismo dos veces'), p('otra cosa'), p('lo mismo dos veces')])

    expect(locateText(instance, 'lo mismo dos veces')).toBeNull()
  })

  it('has nothing to find for an empty passage', () => {
    const instance = mount([p('algo')])

    expect(locateText(instance, '   ')).toBeNull()
  })
})

describe('replacing a passage with a proposal', () => {
  it('puts the proposed text exactly where the original was', () => {
    const instance = mount([p('primer parrafo'), p('el original'), p('ultimo parrafo')])

    const done = applySuggestion(instance, 'el original', 'el propuesto')

    expect(done).toBe(true)
    expect(bodyOf(instance)).toEqual(['primer parrafo', 'el propuesto', 'ultimo parrafo'])
  })

  /** Nothing is written when the target moved — not even partially. */
  it('changes nothing at all when the passage is gone', () => {
    const instance = mount([p('ya no dice eso')])

    const done = applySuggestion(instance, 'el original', 'el propuesto')

    expect(done).toBe(false)
    expect(bodyOf(instance)).toEqual(['ya no dice eso'])
  })

  /**
   * §14.2 offers replacing *or* inserting below, and the difference has to be
   * real: the original stays, so the writer can compare and choose afterwards.
   */
  it('inserts below without disturbing the original', () => {
    const instance = mount([p('el original'), p('lo que sigue')])

    const done = insertBelow(instance, 'el original', 'el propuesto')

    expect(done).toBe(true)
    expect(bodyOf(instance)).toEqual(['el original', 'el propuesto', 'lo que sigue'])
  })

  it('will not insert below a passage it cannot find either', () => {
    const instance = mount([p('otra cosa')])

    expect(insertBelow(instance, 'el original', 'el propuesto')).toBe(false)
    expect(bodyOf(instance)).toEqual(['otra cosa'])
  })

  /**
   * A proposal spanning several paragraphs comes back as several paragraphs,
   * not as one with line breaks inside it.
   */
  it('keeps a multi-paragraph proposal as several paragraphs', () => {
    const instance = mount([p('el original')])

    applySuggestion(instance, 'el original', 'primero\n\nsegundo')

    expect(bodyOf(instance)).toEqual(['primero', 'segundo'])
  })

  /**
   * A passage that is part of a sentence stays part of a sentence. Splitting a
   * clause in two because the model offered a break is worse than losing the
   * break it only suggested.
   */
  it('does not break a clause in two when the passage is mid-sentence', () => {
    const instance = mount([p('antes, el original, despues')])

    applySuggestion(instance, 'el original', 'primero\n\nsegundo')

    expect(bodyOf(instance)).toEqual(['antes, primero segundo, despues'])
  })

  /** And the paragraph it lives in is not left empty above the replacement. */
  it('leaves no empty paragraph where the original was', () => {
    const instance = mount([p('el original')])

    applySuggestion(instance, 'el original', 'el propuesto')

    expect(bodyOf(instance)).toEqual(['el propuesto'])
  })
})
