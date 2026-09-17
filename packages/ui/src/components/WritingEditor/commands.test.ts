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

/**
 * A document with headings rendered nothing in the app while its outline was
 * fully populated — so the content reached the store and did not reach the
 * screen. This asserts the editor actually puts it in the DOM.
 */
describe('the editor renders what it is given', () => {
  const RICH = {
    type: 'doc',
    content: [
      {
        type: 'heading',
        attrs: { level: 1 },
        content: [{ type: 'text', text: 'Titulo de prueba' }],
      },
      { type: 'paragraph', content: [{ type: 'text', text: 'Un parrafo del cuerpo.' }] },
      { type: 'heading', attrs: { level: 2 }, content: [{ type: 'text', text: 'El problema' }] },
      { type: 'paragraph', content: [{ type: 'text', text: 'Otro parrafo.' }] },
    ],
  }

  it('puts headings and paragraphs in the DOM', () => {
    const element = document.createElement('div')
    document.body.appendChild(element)
    editor = new Editor({ element, extensions: createWritingExtensions(), content: RICH })

    expect(editor.state.doc.childCount).toBe(4)
    expect(editor.getText()).toContain('Titulo de prueba')
    expect(element.textContent).toContain('Titulo de prueba')
    expect(element.textContent).toContain('Un parrafo del cuerpo.')
    expect(element.querySelectorAll('h1,h2').length).toBe(2)
  })

  it('keeps a document that already ends in a footnotes block', () => {
    const element = document.createElement('div')
    document.body.appendChild(element)
    const withNotes = structuredClone(RICH)
    editor = new Editor({ element, extensions: createWritingExtensions(), content: withNotes })
    editor.chain().focus().addFootnote().run()

    const after = editor.getJSON()
    editor.destroy()

    const second = document.createElement('div')
    document.body.appendChild(second)
    editor = new Editor({ element: second, extensions: createWritingExtensions(), content: after })

    expect(second.textContent).toContain('Titulo de prueba')
  })
})

describe('table controls', () => {
  function tableEditor() {
    const element = document.createElement('div')
    document.body.appendChild(element)
    editor = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: emptyDocument().doc,
    })
    editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()
    return editor
  }

  function cellCount(instance: Editor) {
    let cells = 0
    instance.state.doc.descendants((node) => {
      if (node.type.name === 'tableCell' || node.type.name === 'tableHeader') cells += 1
    })
    return cells
  }

  it('adds and removes rows and columns', () => {
    const instance = tableEditor()
    const start = cellCount(instance)

    instance.chain().focus().addRowAfter().run()
    expect(cellCount(instance)).toBeGreaterThan(start)

    instance.chain().focus().deleteRow().run()
    expect(cellCount(instance)).toBe(start)

    instance.chain().focus().addColumnAfter().run()
    expect(cellCount(instance)).toBeGreaterThan(start)
  })

  it('deletes the whole table', () => {
    const instance = tableEditor()
    instance.chain().focus().deleteTable().run()
    expect(typeNames(instance)).not.toContain('table')
  })

  it('knows when the caret is inside a table, which is what disables nesting', () => {
    const instance = tableEditor()
    expect(instance.isActive('table')).toBe(true)

    instance.chain().focus().deleteTable().run()
    expect(instance.isActive('table')).toBe(false)
  })
})

/**
 * A table as the last node used to trap the caret: there was nowhere after it
 * to write, and a gap cursor cannot conjure a position that does not exist.
 */
describe('nothing traps the caret at the end of the document', () => {
  function mountWith(content: unknown) {
    const element = document.createElement('div')
    document.body.appendChild(element)
    editor = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: content as never,
    })
    return editor
  }

  it('keeps a paragraph after a trailing table', () => {
    const instance = mountWith(emptyDocument().doc)
    instance.chain().focus().insertTable({ rows: 2, cols: 2, withHeaderRow: true }).run()

    const names = typeNames(instance)
    expect(names.at(-1)).toBe('paragraph')
  })

  it('keeps a paragraph after a trailing blockquote', () => {
    const instance = mountWith(emptyDocument().doc)
    instance.chain().focus().insertContent('cita').toggleBlockquote().run()

    expect(typeNames(instance).at(-1)).toBe('paragraph')
  })

  it('puts it before the footnotes block, which belongs last', () => {
    const instance = mountWith(emptyDocument().doc)
    instance.chain().focus().insertContent('texto').run()
    instance.chain().focus().insertTable({ rows: 2, cols: 2, withHeaderRow: true }).run()
    instance.chain().focus().addFootnote().run()

    const names = typeNames(instance)
    expect(names.at(-1)).toBe('footnotes')
    expect(names).toContain('paragraph')
  })

  it('does not add one after an ordinary paragraph', () => {
    const instance = mountWith(emptyDocument().doc)
    instance.chain().focus().insertContent('solo texto').run()

    expect(typeNames(instance).filter((n) => n === 'paragraph')).toHaveLength(1)
  })
})
