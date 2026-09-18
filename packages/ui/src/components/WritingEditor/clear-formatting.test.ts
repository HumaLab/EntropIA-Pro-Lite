import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { CONTENT_MARKS } from './clear-formatting'

/**
 * Clear formatting removes how the text looks and nothing it says. A link is
 * content — where a citation points — so it stays; so does the block the text
 * sits in, because a heading is structure the outline is built from.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(content: JSONContent) {
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
const link = { type: 'link', attrs: { href: 'https://e.org' } }
const EVERY_FORMAT = [
  { type: 'bold' },
  { type: 'italic' },
  { type: 'underline' },
  { type: 'strike' },
  { type: 'superscript' },
  { type: 'textStyle', attrs: { fontSize: '1.5em' } },
]

function marksOf(instance: Editor, block = 0): string[][] {
  return (instance.getJSON().content?.[block]?.content ?? []).map((node) =>
    (node.marks ?? []).map((mark) => mark.type!)
  )
}

describe('clearFormatting', () => {
  it('keeps only links as content', () => {
    expect(CONTENT_MARKS).toEqual(['link'])
  })

  it('removes every format from the selection', () => {
    const instance = mount(doc(p(text('todo', EVERY_FORMAT))))
    instance.chain().selectAll().clearFormatting().run()

    expect(instance.getJSON().content?.[0]?.content).toEqual([text('todo')])
  })

  /** Nobody listed them: "every mark except links" covers them by itself. */
  it('removes a text colour and a highlight, and the link beside them stays', () => {
    const instance = mount(
      doc(
        p(
          text('rojo', [{ type: 'textStyle', attrs: { color: 'red' } }]),
          text(' y ', [{ type: 'highlight', attrs: { color: 'yellow' } }]),
          text('sitio', [
            { type: 'textStyle', attrs: { color: 'blue', fontSize: '2em' } },
            link,
            { type: 'highlight', attrs: { color: 'green' } },
          ])
        )
      )
    )
    instance.chain().selectAll().clearFormatting().run()

    expect(marksOf(instance)).toEqual([[], ['link']])
    expect(instance.getText()).toBe('rojo y sitio')
  })

  it('removes code, which excludes the others, and subscript', () => {
    const instance = mount(
      doc(p(text('x', [{ type: 'code' }]), text('2', [{ type: 'subscript' }])))
    )
    instance.chain().selectAll().clearFormatting().run()

    expect(instance.getJSON().content?.[0]?.content).toEqual([text('x2')])
  })

  it('keeps a link', () => {
    const instance = mount(doc(p(text('el sitio', [link, { type: 'bold' }]))))
    instance.chain().selectAll().clearFormatting().run()

    expect(marksOf(instance)).toEqual([['link']])
  })

  it('touches only the selection', () => {
    const instance = mount(doc(p(text('hola mundo', [{ type: 'bold' }]))))
    instance.chain().setTextSelection({ from: 6, to: 11 }).clearFormatting().run()

    expect(marksOf(instance)).toEqual([['bold'], []])
  })

  it('leaves headings and lists as they are', () => {
    const instance = mount(
      doc(
        { type: 'heading', attrs: { level: 2 }, content: [text('Título', [{ type: 'italic' }])] },
        {
          type: 'bulletList',
          content: [{ type: 'listItem', content: [p(text('punto', [{ type: 'bold' }]))] }],
        }
      )
    )
    instance.chain().selectAll().clearFormatting().run()

    const json = instance.getJSON()
    expect(json.content?.map((node) => node.type)).toEqual(['heading', 'bulletList'])
    expect(JSON.stringify(json)).not.toContain('"marks"')
  })

  it('clears the formats waiting at a caret', () => {
    const instance = mount(doc(p(text('hola'))))
    instance.chain().focus().setTextSelection(5).toggleBold().toggleItalic().run()
    instance.chain().clearFormatting().insertContent(' mundo').run()

    expect(marksOf(instance)).toEqual([[]])
    expect(instance.getText()).toBe('hola mundo')
  })

  it('is one undo step', () => {
    const instance = mount(doc(p(text('todo', EVERY_FORMAT))))
    instance.chain().selectAll().clearFormatting().run()
    instance.commands.undo()

    expect(marksOf(instance)[0]).toHaveLength(EVERY_FORMAT.length)
  })
})
