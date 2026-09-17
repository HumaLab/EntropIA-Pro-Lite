import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { deleteSection, insertSectionAfter, moveSection, renameSection } from './section-commands'

/**
 * Editing the manuscript through its outline.
 *
 * The thing worth asserting is not that a heading moves — it is that its body
 * moves with it. A section operation that touched only the heading would strand
 * its paragraphs under the chapter above, which reads as data loss even though
 * nothing was lost.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

const text = (value: string) => ({ type: 'text', text: value })
const h = (level: number, title: string) => ({
  type: 'heading',
  attrs: { level },
  content: [text(title)],
})
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

/** The document as a readable outline-plus-body, which is what each test is about. */
function shape(instance: Editor): string[] {
  const names: string[] = []
  instance.state.doc.forEach((node) => {
    if (node.type.name === 'heading') names.push(`h${node.attrs.level}:${node.textContent}`)
    else if (node.type.name === 'paragraph') names.push(`p:${node.textContent}`)
    else names.push(node.type.name)
  })
  return names
}

describe('renameSection', () => {
  it('changes the title and leaves the body alone', () => {
    const instance = mount([h(2, 'Viejo'), p('cuerpo'), h(2, 'Otro')])

    renameSection(instance, 0, 'Nuevo')

    expect(shape(instance)).toEqual(['h2:Nuevo', 'p:cuerpo', 'h2:Otro'])
  })

  it('accepts an emptied title without breaking the heading', () => {
    const instance = mount([h(2, 'Viejo'), p('cuerpo')])

    renameSection(instance, 0, '')

    expect(shape(instance)).toEqual(['h2:', 'p:cuerpo'])
  })
})

describe('deleteSection', () => {
  it('takes the body with the heading', () => {
    const instance = mount([h(2, 'Uno'), p('a'), p('b'), h(2, 'Dos'), p('c')])

    deleteSection(instance, 0)

    expect(shape(instance)).toEqual(['h2:Dos', 'p:c'])
  })

  it('takes the subsections too', () => {
    const instance = mount([h(2, 'Uno'), h(3, 'Uno.a'), p('a'), h(2, 'Dos')])

    deleteSection(instance, 0)

    expect(shape(instance)).toEqual(['h2:Dos'])
  })

  /** The schema wants `block+`; an emptied document is not a valid one. */
  it('leaves a paragraph behind rather than an empty document', () => {
    const instance = mount([h(2, 'Solo'), p('cuerpo')])

    deleteSection(instance, 0)

    expect(shape(instance)).toEqual(['p:'])
    expect(() => instance.state.doc.check()).not.toThrow()
  })

  /**
   * A note belongs to the sentence that cites it, so deleting that section
   * takes the note with it — the footnote extension prunes a note whose marker
   * is gone. What must survive is another section's note, and the block itself
   * must never be swallowed as though it were the last section's body.
   */
  it("keeps a surviving section's footnote, and the block with it", () => {
    const instance = mount([h(2, 'Uno'), p('a'), h(2, 'Dos'), p('b')])
    instance.chain().focus('end').addFootnote().run()
    expect(shape(instance)).toContain('footnotes')

    deleteSection(instance, 0)

    expect(shape(instance)).toContain('footnotes')
    expect(shape(instance).filter((name) => name.startsWith('h2:'))).toEqual(['h2:Dos'])
  })
})

describe('moveSection', () => {
  it('moves the whole section down, body and all', () => {
    const instance = mount([h(2, 'Uno'), p('a'), h(2, 'Dos'), p('b')])

    moveSection(instance, 0, 1)

    expect(shape(instance)).toEqual(['h2:Dos', 'p:b', 'h2:Uno', 'p:a'])
  })

  it('moves it back up again', () => {
    const instance = mount([h(2, 'Uno'), p('a'), h(2, 'Dos'), p('b')])

    moveSection(instance, 2, -1)

    expect(shape(instance)).toEqual(['h2:Dos', 'p:b', 'h2:Uno', 'p:a'])
  })

  it('carries its subsections along', () => {
    const instance = mount([h(2, 'Uno'), h(3, 'Uno.a'), p('a'), h(2, 'Dos'), p('b')])

    moveSection(instance, 0, 1)

    expect(shape(instance)).toEqual(['h2:Dos', 'p:b', 'h2:Uno', 'h3:Uno.a', 'p:a'])
  })

  /** A subsection's neighbours are the other subsections, not the chapters. */
  it('will not launch a subsection out of its chapter', () => {
    const instance = mount([h(2, 'Uno'), h(3, 'Uno.a'), p('a'), h(2, 'Dos')])

    expect(moveSection(instance, 1, 1)).toBe(false)
    expect(shape(instance)).toEqual(['h2:Uno', 'h3:Uno.a', 'p:a', 'h2:Dos'])
  })

  it('does nothing at either end', () => {
    const instance = mount([h(2, 'Uno'), p('a'), h(2, 'Dos'), p('b')])

    expect(moveSection(instance, 0, -1)).toBe(false)
    expect(moveSection(instance, 2, 1)).toBe(false)
    expect(shape(instance)).toEqual(['h2:Uno', 'p:a', 'h2:Dos', 'p:b'])
  })

  it('is a single undo', () => {
    const instance = mount([h(2, 'Uno'), p('a'), h(2, 'Dos'), p('b')])
    moveSection(instance, 0, 1)

    instance.commands.undo()

    expect(shape(instance)).toEqual(['h2:Uno', 'p:a', 'h2:Dos', 'p:b'])
  })
})

describe('insertSectionAfter', () => {
  it('opens a new section at the same level, after the whole current one', () => {
    const instance = mount([h(2, 'Uno'), p('a'), h(2, 'Dos')])

    insertSectionAfter(instance, 0, 'Nueva')

    expect(shape(instance)).toEqual(['h2:Uno', 'p:a', 'h2:Nueva', 'p:', 'h2:Dos'])
  })

  it('puts it after the subsections, not between them', () => {
    const instance = mount([h(2, 'Uno'), h(3, 'Uno.a'), p('a'), h(2, 'Dos')])

    insertSectionAfter(instance, 0, 'Nueva')

    expect(shape(instance)).toEqual(['h2:Uno', 'h3:Uno.a', 'p:a', 'h2:Nueva', 'p:', 'h2:Dos'])
  })
})
