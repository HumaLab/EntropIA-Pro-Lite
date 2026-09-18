import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'

/**
 * Subscript and superscript: real marks, one excluding the other. The footnote
 * reference is a node and only looks like a superscript, so it has to survive
 * a paste as itself rather than turning into raised text.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(content: JSONContent | string) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({ element, extensions: createWritingExtensions(), content })
  return editor
}

const doc = (...content: JSONContent[]): JSONContent => ({ type: 'doc', content })
const p = (...content: JSONContent[]): JSONContent => ({ type: 'paragraph', content })
const text = (value: string, marks?: JSONContent['marks']): JSONContent => ({
  type: 'text',
  text: value,
  ...(marks ? { marks } : {}),
})

function marksOf(instance: Editor, index = 0): string[] {
  return (instance.getJSON().content?.[0]?.content?.[index]?.marks ?? []).map((mark) => mark.type!)
}

describe('subscript and superscript', () => {
  it('toggles subscript on a selection and off again', () => {
    const instance = mount(doc(p(text('H2O'))))
    instance.chain().setTextSelection({ from: 2, to: 3 }).toggleSubscript().run()

    expect(marksOf(instance, 1)).toEqual(['subscript'])
    expect(instance.isActive('subscript')).toBe(true)

    instance.chain().setTextSelection({ from: 2, to: 3 }).toggleSubscript().run()
    expect(instance.getJSON().content?.[0]?.content).toEqual([text('H2O')])
  })

  it('replaces subscript with superscript rather than stacking them', () => {
    const instance = mount(doc(p(text('x2', [{ type: 'subscript' }]))))
    instance.chain().selectAll().toggleSuperscript().run()

    expect(marksOf(instance)).toEqual(['superscript'])
  })

  it('replaces superscript with subscript at a caret too', () => {
    const instance = mount(doc(p(text('x'))))
    instance.chain().focus().setTextSelection(2).toggleSuperscript().toggleSubscript().run()
    instance.commands.insertContent('2')

    expect(marksOf(instance, 1)).toEqual(['subscript'])
  })

  it('sits alongside the other marks', () => {
    const instance = mount(doc(p(text('m2', [{ type: 'bold' }, { type: 'italic' }]))))
    instance.chain().selectAll().toggleSuperscript().run()

    expect(marksOf(instance)).toEqual(expect.arrayContaining(['bold', 'italic', 'superscript']))
  })

  it('draws them as sub and sup', () => {
    const instance = mount(
      doc(p(text('a', [{ type: 'subscript' }]), text('b', [{ type: 'superscript' }])))
    )

    expect(instance.view.dom.querySelector('sub')).toHaveTextContent('a')
    expect(instance.view.dom.querySelector('sup')).toHaveTextContent('b')
  })

  it('is one undo step', () => {
    const instance = mount(doc(p(text('x2', [{ type: 'subscript' }]))))
    instance.chain().selectAll().toggleSuperscript().run()
    instance.commands.undo()

    expect(marksOf(instance)).toEqual(['subscript'])
  })
})

describe('pasting', () => {
  it('keeps sub and sup from pasted HTML', () => {
    const instance = mount('<p>H<sub>2</sub>O y m<sup>2</sup></p>')

    const content = instance.getJSON().content?.[0]?.content ?? []
    expect(content.find((node) => node.text === '2')?.marks).toEqual([{ type: 'subscript' }])
    expect(content.filter((node) => node.text === '2')[1]?.marks).toEqual([{ type: 'superscript' }])
  })

  it('still reads a footnote reference as a footnote, not as raised text', () => {
    const instance = mount(doc(p(text('Una afirmacion'))))
    instance.chain().focus().addFootnote().run()
    const html = instance.getHTML()
    instance.commands.setContent(html)

    expect(JSON.stringify(instance.getJSON())).toContain('footnoteReference')
    expect(JSON.stringify(instance.getJSON())).not.toContain('superscript')
  })
})
