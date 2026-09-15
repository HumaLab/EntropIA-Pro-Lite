import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'

/**
 * The toolbar's insert actions, exercised against a real editor.
 *
 * Reading an extension's type declarations proves a command exists; it does not
 * prove the schema lets it do anything. A table or a footnote that the document
 * node will not accept fails silently — the command runs, nothing changes, and
 * the button looks broken.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount() {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions(),
    content: emptyDocument().doc,
  })
  return editor
}

function typeNames(instance: Editor): string[] {
  const names: string[] = []
  instance.state.doc.forEach((node) => names.push(node.type.name))
  return names
}

describe('toolbar commands — insertions the schema has to accept', () => {
  it('inserts a table', () => {
    const instance = mount()
    instance.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()

    expect(typeNames(instance)).toContain('table')
  })

  it('inserts a footnote and its reference', () => {
    const instance = mount()
    instance.chain().focus().insertContent('Una afirmacion').run()
    instance.chain().focus().addFootnote().run()

    const json = JSON.stringify(instance.getJSON())
    expect(json, 'no footnote reference in the body').toContain('footnoteReference')
    expect(typeNames(instance), 'no footnotes container at the end').toContain('footnotes')
  })

  it('keeps the basic formatting commands working', () => {
    const instance = mount()
    instance.chain().focus().insertContent('texto').run()
    instance.chain().focus().selectAll().toggleBold().run()

    expect(instance.isActive('bold')).toBe(true)
  })

  it('toggles a heading, which the outline depends on', () => {
    const instance = mount()
    instance.chain().focus().insertContent('Capitulo').toggleHeading({ level: 2 }).run()

    expect(typeNames(instance)).toContain('heading')
  })
})
