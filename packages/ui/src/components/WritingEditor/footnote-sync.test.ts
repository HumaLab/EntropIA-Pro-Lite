import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'

/**
 * `tiptap-footnotes@4` syncs the footnote list on mount and after every edit;
 * `WritingFootnoteRules` (extensions.ts) keeps version 3's rule, under which
 * the stored manuscripts were written: only an edit that adds or removes a
 * marker touches the notes.
 */

const editors: Editor[] = []

afterEach(() => {
  editors.splice(0).forEach((instance) => instance.destroy())
})

function mount(content: JSONContent) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  const instance = new Editor({ element, extensions: createWritingExtensions(), content })
  editors.push(instance)
  return instance
}

const footnotesOf = (instance: Editor) =>
  ((instance.getJSON() as JSONContent).content ?? []).find((node) => node.type === 'footnotes')
    ?.content ?? []

const referencesOf = (instance: Editor) => {
  const found: JSONContent[] = []
  const visit = (node: JSONContent) => {
    if (node.type === 'footnoteReference') found.push(node)
    node.content?.forEach(visit)
  }
  visit(instance.getJSON() as JSONContent)
  return found
}

const ORPHAN: JSONContent = {
  type: 'doc',
  content: [
    { type: 'paragraph', content: [{ type: 'text', text: 'Cuerpo' }] },
    {
      type: 'footnotes',
      content: [
        {
          type: 'footnote',
          attrs: { id: 'fn:7', 'data-id': 'f1' },
          content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Nota suelta' }] }],
        },
      ],
    },
  ],
}

describe('footnote sync', () => {
  it('leaves a footnote no marker points at through typing in the body', () => {
    const instance = mount(ORPHAN)
    const before = footnotesOf(instance)
    expect(before.map((note) => note.attrs)).toEqual([{ id: 'fn:7', 'data-id': 'f1' }])

    instance.chain().setTextSelection(7).insertContent(' editado').run()

    expect(instance.getText()).toContain('Cuerpo editado')
    expect(footnotesOf(instance)).toEqual(before)
  })

  it('gives a new marker its footnote, numbered', () => {
    const instance = mount({
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Cuerpo' }] }],
    })
    instance.chain().setTextSelection(7).addFootnote().run()

    const [reference] = referencesOf(instance)
    expect(reference?.attrs).toMatchObject({ referenceNumber: '1', class: 'footnote-ref' })
    expect(footnotesOf(instance).map((note) => note.attrs)).toEqual([
      { id: 'fn:1', 'data-id': reference?.attrs?.['data-id'] },
    ])
  })

  it('removes the footnote of a deleted marker', () => {
    const instance = mount({
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Cuerpo' }] }],
    })
    instance.chain().setTextSelection(7).addFootnote().run()
    expect(footnotesOf(instance)).toHaveLength(1)

    instance.chain().setTextSelection({ from: 7, to: 8 }).deleteSelection().run()

    expect(referencesOf(instance)).toHaveLength(0)
    expect(footnotesOf(instance)).toHaveLength(0)
  })
})

describe('footnote markers through HTML', () => {
  it('keep the drawn number as plain text, as version 3 stored it', () => {
    const source = mount({
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Cuerpo' }] }],
    })
    source.chain().setTextSelection(7).addFootnote().run()
    const [original] = referencesOf(source)

    const copy = mount({ type: 'doc', content: [{ type: 'paragraph' }] })
    copy.commands.setContent(source.getHTML())
    const [parsed] = referencesOf(copy)

    expect(parsed?.content).toEqual([{ type: 'text', text: '1' }])
    expect(Object.keys(parsed?.attrs ?? {})).toEqual([
      'class',
      'data-id',
      'referenceNumber',
      'href',
    ])
    expect(parsed?.attrs).toEqual(original?.attrs)
  })
})
